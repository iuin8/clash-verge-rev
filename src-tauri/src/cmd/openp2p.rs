use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::{CmdResult, CommandFailure, coded_error};
use crate::core::openp2p::account;
use crate::core::openp2p::config as o2p_config;
use crate::core::openp2p::manager::OpenP2PManager;
use crate::core::openp2p::portable::{self, MAX_IMPORT_BYTES, OpenP2PImportSourceKind, ParsedImport};
use crate::core::openp2p::profile_store::{ImportMetadata, ProfileStore};
use crate::core::openp2p::schema::{AppProtocol, AppSpec, OpenP2PConfig, OpenP2PIssue, ProxySpec};
use crate::core::openp2p::state::{OpenP2PAccount, OpenP2PApplyOutcome, OpenP2PPreviewAction, OpenP2PView};
use crate::utils::network::{NetworkManager, ProxyType};
use clash_verge_logging::{Type, logging};

const FETCH_TIMEOUT_SECS: u64 = 20;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct OpenP2PImportSource {
    pub kind: OpenP2PImportSourceKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<u32>,
    pub contains_mihomo: bool,
    /// Every credential this document needs is carried by the document itself.
    pub self_contained: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum OpenP2PSecretState {
    /// Carried by this import and written on apply.
    Inline,
    /// Not in this import, but a credential file already exists.
    Present,
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct OpenP2PImportAccount {
    pub id: std::string::String,
    pub label: std::string::String,
    pub action: OpenP2PPreviewAction,
    pub token: OpenP2PSecretState,
    pub key: OpenP2PSecretState,
    pub nodes: Vec<std::string::String>,
    pub route_count: u32,
    pub warnings: Vec<std::string::String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct OpenP2PImportPreview {
    pub can_apply: bool,
    pub issues: Vec<OpenP2PIssue>,
    pub source: OpenP2PImportSource,
    pub accounts: Vec<OpenP2PImportAccount>,
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
}

impl std::fmt::Debug for ResolvedImport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedImport")
            .field("kind", &self.kind)
            .field("bytes", &self.text.len())
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
                    ImportReadError::new(
                        OpenP2PImportSourceKind::Url,
                        "fetch_failed",
                        format!("failed to fetch the import URL: {error:#}"),
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

const fn secret_state(inline: bool, present: bool) -> OpenP2PSecretState {
    if inline {
        OpenP2PSecretState::Inline
    } else if present {
        OpenP2PSecretState::Present
    } else {
        OpenP2PSecretState::Missing
    }
}

/// Whether a node would be rendered. Mirrors the artifacts build eligibility
/// rule without consulting credentials.
fn renderable_nodes(config: &OpenP2PConfig) -> BTreeSet<std::string::String> {
    config
        .proxies
        .iter()
        .filter_map(|proxy| {
            let account = config.account(&proxy.account)?;
            if !account.enabled {
                return None;
            }
            let tcp_apps: Vec<&AppSpec> = account
                .apps
                .iter()
                .filter(|app| app.enabled.is_enabled() && app.protocol() == AppProtocol::Tcp)
                .collect();
            let selected = match proxy.app.as_deref() {
                Some(name) => tcp_apps.iter().copied().find(|app| app.name == name),
                None if tcp_apps.len() == 1 => tcp_apps.first().copied(),
                None => None,
            };
            selected?;
            proxy.host_key.as_ref()?;
            Some(proxy.name.clone())
        })
        .collect()
}

fn route_count(config: &OpenP2PConfig, account_id: &str) -> u32 {
    let single = config.accounts.len() == 1;
    config
        .networks
        .iter()
        .filter(|network| network.account.as_deref() == Some(account_id) || (network.account.is_none() && single))
        .map(|network| 2 * (network.domains.len() + network.cidrs.len()) as u32)
        .sum()
}

pub(crate) fn build_preview<T, K>(
    parsed: &ParsedImport,
    existing: Option<&OpenP2PConfig>,
    token_present: T,
    key_present: K,
    kind: OpenP2PImportSourceKind,
) -> OpenP2PImportPreview
where
    T: Fn(&str) -> bool,
    K: Fn(&str) -> bool,
{
    let issues = parsed.issues.clone();
    let nodes = renderable_nodes(&parsed.config);
    let mut accounts = Vec::new();
    for account in &parsed.config.accounts {
        let action = match existing.and_then(|old| old.account(&account.id)) {
            None => OpenP2PPreviewAction::Add,
            Some(old) if old == account => OpenP2PPreviewAction::Unchanged,
            Some(_) => OpenP2PPreviewAction::Update,
        };
        let token = secret_state(
            parsed.carries_token(&account.token_ref),
            token_present(&account.token_ref),
        );
        let account_proxies: Vec<&ProxySpec> = parsed
            .config
            .proxies
            .iter()
            .filter(|proxy| proxy.account == account.id)
            .collect();
        let mut key = OpenP2PSecretState::Present;
        for proxy in &account_proxies {
            let state = secret_state(
                parsed.carries_key(&proxy.private_key_ref),
                key_present(&proxy.private_key_ref),
            );
            if state == OpenP2PSecretState::Missing {
                key = OpenP2PSecretState::Missing;
                break;
            }
            if state == OpenP2PSecretState::Inline {
                key = OpenP2PSecretState::Inline;
            }
        }
        let mut warnings = Vec::new();
        if !account.enabled {
            warnings.push("account_disabled".to_owned());
        }
        if account.enabled && token == OpenP2PSecretState::Missing {
            warnings.push("token_missing".to_owned());
        }
        if account.enabled && key == OpenP2PSecretState::Missing {
            warnings.push("key_missing".to_owned());
        }
        accounts.push(OpenP2PImportAccount {
            id: account.id.clone(),
            label: account.label().to_owned(),
            action,
            token,
            key,
            nodes: account_proxies
                .iter()
                .filter(|proxy| nodes.contains(&proxy.name))
                .map(|proxy| proxy.name.clone())
                .collect(),
            route_count: route_count(&parsed.config, &account.id),
            warnings,
        });
    }
    if let Some(existing) = existing {
        for account in &existing.accounts {
            if parsed.config.account(&account.id).is_some() {
                continue;
            }
            let key = if existing
                .proxies_for(&account.id)
                .all(|proxy| key_present(&proxy.private_key_ref))
            {
                OpenP2PSecretState::Present
            } else {
                OpenP2PSecretState::Missing
            };
            accounts.push(OpenP2PImportAccount {
                id: account.id.clone(),
                label: account.label().to_owned(),
                action: OpenP2PPreviewAction::Remove,
                token: secret_state(false, token_present(&account.token_ref)),
                key,
                nodes: Vec::new(),
                route_count: 0,
                warnings: Vec::new(),
            });
        }
    }
    OpenP2PImportPreview {
        can_apply: !issues
            .iter()
            .any(|issue| issue.severity == crate::core::openp2p::schema::IssueSeverity::Error),
        issues,
        source: OpenP2PImportSource {
            kind,
            name: None,
            version: None,
            contains_mihomo: parsed.has_mihomo,
            self_contained: parsed.self_contained(),
        },
        accounts,
    }
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

fn account_failure(error: anyhow::Error) -> CommandFailure {
    coded_error("OPENP2P_ACCOUNT_FAILED", format!("{error:#}"))
}

fn import_failure(issues: &[OpenP2PIssue]) -> CommandFailure {
    let detail = issues
        .iter()
        .map(|issue| format!("{}: {} ({})", issue.path, issue.message, issue.code))
        .collect::<Vec<_>>()
        .join("; ");
    coded_error("OPENP2P_CONFIG_INVALID", detail)
}

fn source_failure(error: ImportReadError) -> CommandFailure {
    let code = match error.code {
        "url_insecure" | "url_invalid" => "OPENP2P_IMPORT_URL_INVALID",
        "fetch_failed" | "source_not_utf8" => "OPENP2P_IMPORT_FETCH_FAILED",
        "source_too_large" => "OPENP2P_IMPORT_TOO_LARGE",
        _ => "OPENP2P_IMPORT_SOURCE_INVALID",
    };
    coded_error(code, error.message)
}

fn key_present(reference: &str) -> bool {
    account::key_path(reference).ok().is_some_and(|path| path.is_file())
}

fn load_existing(store: &ProfileStore) -> Option<OpenP2PConfig> {
    match store.load_config() {
        Ok(config) => config,
        Err(error) => {
            logging!(warn, Type::Config, "failed to read the openp2p source: {error:#}");
            None
        }
    }
}

fn mark_platform_unsupported(preview: &mut OpenP2PImportPreview) {
    if !crate::core::openp2p::manager::PLATFORM_SUPPORTED {
        preview.issues.push(OpenP2PIssue::error(
            "platform_unsupported",
            "",
            "openp2p sidecar is supported on macOS only in this build",
        ));
        preview.can_apply = false;
    }
}

fn failed_preview(error: ImportReadError) -> OpenP2PImportPreview {
    let mut preview = OpenP2PImportPreview {
        can_apply: false,
        issues: vec![OpenP2PIssue::error(error.code, "", error.message)],
        source: OpenP2PImportSource {
            kind: error.kind,
            name: None,
            version: None,
            contains_mihomo: false,
            self_contained: false,
        },
        accounts: Vec::new(),
    };
    mark_platform_unsupported(&mut preview);
    preview
}

async fn refresh_store_profiles() -> CmdResult<()> {
    let store = ProfileStore::app_default()
        .map_err(|error| coded_error("OPENP2P_PROFILE_SYNC_FAILED", format!("{error:#}")))?;
    store
        .project_current()
        .await
        .map_err(|error| coded_error("OPENP2P_PROFILE_SYNC_FAILED", format!("{error:#}")))?;
    OpenP2PManager::global()
        .reload_from_current()
        .await
        .map_err(account_failure)?;
    Ok(())
}

pub(crate) async fn detach_profile_from_bundle(uid: &str) -> anyhow::Result<()> {
    let Some(meta) = crate::config::bundle::load_meta().await? else {
        return Ok(());
    };
    if !meta.members.iter().any(|member| member == uid) {
        return Ok(());
    }
    let members = meta
        .members
        .iter()
        .filter(|member| member.as_str() != uid)
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    if members.len() < crate::config::bundle::MIN_MEMBERS {
        anyhow::bail!(
            "cannot remove {uid}: the Profile Bundle would have fewer than {} members",
            crate::config::bundle::MIN_MEMBERS
        );
    }
    let profiles = crate::config::Config::profiles().await.latest_arc();
    if profiles.current.as_deref() == Some(crate::config::bundle::BUNDLE_UID) {
        crate::cmd::bundle::set_bundle_members(members)
            .await
            .map_err(|error| anyhow::anyhow!("cannot detach {uid} from the active bundle: {error:?}"))?;
    } else {
        let smart_members = members
            .iter()
            .map(|member| member.as_str().into())
            .collect::<Vec<smartstring::alias::String>>();
        let meta = crate::config::bundle::stale_meta_after_prune(&meta, &smart_members);
        crate::config::bundle::save_meta(&meta).await?;
    }
    Ok(())
}

async fn refresh_active_composition() -> CmdResult<()> {
    let profiles = crate::config::Config::profiles().await.latest_arc();
    if profiles.current.as_deref() != Some(crate::config::bundle::BUNDLE_UID) {
        return Ok(());
    }
    let Some(meta) = crate::config::bundle::load_meta()
        .await
        .map_err(|error| coded_error("OPENP2P_BUNDLE_REFRESH_FAILED", format!("{error:#}")))?
    else {
        return Ok(());
    };
    if !meta.members.iter().any(|member| member.starts_with("op2p-")) {
        return Ok(());
    }
    let members = meta.members.iter().map(ToString::to_string).collect::<Vec<_>>();
    crate::cmd::bundle::set_bundle_members(members)
        .await
        .map_err(|error| coded_error("OPENP2P_BUNDLE_REFRESH_FAILED", format!("{error:?}")))?;
    Ok(())
}

fn find_account(view: &OpenP2PView, account_id: &str) -> CmdResult<OpenP2PAccount> {
    view.accounts
        .iter()
        .find(|account| account.id == account_id)
        .cloned()
        .ok_or_else(|| coded_error("OPENP2P_ACCOUNT_NOT_FOUND", "openp2p account not found"))
}

#[tauri::command]
#[specta::specta]
pub async fn get_openp2p_state() -> CmdResult<OpenP2PView> {
    Ok(OpenP2PManager::global().view())
}

#[tauri::command]
#[specta::specta]
pub async fn preview_openp2p_import(
    yaml: Option<std::string::String>,
    url: Option<std::string::String>,
) -> CmdResult<OpenP2PImportPreview> {
    let store =
        ProfileStore::app_default().map_err(|error| coded_error("OPENP2P_IMPORT_FAILED", format!("{error:#}")))?;
    let existing = load_existing(&store);
    match read_import(yaml, url).await {
        Ok(resolved) => {
            let parsed = portable::parse_import(&resolved.text, existing.as_ref());
            let mut preview = build_preview(
                &parsed,
                existing.as_ref(),
                account::token_present,
                key_present,
                resolved.kind,
            );
            mark_platform_unsupported(&mut preview);
            Ok(preview)
        }
        Err(error) => Ok(failed_preview(error)),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn apply_openp2p_import(
    yaml: Option<std::string::String>,
    url: Option<std::string::String>,
) -> CmdResult<OpenP2PApplyOutcome> {
    require_platform()?;
    let resolved = read_import(yaml, url).await.map_err(source_failure)?;
    let store =
        ProfileStore::app_default().map_err(|error| coded_error("OPENP2P_IMPORT_FAILED", format!("{error:#}")))?;
    let existing = load_existing(&store);
    let parsed = portable::parse_import(&resolved.text, existing.as_ref());
    if !parsed.can_apply() {
        return Err(import_failure(&parsed.issues));
    }
    if let Some(existing) = existing.as_ref() {
        let removed = existing
            .accounts
            .iter()
            .filter(|account| !parsed.config.accounts.iter().any(|next| next.id == account.id))
            .map(|account| account.id.clone())
            .collect::<Vec<_>>();
        for account_id in removed {
            detach_profile_from_bundle(&format!("op2p-{account_id}"))
                .await
                .map_err(|error| coded_error("OPENP2P_BUNDLE_DETACH_FAILED", format!("{error:#}")))?;
        }
    }
    let vault = portable::SecretVault::app_default()
        .map_err(|error| coded_error("OPENP2P_IMPORT_APPLY_FAILED", format!("{error:#}")))?;
    let transaction = portable::ImportTransaction::begin(&store, &vault, &parsed)
        .map_err(|error| coded_error("OPENP2P_IMPORT_APPLY_FAILED", format!("{error:#}")))?;
    if let Err(error) = store.project_current().await {
        let detail = format!("{error:#}");
        if let Err(rollback) = transaction.rollback() {
            logging!(warn, Type::Config, "openp2p import rollback failed: {rollback:#}");
        }
        return Err(coded_error("OPENP2P_PROFILE_SYNC_FAILED", detail));
    }
    let metadata = ImportMetadata {
        kind: match resolved.kind {
            OpenP2PImportSourceKind::Local => "local".to_owned(),
            OpenP2PImportSourceKind::Url => "url".to_owned(),
        },
        name: None,
        version: None,
        applied_at: 0,
        contains_mihomo: parsed.has_mihomo,
        self_contained: parsed.self_contained(),
    };
    if let Err(error) = store.save_import_metadata(&metadata) {
        let detail = format!("{error:#}");
        if let Err(rollback) = transaction.rollback() {
            logging!(warn, Type::Config, "openp2p import rollback failed: {rollback:#}");
        }
        return Err(coded_error("OPENP2P_IMPORT_APPLY_FAILED", detail));
    }
    transaction.finish();
    refresh_active_composition().await?;
    if let Err(error) = OpenP2PManager::global().reload_from_current().await {
        return Err(account_failure(error));
    }
    Ok(OpenP2PApplyOutcome {
        applied: true,
        state: OpenP2PManager::global().view(),
        validation: None,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn start_openp2p_account(account_id: std::string::String) -> CmdResult<OpenP2PAccount> {
    require_platform()?;
    OpenP2PManager::global()
        .start_account(&account_id)
        .await
        .map_err(account_failure)
}

#[tauri::command]
#[specta::specta]
pub async fn stop_openp2p_account(account_id: std::string::String) -> CmdResult<OpenP2PAccount> {
    OpenP2PManager::global()
        .stop_account(&account_id)
        .await
        .map_err(account_failure)
}

#[tauri::command]
#[specta::specta]
pub async fn restart_openp2p_account(account_id: std::string::String) -> CmdResult<OpenP2PAccount> {
    require_platform()?;
    OpenP2PManager::global()
        .restart_account(&account_id)
        .await
        .map_err(account_failure)
}

#[tauri::command]
#[tracing::instrument(skip_all, level = "info", fields(account_id = %account_id))]
#[specta::specta]
pub async fn save_openp2p_token(
    account_id: std::string::String,
    token: std::string::String,
) -> CmdResult<OpenP2PAccount> {
    let manager = OpenP2PManager::global();
    let spec = manager
        .account_spec(&account_id)
        .ok_or_else(|| coded_error("OPENP2P_ACCOUNT_NOT_FOUND", "openp2p account not found"))?;
    let token = o2p_config::parse_token(token.trim())
        .map_err(|error| coded_error("OPENP2P_TOKEN_INVALID", format!("{error:#}")))?;
    o2p_config::write_token(&spec.token_ref, token)
        .map_err(|error| coded_error("OPENP2P_TOKEN_WRITE_FAILED", format!("{error:#}")))?;
    // Restart with the new credential before regenerating artifacts; the artifact reload keeps
    // an unchanged running worker as-is, so doing it in the other order would leave the old token.
    if let Err(error) = manager.refresh_account_secrets(&account_id).await {
        tracing::warn!("openp2p account restart after token save failed: {error:#}");
    }
    if let Err(error) = refresh_store_profiles().await {
        tracing::warn!("openp2p profile refresh after token save failed: {error:#}");
    }
    find_account(&manager.view(), &account_id)
}

#[tauri::command]
#[tracing::instrument(skip_all, level = "info", fields(account_id = %account_id))]
#[specta::specta]
pub async fn save_openp2p_key(
    account_id: std::string::String,
    private_key_ref: std::string::String,
    private_key: std::string::String,
) -> CmdResult<OpenP2PAccount> {
    let manager = OpenP2PManager::global();
    let config = manager.config();
    let owned = config
        .proxies
        .iter()
        .any(|proxy| proxy.account == account_id && proxy.private_key_ref == private_key_ref);
    if !owned {
        return Err(coded_error(
            "OPENP2P_KEY_REF_MISMATCH",
            "private_key_ref does not belong to this account",
        ));
    }
    o2p_config::write_private_key(&private_key_ref, &private_key)
        .map_err(|error| coded_error("OPENP2P_KEY_WRITE_FAILED", format!("{error:#}")))?;
    if let Err(error) = refresh_store_profiles().await {
        tracing::warn!("openp2p profile refresh after key save failed: {error:#}");
    }
    find_account(&manager.view(), &account_id)
}

#[tauri::command]
#[specta::specta]
pub async fn delete_openp2p_account(account_id: std::string::String) -> CmdResult<OpenP2PView> {
    OpenP2PManager::global()
        .delete_account(&account_id)
        .await
        .map_err(account_failure)
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
