use super::{CmdResult, CommandFailure, coded_error};
use crate::core::openp2p::account;
use crate::core::openp2p::artifacts::{self, ApplyError, SyncOutcome};
use crate::core::openp2p::config as o2p_config;
use crate::core::openp2p::manager::OpenP2PManager;
use crate::core::openp2p::schema::{self, has_errors};
use crate::core::openp2p::state::{OpenP2PAccount, OpenP2PApplyOutcome, OpenP2PArtifacts, OpenP2PPreview, OpenP2PView};

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

fn apply_failure(error: ApplyError) -> CommandFailure {
    match error {
        ApplyError::Invalid(issues) => {
            let detail = issues
                .iter()
                .map(|issue| format!("{}: {} ({})", issue.path, issue.message, issue.code))
                .collect::<Vec<_>>()
                .join("; ");
            coded_error("OPENP2P_CONFIG_INVALID", detail)
        }
        ApplyError::Rejected(outcome) => coded_error("OPENP2P_APPLY_REJECTED", outcome),
        ApplyError::Profile(message) => coded_error("OPENP2P_PROFILE_UNSUPPORTED", message),
        ApplyError::Io(error) => coded_error("OPENP2P_APPLY_FAILED", format!("{error:#}")),
    }
}

fn account_failure(error: anyhow::Error) -> CommandFailure {
    coded_error("OPENP2P_ACCOUNT_FAILED", format!("{error:#}"))
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
pub async fn preview_openp2p_config(yaml: std::string::String) -> CmdResult<OpenP2PPreview> {
    let mut parsed = schema::parse_seed(&yaml);
    let profiles = crate::config::Config::profiles().await.latest_arc();
    if profiles.current.is_none() {
        parsed.issues.push(crate::core::openp2p::schema::OpenP2PIssue::error(
            "profile_missing",
            "",
            "select a profile before applying an openp2p configuration",
        ));
    }
    if !crate::core::openp2p::manager::PLATFORM_SUPPORTED {
        parsed.issues.push(crate::core::openp2p::schema::OpenP2PIssue::error(
            "platform_unsupported",
            "",
            "openp2p sidecar is supported on macOS only in this build",
        ));
    }
    let (proxies, groups, rules, dns_policy) = artifacts::preview_artifacts(&parsed.config);
    let accounts = artifacts::preview_account_actions(&parsed.config).await;
    let can_apply = !has_errors(&parsed.issues);
    Ok(OpenP2PPreview {
        can_apply,
        issues: parsed.issues,
        accounts,
        proxies,
        groups,
        rules,
        dns_policy,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn apply_openp2p_config(yaml: std::string::String) -> CmdResult<OpenP2PApplyOutcome> {
    require_platform()?;
    let report = artifacts::apply_seed(&yaml).await.map_err(apply_failure)?;
    OpenP2PManager::global()
        .reload_from_current()
        .await
        .map_err(account_failure)?;
    Ok(OpenP2PApplyOutcome {
        applied: true,
        state: OpenP2PManager::global().view(),
        validation: Some(report.validation),
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
    let token = o2p_config::parse_token(token.trim()).map_err(|error| coded_error("OPENP2P_TOKEN_INVALID", error))?;
    o2p_config::write_token(&spec.token_ref, token)
        .map_err(|error| coded_error("OPENP2P_TOKEN_WRITE_FAILED", format!("{error:#}")))?;
    // Restart with the new credential before regenerating artifacts; the artifact reload keeps
    // an unchanged running worker as-is, so doing it in the other order would leave the old token.
    if let Err(error) = manager.refresh_account_secrets(&account_id).await {
        tracing::warn!("openp2p account restart after token save failed: {error:#}");
    }
    if let Err(error) = manager.sync_artifacts().await {
        tracing::warn!("openp2p artifact sync after token save failed: {error:#}");
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
    if let Err(error) = manager.sync_artifacts().await {
        tracing::warn!("openp2p artifact sync after key save failed: {error:#}");
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

#[tauri::command]
#[specta::specta]
pub async fn sync_openp2p_artifacts() -> CmdResult<OpenP2PArtifacts> {
    let manager = OpenP2PManager::global();
    match manager.sync_artifacts().await {
        Ok(SyncOutcome::Applied(report)) => Ok(report.artifacts),
        Ok(SyncOutcome::Cleared | SyncOutcome::Unchanged) => Ok(manager.view().artifacts),
        Err(error) => Err(coded_error("OPENP2P_ARTIFACTS_FAILED", format!("{error:#}"))),
    }
}
