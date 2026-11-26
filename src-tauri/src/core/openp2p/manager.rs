use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::Result;
use parking_lot::{Mutex, RwLock};
use tokio::sync::watch;

use crate::config::Config;
use crate::core::handle::Handle;
use crate::core::openp2p::account::{self, AccountPaths};

use crate::core::openp2p::process::{self, Reconcile};
use crate::core::openp2p::schema::{AccountSpec, OpenP2PConfig};
use crate::core::openp2p::secrets::SecretStore;
use crate::core::openp2p::state::{OpenP2PAccount, OpenP2PChecks, OpenP2PPhase, OpenP2PSource, OpenP2PView};
use crate::core::openp2p::subscriptions::{
    self, PROFILE_DESC_PREFIX, SubscriptionRuntime, SubscriptionStore, id_from_profile_uid, purge_account_dir,
};
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
    secrets: Option<SecretStore>,
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
    fn new(account: AccountSpec, secrets: Option<SecretStore>) -> Result<Self> {
        let paths = account::account_paths(&account.id)?;
        Ok(Self {
            snapshot: Mutex::new(AccountSnapshot {
                account,
                secrets,
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

    fn token_present(&self) -> bool {
        let snapshot = self.snapshot.lock();
        snapshot
            .secrets
            .as_ref()
            .is_some_and(|secrets| secrets.token_present(&snapshot.account.id))
    }

    fn secrets(&self) -> Option<SecretStore> {
        self.snapshot.lock().secrets.clone()
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

    fn to_view(&self) -> OpenP2PAccount {
        let snapshot = self.snapshot.lock();
        let account = &snapshot.account;
        let token_present = snapshot
            .secrets
            .as_ref()
            .is_some_and(|secrets| secrets.token_present(&account.id));
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

/// The active subscription the sidecar accounts were started from.
#[derive(Debug, Clone)]
pub struct OpenP2PActiveSource {
    pub profile_uid: std::string::String,
    pub view: OpenP2PSource,
    pub token_store: SecretStore,
}

struct ManagerData {
    revision: u64,
    config: Arc<OpenP2PConfig>,
    config_error: Option<std::string::String>,
    accounts: BTreeMap<std::string::String, Arc<AccountSlot>>,
    active: Option<OpenP2PActiveSource>,
}

impl Default for ManagerData {
    fn default() -> Self {
        Self {
            revision: 0,
            config: Arc::new(OpenP2PConfig::default()),
            config_error: None,
            accounts: BTreeMap::new(),
            active: None,
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
use supervisor::{force_stop_slot, supervise};

singleton!(OpenP2PManager, OPENP2P_MANAGER);

impl OpenP2PManager {
    fn new() -> Self {
        Self {
            data: RwLock::new(ManagerData::default()),
            last_event: Mutex::new(None),
            event_pending: AtomicBool::new(false),
        }
    }

    pub fn view(&self) -> OpenP2PView {
        let data = self.data.read();
        let source = data.active.as_ref().map(|active| active.view.clone());
        let accounts: Vec<OpenP2PAccount> = data
            .config
            .accounts
            .iter()
            .filter_map(|account| data.accounts.get(&account.id).map(|slot| slot.to_view()))
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
                if let Some(active) = data.active.as_ref() {
                    account.profile_uid = Some(active.profile_uid.clone());
                    account.profile_status = if account.token_present {
                        crate::core::openp2p::state::OpenP2PProfileStatus::Ready
                    } else {
                        crate::core::openp2p::state::OpenP2PProfileStatus::NeedsToken
                    };
                }
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

    pub async fn init() {
        if INITIALIZED.swap(true, Ordering::AcqRel) {
            return;
        }
        let manager = Self::global();
        if !PLATFORM_SUPPORTED {
            manager.mark_degraded("openp2p sidecar is supported on macOS only in this build".to_owned());
            return;
        }
        let _ = tokio::task::spawn_blocking(reconcile_orphans).await;
        if let Err(error) = repair_orphan_subscription_items().await {
            logging!(
                warn,
                Type::Setup,
                "failed to repair restored openp2p subscription cards: {error:#}"
            );
        }
        if let Err(error) = manager.reconcile_current().await {
            logging!(
                error,
                Type::Setup,
                "failed to reconcile openp2p with the current profile: {error:#}"
            );
            manager.deactivate().await;
            manager.set_config_error(Some(format!("{error:#}")));
        }
        let active_uid = manager
            .data
            .read()
            .active
            .as_ref()
            .map(|active| active.profile_uid.clone());
        purge_inactive_accounts(active_uid.as_deref());
    }

    /// profiles.yaml:current is the only authority: a subscription card is the
    /// sidecar source directly, and a Profile Bundle is the source of its single
    /// OpenP2P subscription member. Any other current profile stops the sidecar.
    pub async fn reconcile_current(&self) -> Result<()> {
        if !PLATFORM_SUPPORTED {
            self.deactivate().await;
            return Ok(());
        }
        let current = Config::profiles().await.latest_arc().current.clone();
        let uid = match self.sidecar_profile_uid_for_current(current.as_deref()).await {
            Ok(uid) => uid,
            Err(error) => {
                self.deactivate().await;
                return Err(error);
            }
        };
        let Some(uid) = uid else {
            self.deactivate().await;
            return Ok(());
        };
        let store = SubscriptionStore::app_default()?;
        let Some(runtime) = store.runtime_for_uid(&uid)? else {
            logging!(
                warn,
                Type::Network,
                "the current profile resolves to an OpenP2P subscription card without a subscription store"
            );
            self.deactivate().await;
            return Ok(());
        };
        if let Err(error) = ensure_current_projection(&store, &runtime).await {
            logging!(
                warn,
                Type::Setup,
                "failed to repair the generated openp2p profile: {error:#}"
            );
        }
        let source = OpenP2PActiveSource {
            profile_uid: runtime.profile_uid.clone(),
            view: runtime.source,
            token_store: store.secrets(&runtime.meta.id),
        };
        self.apply_source(runtime.config, Some(source)).await;
        Ok(())
    }

    /// Cheap convergence check for paths that change profiles.yaml:current
    /// without going through the generic activation command (for example a
    /// Profile Bundle activation).
    pub async fn ensure_current(&self) -> Result<()> {
        let current = Config::profiles().await.latest_arc().current.clone();
        let expected = match self.sidecar_profile_uid_for_current(current.as_deref()).await {
            Ok(expected) => expected,
            Err(error) => {
                self.deactivate().await;
                return Err(error);
            }
        };
        let active = self
            .data
            .read()
            .active
            .as_ref()
            .map(|active| active.profile_uid.clone());
        if expected == active {
            return Ok(());
        }
        self.reconcile_current().await
    }

    /// Resolves the profile uid that should drive the sidecar for a current
    /// profile: the subscription card itself, or a bundle's single OpenP2P
    /// subscription member. More than one member is a hard error.
    async fn sidecar_profile_uid_for_current(&self, current: Option<&str>) -> Result<Option<std::string::String>> {
        let Some(current) = current else {
            return Ok(None);
        };
        if current == crate::config::bundle::BUNDLE_UID {
            let Some(meta) = crate::config::bundle::load_meta().await? else {
                return Ok(None);
            };
            return subscriptions::single_subscription_member(&meta.members);
        }
        Ok(Self::direct_subscription_uid(current))
    }

    /// Returns the full profile uid for a direct subscription current profile.
    fn direct_subscription_uid(current: &str) -> Option<std::string::String> {
        id_from_profile_uid(current).map(|_| current.to_owned())
    }

    /// Records a sidecar failure without undoing an already valid mihomo switch.
    pub fn mark_degraded(&self, detail: std::string::String) {
        logging!(warn, Type::Network, "openp2p sidecar is degraded: {detail}");
        self.set_config_error(Some(detail));
    }

    /// Switches the sidecar to a new source without an application restart:
    /// accounts of the previous source stop and their working directories (which
    /// hold rendered credentials) are removed.
    pub async fn apply_source(&self, config: OpenP2PConfig, source: Option<OpenP2PActiveSource>) {
        let previous = self.snapshot_slots();
        for (id, slot) in &previous {
            self.stop_slot(slot, STOP_TIMEOUT).await;
            if !config.accounts.iter().any(|account| &account.id == id) {
                purge_account_dir(id);
            }
        }
        self.set_config(config, None, source).await;
    }

    pub async fn deactivate(&self) {
        let previous = self.snapshot_slots();
        for (id, slot) in &previous {
            self.stop_slot(slot, STOP_TIMEOUT).await;
            purge_account_dir(id);
        }
        self.set_config(OpenP2PConfig::default(), None, None).await;
    }

    fn snapshot_slots(&self) -> BTreeMap<std::string::String, Arc<AccountSlot>> {
        let data = self.data.read();
        data.accounts
            .iter()
            .map(|(id, slot)| (id.clone(), Arc::clone(slot)))
            .collect()
    }

    fn set_config_error(&self, error: Option<std::string::String>) {
        self.data.write().config_error = error;
        Self::notify_changed();
    }

    async fn set_config(
        &self,
        config: OpenP2PConfig,
        config_error: Option<std::string::String>,
        active: Option<OpenP2PActiveSource>,
    ) {
        let config = Arc::new(config);
        let token_store = active.as_ref().map(|active| active.token_store.clone());
        let previous: BTreeMap<std::string::String, Arc<AccountSlot>> = self.data.read().accounts.clone();
        let mut accounts = BTreeMap::new();
        for account in &config.accounts {
            let slot = previous.get(&account.id).cloned().or_else(|| {
                AccountSlot::new(account.clone(), token_store.clone())
                    .ok()
                    .map(Arc::new)
            });
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
            data.active = active;
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
            {
                let mut snapshot = slot.snapshot.lock();
                snapshot.account = account.clone();
                snapshot.secrets = token_store.clone();
            }
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
            if !slot.token_present() {
                if running {
                    self.stop_slot(slot, STOP_TIMEOUT).await;
                }
                slot.set_terminal(
                    OpenP2PPhase::TokenMissing,
                    "no staged token belongs to this account in the active subscription".into(),
                );
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
        let secrets = slot.secrets();
        let slot_clone = Arc::clone(slot);
        let join = tokio::spawn(async move {
            supervise(slot_clone, spec, secrets, stop_rx).await;
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
}

async fn ensure_current_projection(store: &SubscriptionStore, runtime: &SubscriptionRuntime) -> Result<()> {
    let profile_path = store.profile_path(&runtime.profile_uid);
    if !profile_path.is_file() {
        let rendered = store.render(&runtime.meta)?;
        account::atomic_write(&profile_path, rendered.yaml.as_bytes(), 0o600)?;
    }
    let profiles = Config::profiles().await;
    if profiles.latest_arc().get_item(&runtime.profile_uid).is_ok() {
        return Ok(());
    }
    let meta = runtime.meta.clone();
    profiles
        .with_data_modify(|mut candidate| async move {
            if subscriptions::upsert_profile_item(&mut candidate, &meta)? {
                candidate.save_file().await?;
            }
            Ok((candidate, ()))
        })
        .await?;
    Ok(())
}

/// Restored backups carry subscription cards but not the subscription store.
/// Demote those cards to plain local profiles so they stay deletable and
/// editable; the profile file itself is still in the archive.
async fn repair_orphan_subscription_items() -> Result<()> {
    let store = SubscriptionStore::app_default()?;
    let profiles = Config::profiles().await;
    let needs_repair = profiles
        .latest_arc()
        .items
        .as_ref()
        .is_some_and(|items| items.iter().any(|item| is_orphan_subscription_item(item, &store)));
    if !needs_repair {
        return Ok(());
    }
    let store = store.clone();
    profiles
        .with_data_modify(|mut candidate| async move {
            let mut changed = false;
            if let Some(items) = candidate.items.as_mut() {
                for item in items.iter_mut() {
                    if is_orphan_subscription_item(item, &store) {
                        item.desc = None;
                        changed = true;
                    }
                }
            }
            if changed {
                candidate.save_file().await?;
            }
            Ok((candidate, ()))
        })
        .await?;
    Ok(())
}

fn is_orphan_subscription_item(item: &crate::config::PrfItem, store: &SubscriptionStore) -> bool {
    item.desc
        .as_deref()
        .and_then(|desc| desc.strip_prefix(PROFILE_DESC_PREFIX))
        .is_some_and(|id| !store.dir_exists(id))
}

/// An inactive subscription keeps no account working directory: config.json holds
/// the rendered token and must not outlive the sidecar run.
fn purge_inactive_accounts(active_profile_uid: Option<&str>) {
    let Ok(store) = SubscriptionStore::app_default() else {
        return;
    };
    let current_id = active_profile_uid.and_then(id_from_profile_uid);
    let active_accounts = current_id
        .as_ref()
        .and_then(|id| store.load_config(id).ok())
        .map(|config| {
            config
                .accounts
                .into_iter()
                .map(|account| account.id)
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();
    for meta in store.list().unwrap_or_default() {
        if current_id == Some(meta.id.as_str()) {
            continue;
        }
        if let Ok(config) = store.load_config(&meta.id) {
            for account in config.accounts {
                if !active_accounts.contains(&account.id) {
                    purge_account_dir(&account.id);
                }
            }
        }
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
    fn a_direct_subscription_keeps_its_full_profile_uid() {
        assert_eq!(
            OpenP2PManager::direct_subscription_uid("op2p-sub-7bab4a2044fddf4e"),
            Some("op2p-sub-7bab4a2044fddf4e".to_owned())
        );
        assert_eq!(OpenP2PManager::direct_subscription_uid("bundle-main"), None);
        assert_eq!(OpenP2PManager::direct_subscription_uid("plain-profile"), None);
    }

    #[test]
    fn orphan_subscription_cards_are_detected_for_repair() {
        let root = std::env::temp_dir().join(format!(
            "cvr-openp2p-orphan-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let store = SubscriptionStore::at(root.clone());
        let id = "deadbeefdeadbeef";
        let item = crate::config::PrfItem {
            uid: Some(format!("op2p-sub-{id}").into()),
            desc: Some(format!("{PROFILE_DESC_PREFIX}{id}").into()),
            ..Default::default()
        };
        assert!(is_orphan_subscription_item(&item, &store));
        std::fs::create_dir_all(root.join("openp2p/subscriptions").join(id)).expect("subscription dir");
        assert!(!is_orphan_subscription_item(&item, &store));
        let plain = crate::config::PrfItem {
            desc: Some("OpenP2P".into()),
            ..Default::default()
        };
        assert!(!is_orphan_subscription_item(&plain, &store));
        let _ = std::fs::remove_dir_all(root);
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
