use super::{CmdResult, coded_error};
use crate::{
    cmd::StringifyErr as _,
    cmd::validate::{ValidationNoticeTarget, handle_validation_notice},
    config::{Config, IProfiles, PrfItem},
    core::auto_backup::{AutoBackupManager, AutoBackupTrigger},
    core::{
        CoreManager, handle,
        validate::{CoreConfigValidator, ValidationOutcome},
    },
    utils::dirs,
};
use clash_verge_logging::{Type, logging};
use smartstring::alias::String;
use tokio::fs;

/// 保存profiles的配置
#[tauri::command]
#[specta::specta]
pub async fn save_profile_file(
    index: std::string::String,
    file_data: Option<std::string::String>,
) -> CmdResult<ValidationOutcome> {
    let index: String = index.into();
    if crate::config::bundle::is_bundle_uid(&index) {
        return Err(coded_error(
            "BUNDLE_GENERATED_READ_ONLY",
            "the bundle profile is generated from its members; edit the bundle members instead",
        ));
    }
    let file_data = file_data.map(String::from);
    let file_data = match file_data {
        Some(d) => d,
        None => return Ok(ValidationOutcome::Valid),
    };

    let bundle_member = match crate::config::bundle::is_member(&index).await {
        Ok(is_member) => is_member,
        Err(error) => {
            logging!(
                warn,
                Type::Config,
                "failed to read bundle metadata while saving profile '{index}': {error:#}"
            );
            false
        }
    };

    let backup_trigger = match index.as_str() {
        "Merge" => Some(AutoBackupTrigger::GlobalMerge),
        "Script" => Some(AutoBackupTrigger::GlobalScript),
        _ => None,
    };

    // 在异步操作前获取必要元数据并释放锁
    let (rel_path, is_merge_file, is_script_file, affects_runtime, bundle_active) = {
        let profiles = Config::profiles().await;
        let profiles_guard = profiles.latest_arc();
        let item = profiles_guard.get_item(&index).stringify_err()?;
        let is_merge = item.itype.as_ref().is_some_and(|t| t == "merge");
        let path = item.file.clone().ok_or("file field is null")?;
        let is_script = item.itype.as_ref().is_some_and(|t| t == "script") || path.ends_with(".js");
        let affects_runtime = profile_affects_runtime(&profiles_guard, &index, bundle_member);
        // Bundle validation only tracks the ACTIVE bundle. `affects_runtime` can
        // also be true when the saved file is the current (non-bundle) member.
        let bundle_active = profiles_guard.current.as_deref() == Some(crate::config::bundle::BUNDLE_UID);
        (path, is_merge, is_script, affects_runtime, bundle_active)
    };

    let bundle_artifacts = if bundle_member {
        Some(crate::config::bundle::snapshot_artifacts().await.stringify_err()?)
    } else {
        None
    };

    let profiles_dir = dirs::app_profiles_dir().stringify_err()?;
    let file_path = profiles_dir.join(rel_path.as_str());
    let file_path_str = file_path.to_string_lossy().to_string();

    // 读取原始内容（在释放profiles_guard后进行）
    let original_existed = fs::try_exists(&file_path).await.map_err(|err| {
        String::from(format!(
            "failed to check profile file \"{}\": {err}",
            file_path.display()
        ))
    })?;
    let original_content = if original_existed {
        PrfItem {
            file: Some(rel_path.clone()),
            ..Default::default()
        }
        .read_file()
        .await
        .stringify_err()?
    } else {
        String::new()
    };

    // 保存新的配置文件
    fs::write(&file_path, &file_data).await.stringify_err()?;

    let changes_applied = handle_saved_profile_file(
        &file_path_str,
        &file_path,
        &original_content,
        original_existed,
        SavedFileContext {
            is_merge_file,
            is_script_file,
            affects_runtime,
            bundle_member,
            bundle_active,
            bundle_artifacts: bundle_artifacts.as_ref(),
        },
    )
    .await?;

    if changes_applied.is_valid()
        && let Some(trigger) = backup_trigger
    {
        AutoBackupManager::trigger_backup(trigger);
    }

    Ok(changes_applied)
}

async fn restore_original(
    file_path: &std::path::Path,
    original_content: &str,
    original_existed: bool,
) -> CmdResult<()> {
    if original_existed {
        fs::write(file_path, original_content).await.stringify_err()
    } else {
        fs::remove_file(file_path).await.stringify_err()
    }
}

async fn restore_original_and_bundle(
    file_path: &std::path::Path,
    original_content: &str,
    original_existed: bool,
    bundle_artifacts: Option<&crate::config::bundle::BundleArtifacts>,
) -> CmdResult<()> {
    restore_original(file_path, original_content, original_existed).await?;
    if let Some(artifacts) = bundle_artifacts
        && let Err(error) = crate::config::bundle::restore_artifacts(artifacts).await
    {
        logging!(
            warn,
            Type::Config,
            "[cmd配置save] failed to restore bundle artifacts: {error:#}"
        );
    }
    Ok(())
}

fn profile_affects_runtime(profiles: &IProfiles, index: &str, bundle_member: bool) -> bool {
    let Some(current_uid) = profiles.current.as_ref() else {
        return false;
    };
    if current_uid == index {
        return true;
    }
    if bundle_member && crate::config::bundle::is_bundle_uid(current_uid) {
        return true;
    }

    let Ok(item) = profiles.get_item(current_uid) else {
        return false;
    };
    [
        item.current_merge().map_or("Merge", String::as_str),
        item.current_script().map_or("Script", String::as_str),
        item.current_rules().map_or("Rules", String::as_str),
        item.current_proxies().map_or("Proxies", String::as_str),
        item.current_groups().map_or("Groups", String::as_str),
    ]
    .contains(&index)
}

struct SavedFileContext<'a> {
    is_merge_file: bool,
    is_script_file: bool,
    affects_runtime: bool,
    bundle_member: bool,
    bundle_active: bool,
    bundle_artifacts: Option<&'a crate::config::bundle::BundleArtifacts>,
}

/// Only an active bundle may be refreshed with unvalidated output or have a
/// refresh failure treated as fatal to the save. Keeping `affects_runtime`
/// separate prevents an inactive bundle from being falsely marked validated
/// when the saved member happens to be the current profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MemberSaveBundlePolicy {
    allow_unvalidated: bool,
    fatal_refresh_error: bool,
    mark_after_apply: bool,
}

const fn member_save_bundle_policy(bundle_active: bool) -> MemberSaveBundlePolicy {
    MemberSaveBundlePolicy {
        allow_unvalidated: bundle_active,
        fatal_refresh_error: bundle_active,
        mark_after_apply: bundle_active,
    }
}

/// Refresh the bundle after a member file save. Active bundles must roll the
/// whole file back on failure; inactive bundles keep the edit and stay stale
/// until activation/refresh.
async fn refresh_saved_member_bundle(context: &SavedFileContext<'_>) -> std::result::Result<(), std::string::String> {
    let profiles = Config::profiles().await;
    let snapshot = profiles.latest_arc();
    let _profile_write = crate::config::profiles::PROFILE_WRITE_LOCK.lock().await;
    let policy = member_save_bundle_policy(context.bundle_active);
    match crate::config::bundle::refresh(snapshot.as_ref(), true, policy.allow_unvalidated).await {
        Ok(_) => Ok(()),
        Err(error) if policy.fatal_refresh_error => Err(error.to_string()),
        Err(error) => {
            logging!(
                warn,
                Type::Config,
                "[cmd配置save] bundle stays stale until it is activated or refreshed: {error:#}"
            );
            if let Some(artifacts) = context.bundle_artifacts
                && let Err(error) = crate::config::bundle::restore_artifacts(artifacts).await
            {
                logging!(
                    warn,
                    Type::Config,
                    "[cmd配置save] failed to restore bundle artifacts: {error:#}"
                );
            }
            Ok(())
        }
    }
}

async fn handle_saved_profile_file(
    file_path_str: &str,
    file_path: &std::path::Path,
    original_content: &str,
    original_existed: bool,
    context: SavedFileContext<'_>,
) -> CmdResult<ValidationOutcome> {
    let (target, file_type) = if context.is_script_file {
        (ValidationNoticeTarget::Script, "脚本文件")
    } else if context.is_merge_file {
        (ValidationNoticeTarget::Merge, "合并配置文件")
    } else {
        (ValidationNoticeTarget::Runtime, "YAML配置文件")
    };

    match CoreConfigValidator::validate_config_file_outcome(file_path_str, Some(context.is_merge_file)).await {
        Ok(outcome) if outcome.is_valid() => {}
        Ok(outcome) => {
            logging!(warn, Type::Config, "[cmd配置save] 文件验证失败: {}", outcome);
            restore_original_and_bundle(file_path, original_content, original_existed, context.bundle_artifacts)
                .await?;
            handle_validation_notice(&outcome, target, file_type);
            return Ok(outcome);
        }
        Err(e) => {
            logging!(error, Type::Config, "[cmd配置save] 验证过程发生错误: {e:#}");
            restore_original_and_bundle(file_path, original_content, original_existed, context.bundle_artifacts)
                .await?;
            return Err(e.to_string().into());
        }
    }

    if context.bundle_member
        && let Err(message) = refresh_saved_member_bundle(&context).await
    {
        logging!(
            error,
            Type::Config,
            "[cmd配置save] bundle regeneration failed: {message}"
        );
        restore_original_and_bundle(file_path, original_content, original_existed, context.bundle_artifacts).await?;
        return Err(message.into());
    }

    if !context.affects_runtime {
        return Ok(ValidationOutcome::Valid);
    }

    logging!(
        info,
        Type::Config,
        "[cmd配置save] 保存项影响当前运行时配置，开始统一应用"
    );
    match CoreManager::global().update_config_forced().await {
        Ok(outcome) if outcome.is_valid() => {
            if context.bundle_member
                && member_save_bundle_policy(context.bundle_active).mark_after_apply
                && let Err(error) = crate::config::bundle::mark_validated_if_current().await
            {
                logging!(
                    warn,
                    Type::Config,
                    "[cmd配置save] failed to mark the applied bundle validated: {error:#}"
                );
            }
            handle::Handle::refresh_clash();
            Ok(ValidationOutcome::Valid)
        }
        Ok(outcome) => {
            logging!(warn, Type::Config, "[cmd配置save] 运行时配置应用失败: {}", outcome);
            restore_original_and_bundle(file_path, original_content, original_existed, context.bundle_artifacts)
                .await?;
            handle_validation_notice(&outcome, ValidationNoticeTarget::Runtime, "运行时配置");
            Ok(outcome)
        }
        Err(err) => {
            logging!(error, Type::Config, "[cmd配置save] 运行时配置应用错误: {err:#}");
            restore_original_and_bundle(file_path, original_content, original_existed, context.bundle_artifacts)
                .await?;
            Err(err.to_string().into())
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{MemberSaveBundlePolicy, member_save_bundle_policy};

    #[test]
    fn inactive_bundle_never_uses_member_runtime_state_for_bundle_validation() {
        // Regression: saving the CURRENT member while the bundle is inactive makes
        // `affects_runtime` true, but must not allow unvalidated bundle writes,
        // treat refresh failures as fatal, or mark the bundle validated.
        let policy = member_save_bundle_policy(false);
        assert_eq!(
            policy,
            MemberSaveBundlePolicy {
                allow_unvalidated: false,
                fatal_refresh_error: false,
                mark_after_apply: false,
            }
        );

        let active = member_save_bundle_policy(true);
        assert!(active.allow_unvalidated);
        assert!(active.fatal_refresh_error);
        assert!(active.mark_after_apply);
    }
}
