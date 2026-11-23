use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use parking_lot::{Mutex, RwLock};
use tokio::sync::watch;

use crate::core::handle::Handle;
use crate::core::openp2p::account::{self, AccountPaths};
use crate::core::openp2p::config as o2p_config;

use crate::core::openp2p::process::{self, Reconcile};
use crate::core::openp2p::profile_store::ProfileStore;
use crate::core::openp2p::schema::{AccountSpec, OpenP2PConfig};
use crate::core::openp2p::state::{OpenP2PAccount, OpenP2PChecks, OpenP2PPhase, OpenP2PView};
use crate::singleton;
use clash_verge_logging::{Type, logging};

const EVENT_THROTTLE: Duration = Duration::from_millis(500);
const ONLINE_RESET: Duration = Duration::from_secs(600);
const STOP_TIMEOUT: Duration = Duration::from_secs(5);
pub const STOP_ALL_TIMEOUT: Duration = Duration::from_secs(3);

/// The sidecar relies on per-socket interface binding. Darwin implements it
/// without privileges; Linux needs a privileged transport helper and Windows
/// has no equivalent. The page stays visible but read-only on other systems.
pub const PLATFORM_SUPPORTED: bool = cfg!(target_os = "macos");

static INITIALIZED: AtomicBool = AtomicBool::new(false);
static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);
static REVISION: AtomicU64 = AtomicU64::new(0);

struct Worker {
    stop: watch::Sender<bool>,
    join: tokio::task::JoinHandle<()>,
}

struct AccountSnapshot {
    account: AccountSpec,
    phase: OpenP2PPhase,
    pid: Option<u32>,
    checks: OpenP2PChecks,
    last_error: Option<std::string::String>,
    restart_count: u32,
    log_dir: std::string::String,
}

pub struct AccountSlot {
    snapshot: Mutex<AccountSnapshot>,
    worker: tokio::sync::Mutex<Option<Worker>>,
}

impl AccountSlot {
    fn new(account: AccountSpec) -> Result<Self> {
        let paths = account::account_paths(&account.id)?;
        Ok(Self {
            snapshot: Mutex::new(AccountSnapshot {
                account,
                phase: OpenP2PPhase::Stopped,
                pid: None,
                checks: OpenP2PChecks::default(),
                last_error: None,
                restart_count: 0,
                log_dir: paths.log_dir.to_string_lossy().into_owned(),
            }),
            worker: tokio::sync::Mutex::new(None),
        })
    }

    fn spec(&self) -> AccountSpec {
        self.snapshot.lock().account.clone()
    }

    fn last_error(&self) -> Option<std::string::String> {
        self.snapshot.lock().last_error.clone()
    }

    fn set_phase(&self, phase: OpenP2PPhase, error: Option<std::string::String>) {
        let mut snapshot = self.snapshot.lock();
        snapshot.phase = phase;
        if error.is_some() {
            snapshot.last_error = error;
        }
        drop(snapshot);
        OpenP2PManager::notify_changed();
    }

    fn set_running(&self, pid: u32, phase: OpenP2PPhase, checks: OpenP2PChecks, error: Option<std::string::String>) {
        let mut snapshot = self.snapshot.lock();
        snapshot.phase = phase;
        snapshot.pid = Some(pid);
        snapshot.checks = checks;
        if error.is_some() {
            snapshot.last_error = error;
        }
        drop(snapshot);
        OpenP2PManager::notify_changed();
    }

    fn set_backoff(&self, restart_count: u32, seconds: u64, error: Option<std::string::String>) {
        let mut snapshot = self.snapshot.lock();
        snapshot.phase = OpenP2PPhase::Backoff;
        snapshot.pid = None;
        snapshot.restart_count = restart_count;
        snapshot.last_error = error.or_else(|| snapshot.last_error.clone());
        drop(snapshot);
        logging!(
            warn,
            Type::Network,
            "openp2p account {} backing off for {seconds}s",
            self.snapshot.lock().account.id
        );
        OpenP2PManager::notify_changed();
    }

    fn set_stopped(&self) {
        let mut snapshot = self.snapshot.lock();
        snapshot.phase = OpenP2PPhase::Stopped;
        snapshot.pid = None;
        snapshot.checks = OpenP2PChecks::default();
        drop(snapshot);
        OpenP2PManager::notify_changed();
    }

    fn set_warning(&self, warning: std::string::String) {
        self.snapshot.lock().last_error = Some(warning);
        OpenP2PManager::notify_changed();
    }

    fn set_terminal(&self, phase: OpenP2PPhase, error: std::string::String) {
        let mut snapshot = self.snapshot.lock();
        snapshot.phase = phase;
        snapshot.pid = None;
        snapshot.last_error = Some(error);
        drop(snapshot);
        OpenP2PManager::notify_changed();
    }

    fn to_view(&self, config: &OpenP2PConfig) -> OpenP2PAccount {
        let snapshot = self.snapshot.lock();
        let account = &snapshot.account;
        let (token_present, key_present) = o2p_config::account_secrets(config, account);
        OpenP2PAccount {
            id: account.id.clone(),
            label: account.label().to_owned(),
            enabled: account.enabled,
            gateway: crate::core::openp2p::state::OpenP2PGateway {
                host: account
                    .gateway
                    .as_ref()
                    .map(|gateway| gateway.host.clone())
                    .unwrap_or_default(),
                port: account.gateway.as_ref().map(|gateway| gateway.port).unwrap_or(0),
            },
            node: account.node.clone(),
            public_ip_port: account.public_ip_port,
            interface: account.interface().to_owned(),
            phase: snapshot.phase,
            token_present,
            key_present,
            key_refs: config
                .proxies_for(&account.id)
                .map(|proxy| proxy.private_key_ref.clone())
                .collect(),
            pid: snapshot.pid,
            checks: snapshot.checks.clone(),
            last_error: snapshot.last_error.clone(),
            log_dir: snapshot.log_dir.clone(),
            restart_count: snapshot.restart_count,
            route_count: 0,
            profile_uid: None,
            profile_status: crate::core::openp2p::state::OpenP2PProfileStatus::Unknown,
        }
    }
}

struct ManagerData {
    revision: u64,
    config: Arc<OpenP2PConfig>,
    config_error: Option<std::string::String>,
    accounts: BTreeMap<std::string::String, Arc<AccountSlot>>,
}

impl Default for ManagerData {
    fn default() -> Self {
        Self {
            revision: 0,
            config: Arc::new(OpenP2PConfig::default()),
            config_error: None,
            accounts: BTreeMap::new(),
        }
    }
}

pub struct OpenP2PManager {
    data: RwLock<ManagerData>,
    last_event: Mutex<Option<Instant>>,
    event_pending: AtomicBool,
}

#[path = "manager_supervisor.rs"]
mod supervisor;
use supervisor::{force_stop_slot, reconcile_account, supervise};

singleton!(OpenP2PManager, OPENP2P_MANAGER);

impl OpenP2PManager {
    fn new() -> Self {
        Self {
            data: RwLock::new(ManagerData::default()),
            last_event: Mutex::new(None),
            event_pending: AtomicBool::new(false),
        }
    }

    pub fn config(&self) -> Arc<OpenP2PConfig> {
        std::sync::Arc::clone(&self.data.read().config)
    }

    pub fn account_spec(&self, account_id: &str) -> Option<AccountSpec> {
        self.data.read().accounts.get(account_id).map(|slot| slot.spec())
    }

    pub fn view(&self) -> OpenP2PView {
        let data = self.data.read();
        let source = ProfileStore::app_default().ok().and_then(|store| {
            store
                .load_import_metadata()
                .ok()
                .flatten()
                .map(|metadata| crate::core::openp2p::state::OpenP2PSource {
                    kind: metadata.kind,
                    name: metadata.name,
                    version: metadata.version,
                    applied_at: Some(metadata.applied_at),
                    status: Some("applied".to_owned()),
                    contains_mihomo: metadata.contains_mihomo,
                    self_contained: metadata.self_contained,
                })
        });
        let accounts: Vec<OpenP2PAccount> = data
            .config
            .accounts
            .iter()
            .filter_map(|account| data.accounts.get(&account.id).map(|slot| slot.to_view(&data.config)))
            .map(|mut account| {
                let id = account.id.clone();
                account.route_count = data
                    .config
                    .networks
                    .iter()
                    .filter(|network| {
                        network.account.as_deref() == Some(id.as_str())
                            || (network.account.is_none() && data.config.accounts.len() == 1)
                    })
                    .map(|network| (network.domains.len() + network.cidrs.len()) as u32)
                    .sum();
                account.profile_uid = Some(format!("op2p-{id}"));
                account.profile_status = if !account.token_present {
                    crate::core::openp2p::state::OpenP2PProfileStatus::NeedsToken
                } else if !account.key_present {
                    crate::core::openp2p::state::OpenP2PProfileStatus::NeedsKey
                } else {
                    crate::core::openp2p::state::OpenP2PProfileStatus::Ready
                };
                account
            })
            .collect();
        OpenP2PView {
            config_revision: data.revision,
            source,
            accounts,
            config_error: data.config_error.clone(),
        }
    }

    async fn ensure_profile_store() {
        let Ok(store) = ProfileStore::app_default() else {
            return;
        };
        match store.load_config() {
            Ok(Some(config)) => {
                if let Err(error) = store.project_live(&config).await {
                    logging!(
                        warn,
                        Type::Setup,
                        "failed to refresh generated openp2p profiles: {error:#}"
                    );
                }
            }
            Ok(None) => {}
            Err(error) => logging!(warn, Type::Setup, "failed to read openp2p profile store: {error:#}"),
        }
    }

    pub async fn init() {
        if INITIALIZED.swap(true, Ordering::AcqRel) {
            return;
        }
        let manager = Self::global();
        if !PLATFORM_SUPPORTED {
            manager.set_config_error(Some(
                "openp2p sidecar is supported on macOS only in this build".to_owned(),
            ));
            return;
        }
        let _ = tokio::task::spawn_blocking(reconcile_orphans).await;
        Self::ensure_profile_store().await;
        if let Err(error) = manager.reload_from_current().await {
            logging!(error, Type::Setup, "failed to initialize openp2p: {error:#}");
            manager.set_config_error(Some(format!("{error:#}")));
        }
    }

    pub async fn reload_from_current(&self) -> Result<()> {
        if crate::core::handle::Handle::global().is_exiting() {
            return Ok(());
        }
        let config = ProfileStore::app_default()
            .and_then(|store| store.load_config())
            .map(|config| config.unwrap_or_default())?;
        self.set_config(config, None).await;
        Ok(())
    }

    fn set_config_error(&self, error: Option<std::string::String>) {
        self.data.write().config_error = error;
        Self::notify_changed();
    }

    pub async fn set_config(&self, config: OpenP2PConfig, config_error: Option<std::string::String>) {
        let config = Arc::new(config);
        let previous: BTreeMap<std::string::String, Arc<AccountSlot>> = self.data.read().accounts.clone();
        let mut accounts = BTreeMap::new();
        for account in &config.accounts {
            let slot = previous
                .get(&account.id)
                .cloned()
                .or_else(|| AccountSlot::new(account.clone()).ok().map(Arc::new));
            if let Some(slot) = slot {
                accounts.insert(account.id.clone(), slot);
            }
        }
        REVISION.fetch_add(1, Ordering::AcqRel);
        {
            let mut data = self.data.write();
            data.revision = REVISION.load(Ordering::Acquire);
            data.config = std::sync::Arc::clone(&config);
            data.config_error = config_error;
            data.accounts = accounts.clone();
        }
        Self::notify_changed();

        for (id, slot) in &previous {
            if !accounts.contains_key(id) {
                self.stop_slot(slot, STOP_TIMEOUT).await;
            }
        }
        for account in &config.accounts {
            let Some(slot) = accounts.get(&account.id) else {
                continue;
            };
            let changed = slot.spec() != *account;
            slot.snapshot.lock().account = account.clone();
            let running = slot
                .worker
                .lock()
                .await
                .as_ref()
                .is_some_and(|worker| !worker.join.is_finished());
            if !account.enabled {
                if running {
                    self.stop_slot(slot, STOP_TIMEOUT).await;
                }
                if !running {
                    slot.set_phase(OpenP2PPhase::Stopped, None);
                }
                continue;
            }
            if !account::token_present(&account.token_ref) {
                if running {
                    self.stop_slot(slot, STOP_TIMEOUT).await;
                }
                slot.set_terminal(OpenP2PPhase::TokenMissing, "token is not saved for this account".into());
                continue;
            }
            if running && !changed {
                continue;
            }
            if running {
                self.stop_slot(slot, STOP_TIMEOUT).await;
            }
            self.start_slot(slot).await;
        }
    }

    pub async fn start_account(&self, account_id: &str) -> Result<OpenP2PAccount> {
        let (config, slot) = {
            let data = self.data.read();
            let slot = data
                .accounts
                .get(account_id)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("openp2p account not found"))?;
            (std::sync::Arc::clone(&data.config), slot)
        };
        let account = slot.spec();
        if !account::token_present(&account.token_ref) {
            slot.set_terminal(OpenP2PPhase::TokenMissing, "token is not saved for this account".into());
            return Ok(slot.to_view(&config));
        }
        let paths = account::account_paths(account_id)?;
        match reconcile_account(paths.clone()).await {
            Reconcile::Indeterminate => {
                let message =
                    "an unverified previous child record exists; inspect the account child.json before starting"
                        .to_owned();
                slot.set_terminal(OpenP2PPhase::Failed, message);
                return Ok(slot.to_view(&config));
            }
            Reconcile::Killed | Reconcile::Stale => {}
            Reconcile::Missing => {}
        }
        let running = slot
            .worker
            .lock()
            .await
            .as_ref()
            .is_some_and(|worker| !worker.join.is_finished());
        if !running {
            self.start_slot(&slot).await;
        }
        Ok(slot.to_view(&config))
    }

    pub async fn stop_account(&self, account_id: &str) -> Result<OpenP2PAccount> {
        let (config, slot) = {
            let data = self.data.read();
            let slot = data
                .accounts
                .get(account_id)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("openp2p account not found"))?;
            (std::sync::Arc::clone(&data.config), slot)
        };
        self.stop_slot(&slot, STOP_TIMEOUT).await;
        Ok(slot.to_view(&config))
    }

    pub async fn restart_account(&self, account_id: &str) -> Result<OpenP2PAccount> {
        self.stop_account(account_id).await?;
        self.start_account(account_id).await
    }

    pub async fn refresh_account_secrets(&self, account_id: &str) -> Result<OpenP2PAccount> {
        let config = self.config();
        let slot = self
            .data
            .read()
            .accounts
            .get(account_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("openp2p account not found"))?;
        let account = slot.spec();
        let running = slot
            .worker
            .lock()
            .await
            .as_ref()
            .is_some_and(|worker| !worker.join.is_finished());
        if !account::token_present(&account.token_ref) {
            if running {
                self.stop_slot(&slot, STOP_TIMEOUT).await;
            }
            slot.set_terminal(OpenP2PPhase::TokenMissing, "token is not saved for this account".into());
            return Ok(slot.to_view(&config));
        }
        if !account.enabled {
            return Ok(slot.to_view(&config));
        }
        if running {
            self.stop_slot(&slot, STOP_TIMEOUT).await;
        }
        self.start_slot(&slot).await;
        Ok(slot.to_view(&config))
    }

    pub async fn stop_all(&self, limit: Duration) {
        SHUTTING_DOWN.store(true, Ordering::Release);
        let deadline = Instant::now() + limit;
        let slots: Vec<Arc<AccountSlot>> = self.data.read().accounts.values().cloned().collect();
        for slot in slots {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                force_stop_slot(&slot);
                continue;
            }
            self.stop_slot(&slot, remaining.min(STOP_TIMEOUT)).await;
        }
    }

    async fn start_slot(&self, slot: &Arc<AccountSlot>) {
        let mut worker_guard = slot.worker.lock().await;
        if worker_guard.as_ref().is_some_and(|worker| !worker.join.is_finished()) {
            return;
        }
        *worker_guard = None;
        let (stop_tx, stop_rx) = watch::channel(false);
        let spec = slot.spec();
        let slot_clone = Arc::clone(slot);
        let join = tokio::spawn(async move {
            supervise(slot_clone, spec, stop_rx).await;
        });
        *worker_guard = Some(Worker { stop: stop_tx, join });
    }

    async fn stop_slot(&self, slot: &Arc<AccountSlot>, limit: Duration) {
        let worker = {
            let mut guard = slot.worker.lock().await;
            guard.take()
        };
        let Some(worker) = worker else {
            return;
        };
        let _ = worker.stop.send(true);
        let mut join = worker.join;
        if tokio::time::timeout(limit, &mut join).await.is_err() {
            logging!(
                warn,
                Type::Network,
                "openp2p account {} did not stop in time; killing its tracked process group",
                slot.spec().id
            );
            force_stop_slot(slot);
            join.abort();
        }
    }

    fn send_event(&self) {
        if crate::APP_HANDLE.get().is_none() || Handle::global().is_exiting() {
            return;
        }
        let view = self.view();
        Handle::notify_openp2p_state(&view);
    }

    pub fn notify_changed() {
        let manager = Self::global();
        if crate::APP_HANDLE.get().is_none() {
            return;
        }
        let now = Instant::now();
        let send_now = {
            let mut last = manager.last_event.lock();
            if last.is_none_or(|last| now.duration_since(last) >= EVENT_THROTTLE) {
                *last = Some(now);
                true
            } else {
                false
            }
        };
        if send_now {
            manager.send_event();
            return;
        }
        if manager.event_pending.swap(true, Ordering::AcqRel) {
            return;
        }
        tokio::spawn(async move {
            tokio::time::sleep(EVENT_THROTTLE).await;
            let manager = Self::global();
            manager.event_pending.store(false, Ordering::Release);
            *manager.last_event.lock() = Some(Instant::now());
            manager.send_event();
        });
    }

    pub async fn delete_account(&self, account_id: &str) -> Result<OpenP2PView> {
        let Some(account) = self.account_spec(account_id) else {
            bail!("openp2p account not found");
        };
        let before = self.config();
        self.stop_account(account_id).await?;
        let paths = account::account_paths(account_id)?;
        if reconcile_account(paths.clone()).await == Reconcile::Indeterminate {
            bail!("cannot delete the account: an unverified child record remains");
        }
        crate::cmd::openp2p::detach_profile_from_bundle(&format!("op2p-{account_id}")).await?;
        let store = ProfileStore::app_default()?;
        let mut config = store
            .load_config()?
            .ok_or_else(|| anyhow::anyhow!("openp2p config store is missing"))?;
        config.accounts.retain(|entry| entry.id != account_id);
        config.proxies.retain(|proxy| proxy.account != account_id);
        if config.accounts.is_empty() {
            config.networks.clear();
        } else {
            config
                .networks
                .retain(|network| network.account.as_deref() != Some(account_id));
        }
        store.save_config(&config)?;
        store.project_current().await?;
        self.set_config(config, None).await;

        // Credentials and the rendered config.json must not outlive the account.
        if let Ok(token_path) = account::token_path(&account.token_ref)
            && let Err(error) = std::fs::remove_file(&token_path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            logging!(warn, Type::Network, "failed to remove an openp2p token file: {error}");
        }
        let after = self.config();
        for proxy in before.proxies_for(account_id) {
            if !after
                .proxies
                .iter()
                .any(|remaining| remaining.private_key_ref == proxy.private_key_ref)
                && let Ok(key_path) = account::key_path(&proxy.private_key_ref)
                && let Err(error) = std::fs::remove_file(&key_path)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                logging!(warn, Type::Network, "failed to remove an openp2p key file: {error}");
            }
        }
        if let Err(error) = std::fs::remove_dir_all(&paths.workdir)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            logging!(
                warn,
                Type::Network,
                "failed to remove the openp2p account directory: {error}"
            );
        }
        Ok(self.view())
    }
}

pub fn reconcile_orphans() {
    let Ok(root) = account::accounts_root() else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        let paths = AccountPaths {
            workdir: entry.path(),
            config_path: entry.path().join("config.json"),
            child_path: entry.path().join("child.json"),
            log_dir: entry.path().join("log"),
            log_path: entry.path().join("log/openp2p.log"),
        };
        match process::reconcile(&paths) {
            Reconcile::Killed => logging!(
                warn,
                Type::Setup,
                "killed an orphaned openp2p process for account directory {}",
                entry.file_name().to_string_lossy()
            ),
            Reconcile::Indeterminate => logging!(
                error,
                Type::Setup,
                "openp2p account {} has an unverified child.json; it will not auto-start",
                entry.file_name().to_string_lossy()
            ),
            Reconcile::Missing | Reconcile::Stale => {}
        }
    }
}

/// Synchronous last resort used by RunEvent::Exit after the async shutdown paths.
pub fn stop_all_now_blocking() {
    let killed = process::kill_tracked_children();
    if killed > 0 {
        logging!(
            warn,
            Type::System,
            "killed {killed} openp2p process group(s) during application exit"
        );
    }
}

pub async fn stop_all_bounded() -> bool {
    OpenP2PManager::global().stop_all(Duration::from_secs(3)).await;
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::openp2p::health;

    #[test]
    fn backoff_follows_the_documented_ladder() {
        let seconds: Vec<u64> = (0..6).map(|attempt| health::backoff(attempt).as_secs()).collect();
        assert_eq!(seconds, vec![2, 5, 15, 60, 300, 300]);
    }

    #[test]
    fn event_revision_is_monotonic() {
        let first = REVISION.fetch_add(1, Ordering::AcqRel);
        let second = REVISION.fetch_add(1, Ordering::AcqRel);
        assert!(second > first);
    }

    #[tokio::test]
    async fn a_manager_without_config_has_an_empty_view() {
        let manager = OpenP2PManager::new();
        let view = manager.view();
        assert!(view.accounts.is_empty());
    }

    #[test]
    fn child_state_round_trip_is_serializable() {
        use crate::core::openp2p::process::ChildState;
        let record = process::ChildRecord {
            account_id: "home".into(),
            state: ChildState::Running,
            pid: 42,
            pgid: 42,
            start_time: 7,
            uid: 501,
            spawned_at: 0,
        };
        let encoded = serde_json::to_string(&record).expect("serialize");
        let decoded: process::ChildRecord = serde_json::from_str(&encoded).expect("deserialize");
        assert_eq!(decoded.pid, 42);
    }
}
