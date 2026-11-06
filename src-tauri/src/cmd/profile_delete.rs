//! Profile deletion, including the Profile Bundle side effects.

use super::{CmdResult, coded_error};
use crate::cmd::validate::{ValidationNoticeTarget, handle_validation_notice};
use crate::config::{
    Config, IProfiles,
    profiles::{self, PROFILE_WRITE_LOCK},
};
use crate::core::{CoreManager, handle, timer::Timer, tray::Tray};
use clash_verge_draft::Draft;
use clash_verge_logging::{Type, logging, logging_error};
use smartstring::alias::String;

async fn load_bundle_meta_for_delete(deleting_bundle: bool) -> CmdResult<Option<crate::config::bundle::BundleMeta>> {
    match crate::config::bundle::load_meta().await {
        Ok(meta) => Ok(meta),
        Err(error) if deleting_bundle => {
            // Corrupt metadata must not block deleting the bundle itself (that is the repair).
            logging!(
                warn,
                Type::Cmd,
                "ignoring corrupt bundle metadata while deleting the bundle: {error:#}"
            );
            Ok(None)
        }
        Err(error) => Err(coded_error(
            "BUNDLE_META_INVALID",
            format!("invalid bundle metadata: {error:#}"),
        )),
    }
}

async fn finish_profile_delete(needs_runtime_update: bool, current: Option<&String>) {
    if let Err(e) = Tray::global().update_tooltip().await {
        logging!(warn, Type::Cmd, "异步更新托盘提示失败: {e:#}");
    }
    if let Err(e) = Tray::global().update_menu().await {
        logging!(warn, Type::Cmd, "异步更新托盘菜单失败: {e:#}");
    }
    if needs_runtime_update {
        handle::Handle::refresh_clash();
        if let Some(current) = current {
            handle::Handle::notify_profile_changed(current);
        }
    }
    logging_error!(Type::Timer, Timer::global().refresh().await);
}

struct BundleDeletePlan {
    affects_bundle: bool,
    remove_bundle_item: bool,
    bundle_was_current: bool,
    artifacts: Option<crate::config::bundle::BundleArtifacts>,
}

/// Prepare the bundle side of a profile deletion before the profile transaction:
/// regeneration happens while the deleted member still exists on disk, and the
/// profile deletion applies (or rolls back) together with the runtime update.
async fn prepare_bundle_delete(
    profiles: &Draft<IProfiles>,
    index: &str,
    deleting_bundle: bool,
    meta: Option<&crate::config::bundle::BundleMeta>,
) -> anyhow::Result<BundleDeletePlan> {
    let bundle_was_current = profiles.latest_arc().current.as_deref() == Some(crate::config::bundle::BUNDLE_UID);
    let unchanged = BundleDeletePlan {
        affects_bundle: false,
        remove_bundle_item: deleting_bundle,
        bundle_was_current,
        artifacts: None,
    };
    if deleting_bundle {
        return Ok(unchanged);
    }
    let Some(meta) = meta else {
        return Ok(unchanged);
    };

    match crate::config::bundle::plan_prune(meta, index) {
        crate::config::bundle::PrunePlan::Unchanged => Ok(unchanged),
        crate::config::bundle::PrunePlan::Regenerate(remaining) => {
            let artifacts = crate::config::bundle::snapshot_artifacts().await?;
            if bundle_was_current {
                let snapshot = profiles.latest_arc();
                if let Err(error) = crate::config::bundle::regenerate(snapshot.as_ref(), &remaining, true).await {
                    let _ = crate::config::bundle::restore_artifacts(&artifacts).await;
                    return Err(error);
                }
            } else {
                // Inactive bundle: prune metadata only and mark the old generated
                // file stale; regeneration waits for activation or explicit refresh.
                let stale = crate::config::bundle::stale_meta_after_prune(meta, &remaining);
                if let Err(error) = crate::config::bundle::save_meta(&stale).await {
                    let _ = crate::config::bundle::restore_artifacts(&artifacts).await;
                    return Err(error);
                }
            }
            Ok(BundleDeletePlan {
                affects_bundle: true,
                remove_bundle_item: false,
                bundle_was_current,
                artifacts: Some(artifacts),
            })
        }
        crate::config::bundle::PrunePlan::RemoveBundle => Ok(BundleDeletePlan {
            affects_bundle: true,
            remove_bundle_item: true,
            bundle_was_current,
            artifacts: None,
        }),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn delete_profile(index: std::string::String) -> CmdResult {
    let index: String = index.into();
    let deleting_bundle = crate::config::bundle::is_bundle_uid(&index);
    let profile_write_guard = PROFILE_WRITE_LOCK.lock().await;
    let profiles = Config::profiles().await;

    let bundle_meta = load_bundle_meta_for_delete(deleting_bundle).await?;

    let bundle_plan = prepare_bundle_delete(&profiles, &index, deleting_bundle, bundle_meta.as_ref())
        .await
        .map_err(|error| coded_error("BUNDLE_UPDATE_FAILED", format!("cannot delete this member: {error:#}")))?;
    let affects_bundle = bundle_plan.affects_bundle;
    let remove_bundle_item = bundle_plan.remove_bundle_item;
    let bundle_was_current = bundle_plan.bundle_was_current;
    let bundle_artifacts = bundle_plan.artifacts;

    let update_result = profiles
        .with_data_modify(|mut candidate| async move {
            let original = candidate.clone();
            let (deleting_current, mut plan) = candidate.plan_delete_item(&index)?;
            if remove_bundle_item
                && !crate::config::bundle::is_bundle_uid(&index)
                && candidate.get_item(crate::config::bundle::BUNDLE_UID).is_ok()
            {
                let (_, bundle_delete) = candidate.plan_delete_item(crate::config::bundle::BUNDLE_UID)?;
                plan.absorb(bundle_delete);
            }
            let needs_runtime_update = deleting_current || (affects_bundle && bundle_was_current);
            let guard = if needs_runtime_update {
                match CoreManager::global()
                    .update_config_forced_with_profiles(&candidate, &original)
                    .await?
                {
                    Ok(guard) => Some(guard),
                    Err(outcome) => return Ok((original, Err(outcome))),
                }
            } else {
                candidate.save_file().await?;
                None
            };
            let current = candidate.current.clone();
            plan.cleanup().await;
            Ok((candidate, Ok((needs_runtime_update, current, guard))))
        })
        .await;

    let (needs_runtime_update, current, config_update_guard) = match update_result {
        Ok(Ok(result)) => result,
        Ok(Err(outcome)) => {
            super::bundle::rollback_bundle_artifacts(bundle_artifacts.as_ref(), "profile delete rejected").await;
            handle_validation_notice(&outcome, ValidationNoticeTarget::Runtime, "运行时配置");
            return Err(coded_error("PROFILE_DELETE_FAILED", outcome));
        }
        Err(error) => {
            super::bundle::rollback_bundle_artifacts(bundle_artifacts.as_ref(), "profile delete failed").await;
            return Err(coded_error("PROFILE_DELETE_FAILED", error));
        }
    };

    if remove_bundle_item {
        if let Err(error) = crate::config::bundle::remove_artifacts().await {
            logging!(warn, Type::Cmd, "failed to remove bundle artifacts: {error:#}");
        }
        if let Err(error) = crate::config::bundle::clear_dns_state().await {
            logging!(warn, Type::Cmd, "failed to clear bundle DNS settings: {error:#}");
        }
    }
    if needs_runtime_update {
        profiles::activate_selected_nodes();
    }
    drop(config_update_guard);
    drop(profile_write_guard);
    finish_profile_delete(needs_runtime_update, current.as_ref()).await;
    Ok(())
}
