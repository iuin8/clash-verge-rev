use anyhow::Context as _;
use serde::{Deserialize, Serialize};

use super::{CmdResult, CommandFailure, coded_error};
use crate::config::{Config, PrfOption};
use crate::core::notify::{Refresh, announce};
use crate::core::openp2p::account;
use crate::core::openp2p::import_transaction::PreparedSubscriptionImport;
use crate::core::openp2p::manager::OpenP2PManager;
use crate::core::openp2p::portable::{self, MAX_IMPORT_BYTES, OpenP2PImportSourceKind};
use crate::core::openp2p::state::OpenP2PView;
use crate::core::openp2p::subscriptions::{
    self, SubscriptionImport, SubscriptionMeta, SubscriptionRuntime, SubscriptionStore,
};
use crate::core::validate::{CoreConfigValidator, ValidationOutcome};
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
///
/// `option` carries the generic profile fetch settings; only URL reads use them.
pub async fn read_import(
    yaml: Option<std::string::String>,
    url: Option<std::string::String>,
    option: Option<&PrfOption>,
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
            let (proxy_type, timeout_secs, user_agent, accept_invalid_certs) = fetch_settings(option);
            let fetched = NetworkManager::new()
                .get_bytes(
                    url.as_str(),
                    proxy_type,
                    timeout_secs,
                    user_agent,
                    accept_invalid_certs,
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

/// The generic profile fetch settings an OpenP2P URL download honors. Proxy
/// flags use the same precedence as remote profile downloads.
fn fetch_settings(option: Option<&PrfOption>) -> (ProxyType, Option<u64>, Option<smartstring::alias::String>, bool) {
    let proxy_type = if option.and_then(|option| option.self_proxy).unwrap_or(false) {
        ProxyType::Localhost
    } else if option.and_then(|option| option.with_proxy).unwrap_or(false) {
        ProxyType::System
    } else {
        ProxyType::None
    };
    let timeout_secs = option
        .and_then(|option| option.timeout_seconds)
        .or(Some(FETCH_TIMEOUT_SECS));
    let user_agent = option.and_then(|option| option.user_agent.clone());
    let accept_invalid_certs = option
        .and_then(|option| option.danger_accept_invalid_certs)
        .unwrap_or(false);
    (proxy_type, timeout_secs, user_agent, accept_invalid_certs)
}

/// The timer may only update a subscription whose option permits it; a manual
/// update always runs.
pub(crate) fn should_update_openp2p(option: Option<&PrfOption>, is_mannual_trigger: bool) -> bool {
    is_mannual_trigger || option.and_then(|option| option.allow_auto_update).unwrap_or(true)
}

/// Update settings are the only fields an OpenP2P profile patch may change; the
/// merge/script/rules/proxies/groups references belong to ordinary profiles.
pub(crate) fn sanitize_option_patch(option: PrfOption) -> PrfOption {
    PrfOption {
        user_agent: option.user_agent,
        with_proxy: option.with_proxy,
        self_proxy: option.self_proxy,
        update_interval: option.update_interval,
        timeout_seconds: option.timeout_seconds,
        danger_accept_invalid_certs: option.danger_accept_invalid_certs,
        allow_auto_update: option.allow_auto_update,
        ..PrfOption::default()
    }
}

/// Persists a managed card's display fields on the subscription meta, then
/// mirrors them and the merged update options onto the generated profile card.
/// The meta is written first; a failed card write restores the previous display
/// text so the card never wins over the durable copy.
pub(crate) async fn patch_managed_subscription(
    store: &SubscriptionStore,
    meta: &SubscriptionMeta,
    display: &subscriptions::DisplayPatch,
    option: Option<PrfOption>,
) -> anyhow::Result<()> {
    let mut updated = meta.clone();
    let meta_changed = updated.apply_display_patch(display);
    if meta_changed {
        store
            .save_meta(&updated)
            .context("save the OpenP2P subscription display fields")?;
    }
    let updated_meta = updated;
    let display = display.clone();
    let item_result = Config::profiles()
        .await
        .with_data_modify(|mut profiles| async move {
            subscriptions::patch_profile_item(&mut profiles, &updated_meta, &display, option)?;
            profiles.save_file().await?;
            Ok((profiles, ()))
        })
        .await;
    if let Err(error) = item_result {
        if meta_changed && let Err(rollback) = store.save_meta(meta) {
            logging!(
                warn,
                Type::Config,
                "failed to restore the OpenP2P subscription display fields: {rollback:#}"
            );
        }
        return Err(error.context("save the OpenP2P profile card"));
    }
    Ok(())
}

/// A partial edit-info patch keeps the settings it does not mention instead of
/// clearing them; generated overlay references never survive.
pub(crate) fn merge_option_patch(existing: Option<&PrfOption>, patch: PrfOption) -> PrfOption {
    let patch = sanitize_option_patch(patch);
    let Some(existing) = existing else {
        return patch;
    };
    PrfOption {
        user_agent: patch.user_agent.or_else(|| existing.user_agent.clone()),
        with_proxy: patch.with_proxy.or(existing.with_proxy),
        self_proxy: patch.self_proxy.or(existing.self_proxy),
        update_interval: patch.update_interval.or(existing.update_interval),
        timeout_seconds: patch.timeout_seconds.or(existing.timeout_seconds),
        danger_accept_invalid_certs: patch
            .danger_accept_invalid_certs
            .or(existing.danger_accept_invalid_certs),
        allow_auto_update: patch.allow_auto_update.or(existing.allow_auto_update),
        ..PrfOption::default()
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
    let active = current_sidecar_uid().await.as_deref() == Some(profile_uid.as_str());
    Ok(OpenP2PSubscription {
        id: meta.id.clone(),
        profile_uid: profile_uid.clone(),
        name: meta.name.clone(),
        source: meta.source,
        source_url: meta.source_url.clone(),
        imported_at: meta.imported_at,
        account_count: meta.account_count,
        active,
    })
}

/// The subscription profile uid that currently drives the sidecar: the current
/// subscription card itself, or the single OpenP2P subscription member of the
/// current Profile Bundle.
async fn current_sidecar_uid() -> Option<std::string::String> {
    let current = Config::profiles().await.latest_arc().current.clone();
    let current = current?;
    if current == crate::config::bundle::BUNDLE_UID {
        return bundle_subscription_uid().await;
    }
    subscriptions::is_subscription_profile_uid(&current).then(|| current.to_string())
}

async fn bundle_subscription_uid() -> Option<std::string::String> {
    let meta = crate::config::bundle::load_meta().await.ok().flatten()?;
    subscriptions::single_subscription_member(&meta.members).ok().flatten()
}

/// The live OpenP2P subscriptions referenced by the current bundle membership.
/// Used before deleting a bundle so its subscription store can be cleaned up.
pub(crate) async fn bundle_subscriptions() -> Vec<SubscriptionMeta> {
    let Ok(Some(meta)) = crate::config::bundle::load_meta().await else {
        return Vec::new();
    };
    let Ok(store) = SubscriptionStore::app_default() else {
        return Vec::new();
    };
    meta.members
        .iter()
        .filter_map(|uid| store.find_by_profile_uid(uid).ok().flatten())
        .collect()
}

/// After a bundle/profile deletion: converge the sidecar to the new current
/// profile first (so account working directories are idle), then remove the
/// subscription stores that belonged to deleted cards.
pub(crate) async fn cleanup_deleted_subscriptions(subscriptions: &[SubscriptionMeta]) {
    if subscriptions.is_empty() {
        return;
    }
    let manager = OpenP2PManager::global();
    if let Err(error) = manager.ensure_current().await {
        manager.mark_degraded(format!("{error:#}"));
    }
    let Ok(store) = SubscriptionStore::app_default() else {
        return;
    };
    for subscription in subscriptions {
        if let Err(error) = store.remove(&subscription.id) {
            logging!(
                warn,
                Type::Config,
                "failed to remove the openp2p subscription {}: {error:#}",
                subscription.id
            );
        }
    }
}

/// Regenerates the current bundle from its member files after an OpenP2P member
/// subscription changed, so the merged mihomo profile reflects the new content.
async fn refresh_active_bundle() -> CmdResult<()> {
    let _profile_write_guard = crate::config::profiles::PROFILE_WRITE_LOCK.lock().await;
    let profiles = Config::profiles().await;
    let snapshot = profiles.latest_arc();
    if snapshot.current.as_deref() != Some(crate::config::bundle::BUNDLE_UID) {
        return Ok(());
    }
    let artifacts = crate::config::bundle::snapshot_artifacts()
        .await
        .map_err(|error| coded_error("OPENP2P_PROFILE_SYNC_FAILED", format!("{error:#}")))?;
    if let Err(error) = crate::config::bundle::refresh(&snapshot, true, true).await {
        if let Err(rollback) = crate::config::bundle::restore_artifacts(&artifacts).await {
            logging!(
                warn,
                Type::Config,
                "failed to restore bundle artifacts after an openp2p update: {rollback:#}"
            );
        }
        return Err(coded_error(
            "OPENP2P_PROFILE_SYNC_FAILED",
            format!("failed to refresh the active profile bundle: {error:#}"),
        ));
    }
    Ok(())
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

/// The projected profile reached the profile file but the core rejected it.
#[derive(Debug)]
struct ProjectedProfileRejected(std::string::String);

impl std::fmt::Display for ProjectedProfileRejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ProjectedProfileRejected {}

/// Validates the projected profile after it is written and its staged key files
/// exist, so mihomo can resolve `private-key` during parsing. A missing core does
/// not block an inert import; the skip reason is logged.
async fn validate_written_profile(
    store: &SubscriptionStore,
    prepared: &PreparedSubscriptionImport,
) -> std::result::Result<(), ProjectedProfileRejected> {
    let path = store.profile_path(&prepared.outcome().profile_uid);
    let Ok(path) = crate::utils::dirs::path_to_str(&path) else {
        return Ok(());
    };
    match CoreConfigValidator::validate_config_file_outcome(path, None).await {
        Ok(ValidationOutcome::Valid) => Ok(()),
        Ok(ValidationOutcome::Invalid { message, .. }) => Err(ProjectedProfileRejected(format!(
            "the core rejected the projected mihomo profile: {message}"
        ))),
        Ok(ValidationOutcome::Skipped { reason }) => {
            logging!(
                warn,
                Type::Network,
                "openp2p projected profile was not checked by the core: {reason}"
            );
            Ok(())
        }
        Ok(ValidationOutcome::Busy) => {
            logging!(
                warn,
                Type::Network,
                "openp2p projected profile was not checked because core validation is already running"
            );
            Ok(())
        }
        Err(error) => {
            logging!(
                warn,
                Type::Network,
                "openp2p projected profile was not checked by the core: {error:#}"
            );
            Ok(())
        }
    }
}

/// Writes one subscription and its generated profile, then persists the profile
/// item list. On a failed profiles.yaml write the subscription files,
/// credentials and generated profile are rolled back.
async fn run_subscription_write(
    store: &SubscriptionStore,
    write: SubscriptionWrite<'_>,
) -> CmdResult<SubscriptionImport> {
    let _profile_write_guard = crate::config::profiles::PROFILE_WRITE_LOCK.lock().await;
    let preview = match write {
        SubscriptionWrite::Import { kind, text, url } => store.preview_import(kind, text, url),
        SubscriptionWrite::Update { text, id } => store.preview_update(text, id),
    }
    .map_err(subscription_failure)?;
    let profiles = Config::profiles().await;
    profiles
        .with_data_modify(|mut candidate| async move {
            let prepared = store.commit_preview(preview, &mut candidate)?;
            if let Err(rejected) = validate_written_profile(store, &prepared).await {
                if let Err(rollback) = prepared.rollback() {
                    logging!(
                        warn,
                        Type::Config,
                        "openp2p profile validation rollback failed: {rollback:#}"
                    );
                }
                return Err(anyhow::Error::new(rejected));
            }
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
        .map_err(
            |error: anyhow::Error| match error.downcast::<ProjectedProfileRejected>() {
                Ok(rejected) => coded_error("OPENP2P_PROFILE_INVALID", rejected),
                Err(error) => coded_error("OPENP2P_IMPORT_FAILED", format!("{error:#}")),
            },
        )
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

/// Resolves an OpenP2P profile uid to its live subscription. `Ok(None)` means the
/// uid is not a live subscription (a plain profile or a restored orphan card).
pub(crate) fn find_managed_subscription(profile_uid: &str) -> CmdResult<Option<(SubscriptionStore, SubscriptionMeta)>> {
    let Ok(store) = SubscriptionStore::app_default() else {
        return Ok(None);
    };
    match store.find_by_profile_uid(profile_uid) {
        Ok(Some(meta)) => Ok(Some((store, meta))),
        Ok(None) => Ok(None),
        Err(error) => Err(subscription_failure(error)),
    }
}

/// Which caller is updating a subscription. Only a URL-backed source can be
/// refreshed automatically or by a bundle batch; a local source is re-read only
/// by an explicit manual update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SubscriptionUpdateTrigger {
    Manual,
    Automatic,
    Batch,
}

impl SubscriptionUpdateTrigger {
    const fn allows_local(self) -> bool {
        matches!(self, Self::Manual)
    }

    const fn honors_auto_update(self) -> bool {
        matches!(self, Self::Automatic)
    }
}

fn find_subscription_for_update(store: &SubscriptionStore, profile_uid: &str) -> CmdResult<SubscriptionMeta> {
    match find_subscription(store, profile_uid) {
        Ok(meta) => Ok(meta),
        Err(failure) if failure.code.as_deref() == Some("OPENP2P_SUBSCRIPTION_NOT_FOUND") => Err(coded_error(
            "OPENP2P_SUBSCRIPTION_MISSING",
            "the OpenP2P subscription data is missing; re-import the subscription",
        )),
        Err(failure) => Err(failure),
    }
}

/// Reconciles the sidecar only when the current profile resolves to an OpenP2P
/// source. Callers own the bundle/runtime policy.
pub(crate) async fn reconcile_active_sidecar() {
    let current = Config::profiles().await.latest_arc().current.clone();
    let relevant = current.as_deref() == Some(crate::config::bundle::BUNDLE_UID)
        || current
            .as_deref()
            .is_some_and(subscriptions::is_subscription_profile_uid);
    if !relevant {
        return;
    }
    if let Err(error) = OpenP2PManager::global().reconcile_current().await {
        OpenP2PManager::global().mark_degraded(format!("{error:#}"));
    }
}

/// A saved raw source must immediately converge an active bundle and runtime;
/// unlike timer/batch updates it has no coalescing caller behind it.
async fn refresh_saved_subscription(import: &SubscriptionImport) -> CmdResult<()> {
    let current = Config::profiles().await.latest_arc().current.clone();
    let direct_current = current.as_deref() == Some(import.profile_uid.as_str());
    let bundle_member_active = current.as_deref() == Some(crate::config::bundle::BUNDLE_UID)
        && bundle_subscription_uid().await.as_deref() == Some(import.profile_uid.as_str());
    if bundle_member_active {
        refresh_active_bundle().await?;
    }
    if direct_current || bundle_member_active {
        reconcile_active_sidecar().await;
        refresh_runtime().await?;
        if bundle_member_active && let Err(error) = crate::config::bundle::mark_validated_if_current().await {
            logging!(
                warn,
                Type::Config,
                "failed to mark the refreshed bundle validated: {error:#}"
            );
        }
    }
    Ok(())
}

async fn read_subscription_source(
    store: &SubscriptionStore,
    meta: &SubscriptionMeta,
    option: Option<&PrfOption>,
) -> CmdResult<std::string::String> {
    match meta.source {
        OpenP2PImportSourceKind::Url => {
            let url = meta.source_url.clone().ok_or_else(|| {
                coded_error(
                    "OPENP2P_SUBSCRIPTION_FAILED",
                    "the URL subscription has no saved source URL",
                )
            })?;
            read_import(None, Some(url), option)
                .await
                .map(|resolved| resolved.text)
                .map_err(source_failure)
        }
        OpenP2PImportSourceKind::Local => store.read_source(&meta.id).map_err(subscription_failure),
    }
}

/// The profile card's updated timestamp moved; re-arm the delay so the next
/// run counts from this write instead of the previous import.
async fn reschedule_timer_after_managed_write() {
    if let Err(error) = crate::core::Timer::global().reschedule().await {
        logging!(
            warn,
            Type::Timer,
            "failed to reschedule the update timer after an openp2p write: {error:#}"
        );
    }
}

async fn persist_subscription_text(
    store: &SubscriptionStore,
    meta: &SubscriptionMeta,
    text: &str,
) -> CmdResult<SubscriptionImport> {
    let import = run_subscription_write(store, SubscriptionWrite::Update { text, id: &meta.id }).await?;
    log_import_warnings(&import);
    reschedule_timer_after_managed_write().await;
    Ok(import)
}

/// One shared source-read/reimport implementation for the generic manual, timer
/// and bundle-batch paths.
pub(crate) async fn update_subscription(
    profile_uid: &str,
    option: Option<&PrfOption>,
    trigger: SubscriptionUpdateTrigger,
) -> CmdResult<Option<SubscriptionImport>> {
    let store = subscription_store()?;
    let meta = find_subscription_for_update(&store, profile_uid)?;
    if !trigger.allows_local() && meta.source == OpenP2PImportSourceKind::Local {
        logging!(info, Type::Config, "[订阅更新] OpenP2P 本地订阅没有远端来源，跳过更新");
        return Ok(None);
    }
    let item_option = Config::profiles()
        .await
        .latest_arc()
        .get_item(profile_uid)
        .ok()
        .and_then(|item| item.option.clone());
    if trigger.honors_auto_update() && !should_update_openp2p(item_option.as_ref(), false) {
        logging!(info, Type::Config, "[订阅更新] OpenP2P 自动更新已关闭，跳过更新");
        return Ok(None);
    }
    let effective = PrfOption::merge(item_option.as_ref(), option);
    let text = read_subscription_source(&store, &meta, effective.as_ref()).await?;
    persist_subscription_text(&store, &meta, &text).await.map(Some)
}

/// The profile editor's raw-source save for an OpenP2P subscription. The source
/// id, kind and URL stay fixed; only the document content is replaced.
pub(crate) async fn save_subscription_source(profile_uid: &str, text: &str) -> CmdResult<SubscriptionImport> {
    let store = subscription_store()?;
    let meta = find_subscription_for_update(&store, profile_uid)?;
    let import = persist_subscription_text(&store, &meta, text).await?;
    refresh_saved_subscription(&import).await?;
    Ok(import)
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
    let resolved = read_import(yaml, url, None).await.map_err(source_failure)?;
    let store = subscription_store()?;
    let import = run_subscription_write(
        &store,
        SubscriptionWrite::Import {
            kind: resolved.kind,
            text: &resolved.text,
            url: resolved.url.as_deref(),
        },
    )
    .await?;
    log_import_warnings(&import);
    reschedule_timer_after_managed_write().await;
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
                "OPENP2P_SUBSCRIPTION_MISSING",
                "the OpenP2P subscription data is missing; re-import the subscription",
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
    Ok(read_import(yaml, url, None)
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

#[cfg(test)]
#[path = "openp2p_tests.rs"]
mod tests;
