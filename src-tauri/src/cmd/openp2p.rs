use serde::{Deserialize, Serialize};

use super::{CmdResult, CommandFailure, coded_error};
use crate::config::Config;
use crate::core::notify::{Refresh, announce};
use crate::core::openp2p::account;
use crate::core::openp2p::manager::OpenP2PManager;
use crate::core::openp2p::portable::{self, MAX_IMPORT_BYTES, OpenP2PImportSourceKind, SecretVault};
use crate::core::openp2p::state::OpenP2PView;
use crate::core::openp2p::subscriptions::{
    SubscriptionImport, SubscriptionMeta, SubscriptionRuntime, SubscriptionStore,
};
use crate::core::validate::ValidationOutcome;
use crate::utils::network::{NetworkManager, ProxyType};
use clash_verge_logging::{Type, logging};

const FETCH_TIMEOUT_SECS: u64 = 20;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct OpenP2PSubscription {
    pub id: std::string::String,
    pub profile_uid: std::string::String,
    pub name: std::string::String,
    pub source: OpenP2PImportSourceKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_url: Option<std::string::String>,
    pub imported_at: u64,
    pub account_count: usize,
    pub active: bool,
}

#[derive(Debug)]
pub struct ImportReadError {
    pub kind: OpenP2PImportSourceKind,
    pub code: &'static str,
    pub message: std::string::String,
}

impl ImportReadError {
    fn new(kind: OpenP2PImportSourceKind, code: &'static str, message: impl Into<std::string::String>) -> Self {
        Self {
            kind,
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ImportReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ImportReadError {}

pub struct ResolvedImport {
    pub kind: OpenP2PImportSourceKind,
    pub text: std::string::String,
    /// Canonical HTTPS URL of a remote source; never logged.
    pub url: Option<std::string::String>,
}

impl std::fmt::Debug for ResolvedImport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedImport")
            .field("kind", &self.kind)
            .field("bytes", &self.text.len())
            .field("url", &self.url.as_ref().map(|_| "<url>"))
            .finish()
    }
}

/// Reads exactly one source. A URL must be HTTPS; the body is capped at 1 MiB.
pub async fn read_import(
    yaml: Option<std::string::String>,
    url: Option<std::string::String>,
) -> std::result::Result<ResolvedImport, ImportReadError> {
    let yaml = yaml.filter(|text| !text.trim().is_empty());
    let url = url.filter(|text| !text.trim().is_empty());
    match (yaml, url) {
        (Some(_), Some(_)) => Err(ImportReadError::new(
            OpenP2PImportSourceKind::Url,
            "source_ambiguous",
            "supply either inline YAML or a URL, not both",
        )),
        (None, None) => Err(ImportReadError::new(
            OpenP2PImportSourceKind::Local,
            "source_missing",
            "no YAML or URL was supplied",
        )),
        (Some(text), None) => {
            if text.len() > MAX_IMPORT_BYTES {
                return Err(ImportReadError::new(
                    OpenP2PImportSourceKind::Local,
                    "source_too_large",
                    "the configuration exceeds 1 MiB",
                ));
            }
            Ok(ResolvedImport {
                kind: OpenP2PImportSourceKind::Local,
                text,
                url: None,
            })
        }
        (None, Some(raw_url)) => {
            let url = parse_https_url(&raw_url)?;
            let fetched = NetworkManager::new()
                .get_bytes(
                    url.as_str(),
                    ProxyType::None,
                    Some(FETCH_TIMEOUT_SECS),
                    MAX_IMPORT_BYTES,
                )
                .await
                .map_err(|error| {
                    let detail = crate::utils::help::mask_err(&format!("{error:#}"));
                    ImportReadError::new(
                        OpenP2PImportSourceKind::Url,
                        "fetch_failed",
                        format!("failed to fetch the import URL: {detail}"),
                    )
                })?;
            let text = std::string::String::from_utf8(fetched).map_err(|_| {
                ImportReadError::new(
                    OpenP2PImportSourceKind::Url,
                    "source_not_utf8",
                    "the downloaded document is not valid UTF-8",
                )
            })?;
            if text.len() > MAX_IMPORT_BYTES {
                return Err(ImportReadError::new(
                    OpenP2PImportSourceKind::Url,
                    "source_too_large",
                    "the configuration exceeds 1 MiB",
                ));
            }
            Ok(ResolvedImport {
                kind: OpenP2PImportSourceKind::Url,
                text,
                url: Some(url.to_string()),
            })
        }
    }
}

fn parse_https_url(raw: &str) -> std::result::Result<reqwest::Url, ImportReadError> {
    let url = reqwest::Url::parse(raw.trim()).map_err(|error| {
        ImportReadError::new(
            OpenP2PImportSourceKind::Url,
            "url_invalid",
            format!("invalid import URL: {error}"),
        )
    })?;
    if url.scheme() != "https" {
        return Err(ImportReadError::new(
            OpenP2PImportSourceKind::Url,
            "url_insecure",
            "the import URL must use HTTPS",
        ));
    }
    if url.host_str().is_none() {
        return Err(ImportReadError::new(
            OpenP2PImportSourceKind::Url,
            "url_invalid",
            "the import URL has no host",
        ));
    }
    Ok(url)
}

fn require_platform() -> CmdResult<()> {
    if crate::core::openp2p::manager::PLATFORM_SUPPORTED {
        Ok(())
    } else {
        Err(coded_error(
            "OPENP2P_PLATFORM_UNSUPPORTED",
            "openp2p sidecar is supported on macOS only in this build",
        ))
    }
}

fn subscription_failure(error: anyhow::Error) -> CommandFailure {
    coded_error("OPENP2P_SUBSCRIPTION_FAILED", format!("{error:#}"))
}

fn source_failure(error: ImportReadError) -> CommandFailure {
    logging!(
        warn,
        Type::Cmd,
        "openp2p import source rejected ({:?}): {}",
        error.kind,
        error.message
    );
    let code = match error.code {
        "url_insecure" | "url_invalid" => "OPENP2P_IMPORT_URL_INVALID",
        "fetch_failed" | "source_not_utf8" => "OPENP2P_IMPORT_FETCH_FAILED",
        "source_too_large" => "OPENP2P_IMPORT_TOO_LARGE",
        _ => "OPENP2P_IMPORT_SOURCE_INVALID",
    };
    coded_error(code, error.message)
}

fn subscription_store() -> CmdResult<SubscriptionStore> {
    SubscriptionStore::app_default().map_err(subscription_failure)
}

fn secret_vault() -> CmdResult<SecretVault> {
    SecretVault::app_default().map_err(|error| coded_error("OPENP2P_IMPORT_FAILED", format!("{error:#}")))
}

fn find_subscription(store: &SubscriptionStore, profile_uid: &str) -> CmdResult<SubscriptionMeta> {
    store
        .find_by_profile_uid(profile_uid)
        .map_err(subscription_failure)?
        .ok_or_else(|| {
            coded_error(
                "OPENP2P_SUBSCRIPTION_NOT_FOUND",
                "no openp2p subscription belongs to this profile",
            )
        })
}

async fn subscription_view(meta: &SubscriptionMeta) -> CmdResult<OpenP2PSubscription> {
    let profile_uid = meta.profile_uid();
    let current = Config::profiles().await.latest_arc().current.clone();
    Ok(OpenP2PSubscription {
        id: meta.id.clone(),
        profile_uid: profile_uid.clone(),
        name: meta.name.clone(),
        source: meta.source,
        source_url: meta.source_url.clone(),
        imported_at: meta.imported_at,
        account_count: meta.account_count,
        active: current.as_deref() == Some(profile_uid.as_str()),
    })
}

enum SubscriptionWrite<'a> {
    Import {
        kind: OpenP2PImportSourceKind,
        text: &'a str,
        url: Option<&'a str>,
    },
    Update {
        text: &'a str,
        id: &'a str,
    },
}

/// Writes one subscription and its generated profile, then persists the profile
/// item list. On a failed profiles.yaml write the subscription files,
/// credentials and generated profile are rolled back.
async fn run_subscription_write(
    store: &SubscriptionStore,
    vault: &SecretVault,
    write: SubscriptionWrite<'_>,
) -> CmdResult<SubscriptionImport> {
    let _profile_write_guard = crate::config::profiles::PROFILE_WRITE_LOCK.lock().await;
    let profiles = Config::profiles().await;
    profiles
        .with_data_modify(|mut candidate| async move {
            let prepared = match write {
                SubscriptionWrite::Import { kind, text, url } => {
                    store.prepare_import(vault, kind, text, url, &mut candidate)?
                }
                SubscriptionWrite::Update { text, id } => store.prepare_update(vault, text, id, &mut candidate)?,
            };
            if let Err(error) = candidate.save_file().await {
                if let Err(rollback) = prepared.rollback() {
                    logging!(
                        warn,
                        Type::Config,
                        "openp2p subscription write rollback failed: {rollback:#}"
                    );
                }
                return Err(error);
            }
            let import = prepared.commit();
            Ok((candidate, import))
        })
        .await
        .map_err(|error| coded_error("OPENP2P_IMPORT_FAILED", format!("{error:#}")))
}

fn log_import_warnings(import: &SubscriptionImport) {
    logging!(
        info,
        Type::Network,
        "openp2p subscription {} {} (profile changed: {}, cards changed: {})",
        import.meta.id,
        if import.created { "imported" } else { "updated" },
        import.profile_changed,
        import.items_changed
    );
    for warning in &import.warnings {
        logging!(
            warn,
            Type::Network,
            "openp2p subscription {}: {} ({})",
            import.meta.id,
            warning.message,
            warning.path
        );
    }
}

async fn refresh_runtime() -> CmdResult<()> {
    match crate::core::CoreManager::global().update_config_with_force(true).await {
        Ok(outcome) if outcome.is_valid() => {
            announce(Refresh::Clash);
            Ok(())
        }
        Ok(outcome) => Err(coded_error(
            "OPENP2P_PROFILE_SYNC_FAILED",
            format!("the updated profile did not pass validation: {outcome}"),
        )),
        Err(error) => Err(coded_error("OPENP2P_PROFILE_SYNC_FAILED", format!("{error:#}"))),
    }
}

fn ensure_subscription_profile(store: &SubscriptionStore, runtime: &SubscriptionRuntime) -> CmdResult<()> {
    let path = store.profile_path(&runtime.profile_uid);
    if path.is_file() {
        return Ok(());
    }
    let rendered = store.render(&runtime.meta).map_err(subscription_failure)?;
    account::atomic_write(&path, rendered.yaml.as_bytes(), 0o600).map_err(subscription_failure)
}

#[tauri::command]
#[specta::specta]
pub async fn get_openp2p_state() -> CmdResult<OpenP2PView> {
    let manager = OpenP2PManager::global();
    if let Err(error) = manager.ensure_current().await {
        manager.mark_degraded(format!("{error:#}"));
    }
    Ok(manager.view())
}

#[tauri::command]
#[specta::specta]
pub async fn import_openp2p_subscription(
    yaml: Option<std::string::String>,
    url: Option<std::string::String>,
) -> CmdResult<OpenP2PSubscription> {
    let resolved = read_import(yaml, url).await.map_err(source_failure)?;
    let store = subscription_store()?;
    let vault = secret_vault()?;
    let import = run_subscription_write(
        &store,
        &vault,
        SubscriptionWrite::Import {
            kind: resolved.kind,
            text: &resolved.text,
            url: resolved.url.as_deref(),
        },
    )
    .await?;
    log_import_warnings(&import);
    subscription_view(&import.meta).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_openp2p_subscription_content(profile_uid: std::string::String) -> CmdResult<std::string::String> {
    let store = subscription_store()?;
    let meta = find_subscription(&store, &profile_uid)?;
    store.read_source(&meta.id).map_err(subscription_failure)
}

#[tauri::command]
#[specta::specta]
pub async fn update_openp2p_subscription(profile_uid: std::string::String) -> CmdResult<OpenP2PSubscription> {
    let store = subscription_store()?;
    let meta = find_subscription(&store, &profile_uid)?;
    let text = match meta.source {
        OpenP2PImportSourceKind::Url => {
            let url = meta.source_url.clone().ok_or_else(|| {
                coded_error(
                    "OPENP2P_SUBSCRIPTION_FAILED",
                    "the URL subscription has no saved source URL",
                )
            })?;
            read_import(None, Some(url)).await.map_err(source_failure)?.text
        }
        OpenP2PImportSourceKind::Local => store.read_source(&meta.id).map_err(subscription_failure)?,
    };
    let vault = secret_vault()?;
    let import = run_subscription_write(
        &store,
        &vault,
        SubscriptionWrite::Update {
            text: &text,
            id: &meta.id,
        },
    )
    .await?;
    log_import_warnings(&import);
    let current = Config::profiles().await.latest_arc().current.clone();
    if current.as_deref() == Some(import.profile_uid.as_str()) {
        if let Err(error) = OpenP2PManager::global().reconcile_current().await {
            OpenP2PManager::global().mark_degraded(format!("{error:#}"));
        }
        refresh_runtime().await?;
    }
    subscription_view(&import.meta).await
}

#[tauri::command]
#[specta::specta]
pub async fn activate_openp2p_subscription(profile_uid: std::string::String) -> CmdResult<OpenP2PView> {
    require_platform()?;
    let store = subscription_store()?;
    let runtime = store
        .runtime_for_uid(&profile_uid)
        .map_err(subscription_failure)?
        .ok_or_else(|| {
            coded_error(
                "OPENP2P_SUBSCRIPTION_NOT_FOUND",
                "no openp2p subscription belongs to this profile",
            )
        })?;
    if Config::profiles().await.latest_arc().get_item(&profile_uid).is_err() {
        return Err(coded_error(
            "OPENP2P_SUBSCRIPTION_FAILED",
            "the subscription profile item is missing; import the subscription again",
        ));
    }
    // Validate the source and repair a missing generated profile before the switch.
    store.render(&runtime.meta).map_err(subscription_failure)?;
    ensure_subscription_profile(&store, &runtime)?;

    let outcome = crate::cmd::profile::patch_profiles_config_by_profile_index(profile_uid.as_str().into()).await?;
    if !matches!(outcome, ValidationOutcome::Valid) {
        return Err(coded_error(
            "OPENP2P_ACTIVATION_FAILED",
            format!("mihomo rejected the subscription profile: {outcome}"),
        ));
    }
    // The generic profile switch already reconciled the sidecar to the new
    // profiles.yaml:current. A sidecar failure is recorded as config_error on the
    // view; the valid mihomo switch is never rolled back.
    Ok(OpenP2PManager::global().view())
}

#[tauri::command]
#[specta::specta]
pub async fn detect_openp2p_source(
    yaml: Option<std::string::String>,
    url: Option<std::string::String>,
) -> CmdResult<bool> {
    Ok(read_import(yaml, url)
        .await
        .map(|resolved| portable::is_openp2p_document(&resolved.text))
        .unwrap_or(false))
}

#[tauri::command]
#[specta::specta]
pub async fn get_openp2p_version() -> CmdResult<Option<std::string::String>> {
    let binary = crate::core::openp2p::process::sidecar_binary()
        .map_err(|error| coded_error("OPENP2P_VERSION_FAILED", format!("{error:#}")))?;
    if !binary.is_file() {
        return Ok(None);
    }
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio::process::Command::new(&binary).arg("-v").output(),
    )
    .await
    .map_err(|_| coded_error("OPENP2P_VERSION_FAILED", "openp2p -v timed out"))?
    .map_err(|error| coded_error("OPENP2P_VERSION_FAILED", format!("{error:#}")))?;
    if !output.status.success() {
        return Ok(None);
    }
    let version = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if version.is_empty() || version.len() > 128 {
        return Ok(None);
    }
    Ok(Some(version))
}

#[tauri::command]
#[specta::specta]
pub async fn open_openp2p_log_dir(account_id: std::string::String) -> CmdResult<()> {
    let paths = account::account_paths(&account_id)
        .map_err(|error| coded_error("OPENP2P_ACCOUNT_NOT_FOUND", format!("{error:#}")))?;
    paths
        .ensure()
        .map_err(|error| coded_error("OPENP2P_LOG_DIR_FAILED", format!("{error:#}")))?;
    open::that(&paths.log_dir).map_err(|error| coded_error("OPENP2P_LOG_DIR_FAILED", format!("{error:#}")))
}
