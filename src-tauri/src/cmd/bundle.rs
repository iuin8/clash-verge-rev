//! FORK: Profile Bundle commands — one reusable, materialized local profile
//! assembled from an ordered list of subscription profiles.

use super::{CmdResult, WithErrorCode as _, coded_error};
use crate::cmd::validate::{ValidationNoticeTarget, handle_validation_notice};
use crate::config::{Config, bundle, profiles::PROFILE_WRITE_LOCK};
use crate::core::{CoreManager, handle, tray::Tray};
use crate::enhance::multi_merge::{self, ConflictEntry};
use clash_verge_logging::{Type, logging};
use smartstring::alias::String;

#[derive(serde::Serialize, specta::Type)]
pub struct BundleMemberView {
    pub uid: std::string::String,
    pub name: std::string::String,
    pub kind: std::string::String,
}

#[derive(serde::Serialize, specta::Type)]
pub struct BundleView {
    pub uid: std::string::String,
    pub name: std::string::String,
    pub members: Vec<BundleMemberView>,
    pub stale: bool,
    pub generated_at: u64,
}

/// FORK: describe the reusable bundle item and its ordered members.
#[tauri::command]
#[specta::specta]
pub async fn get_bundle_view() -> CmdResult<Option<BundleView>> {
    let profiles = Config::profiles().await;
    let snapshot = profiles.latest_arc();
    let Ok(bundle_item) = snapshot.get_item(bundle::BUNDLE_UID) else {
        return Ok(None);
    };
    let Some(meta) = bundle::load_meta().await.with_error_code("BUNDLE_META_INVALID")? else {
        return Ok(None);
    };
    let stale = bundle::is_stale(&snapshot, &meta)
        .await
        .with_error_code("BUNDLE_META_INVALID")?;

    let members = meta
        .members
        .iter()
        .map(|uid| match snapshot.get_item(uid) {
            Ok(item) => BundleMemberView {
                uid: uid.to_string(),
                name: item.name.as_ref().map_or_else(|| uid.to_string(), ToString::to_string),
                kind: item
                    .itype
                    .as_ref()
                    .map_or_else(std::string::String::new, ToString::to_string),
            },
            Err(_) => BundleMemberView {
                uid: uid.to_string(),
                name: uid.to_string(),
                kind: "missing".into(),
            },
        })
        .collect();

    Ok(Some(BundleView {
        uid: bundle::BUNDLE_UID.into(),
        name: bundle_item
            .name
            .as_ref()
            .map_or_else(|| bundle::BUNDLE_NAME.to_string(), ToString::to_string),
        members,
        stale,
        generated_at: meta.generated_at,
    }))
}

/// FORK: create or overwrite the single bundle item from ordered member uids,
/// then activate it and apply the generated profile.
#[tauri::command]
#[specta::specta]
pub async fn set_bundle_members(uids: Vec<std::string::String>) -> CmdResult {
    let members: Vec<String> = uids.into_iter().map(Into::into).collect();
    let profile_write_guard = PROFILE_WRITE_LOCK.lock().await;
    let profiles = Config::profiles().await;
    let snapshot = profiles.latest_arc();

    let mut generated = bundle::generate(&snapshot, &members)
        .await
        .with_error_code("BUNDLE_MEMBER_INVALID")?;
    let artifacts = bundle::snapshot_artifacts()
        .await
        .with_error_code("BUNDLE_WRITE_FAILED")?;
    if let Err(error) = bundle::commit_generated(&mut generated, true).await {
        let _ = bundle::restore_artifacts(&artifacts).await;
        return Err(coded_error("BUNDLE_WRITE_FAILED", error));
    }

    let update_result = profiles
        .with_data_modify(|mut candidate| async move {
            let original = candidate.clone();
            bundle::upsert_item(&mut candidate);
            candidate.current = Some(bundle::BUNDLE_UID.into());
            match CoreManager::global()
                .update_config_forced_with_profiles(&candidate, &original)
                .await?
            {
                Ok(guard) => Ok((candidate, Ok(Some(guard)))),
                Err(outcome) => Ok((original, Err(coded_error("BUNDLE_APPLY_FAILED", outcome)))),
            }
        })
        .await;

    let config_update_guard = match update_result {
        Ok(Ok(guard)) => guard,
        Ok(Err(error)) => {
            rollback_bundle_artifacts(Some(&artifacts), "bundle apply rejected").await;
            return Err(error);
        }
        Err(error) => {
            rollback_bundle_artifacts(Some(&artifacts), "bundle apply failed").await;
            return Err(coded_error("BUNDLE_APPLY_FAILED", error));
        }
    };

    if let Err(error) = bundle::mark_validated().await {
        logging!(
            warn,
            Type::Cmd,
            "failed to mark the applied bundle validated: {error:#}"
        );
    }
    multi_merge::publish_conflicts(generated.conflicts);
    drop(config_update_guard);
    drop(profile_write_guard);
    handle::Handle::refresh_clash();
    Ok(())
}

/// FORK: regenerate the bundle from its stored members (stale member recovery).
#[tauri::command]
#[specta::specta]
pub async fn refresh_bundle() -> CmdResult {
    let _profile_write_guard = PROFILE_WRITE_LOCK.lock().await;
    let profiles = Config::profiles().await;
    let snapshot = profiles.latest_arc();
    if snapshot.get_item(bundle::BUNDLE_UID).is_err() {
        return Err(coded_error("BUNDLE_NOT_FOUND", "no profile bundle exists"));
    }

    let artifacts = bundle::snapshot_artifacts()
        .await
        .with_error_code("BUNDLE_REFRESH_FAILED")?;
    let Some(outcome) = (match bundle::refresh(&snapshot, true, false).await {
        Ok(outcome) => outcome,
        Err(error) => {
            let _ = bundle::restore_artifacts(&artifacts).await;
            return Err(coded_error("BUNDLE_REFRESH_FAILED", error));
        }
    }) else {
        return Err(coded_error("BUNDLE_NOT_FOUND", "no profile bundle exists"));
    };

    if outcome.regenerated && snapshot.current.as_deref() == Some(bundle::BUNDLE_UID) {
        if let Err(error) = apply_runtime().await {
            rollback_bundle_artifacts(Some(&artifacts), "bundle refresh apply failed").await;
            return Err(error);
        }
        if let Err(error) = bundle::mark_validated().await {
            logging!(
                warn,
                Type::Cmd,
                "failed to mark the refreshed bundle validated: {error:#}"
            );
        }
    }
    Ok(())
}

/// FORK: remove the bundle item, generated file and metadata; members stay.
#[tauri::command]
#[specta::specta]
pub async fn delete_bundle() -> CmdResult {
    let _profile_write_guard = PROFILE_WRITE_LOCK.lock().await;
    let profiles = Config::profiles().await;
    let outcome = bundle::teardown(&profiles)
        .await
        .with_error_code("BUNDLE_DELETE_FAILED")?;

    if outcome.runtime_changed {
        apply_runtime().await?;
    }
    if let Err(error) = Tray::global().update_menu().await {
        logging!(
            warn,
            Type::Cmd,
            "failed to refresh the tray menu after deleting the bundle: {error:#}"
        );
    }
    Ok(())
}

/// FORK: conflicts produced by the most recent bundle generation.
#[tauri::command]
#[specta::specta]
pub async fn get_bundle_conflicts() -> CmdResult<Vec<ConflictEntry>> {
    Ok(multi_merge::conflicts())
}

/// Restore the previous artifacts BEFORE rebuilding the runtime from the
/// rolled-back profiles: the core's own rollback regenerates from whatever
/// generated file is on disk.
pub(super) async fn rollback_bundle_artifacts(artifacts: Option<&bundle::BundleArtifacts>, context: &str) {
    let Some(artifacts) = artifacts else {
        return;
    };
    if let Err(error) = bundle::restore_artifacts(artifacts).await {
        logging!(
            error,
            Type::Cmd,
            "{context}: failed to restore bundle artifacts: {error:#}"
        );
    }
    match CoreManager::global().update_config_forced().await {
        Ok(outcome) if outcome.is_valid() => handle::Handle::refresh_clash(),
        Ok(outcome) => logging!(
            error,
            Type::Cmd,
            "{context}: runtime rebuild after artifact rollback failed: {outcome}"
        ),
        Err(error) => logging!(
            error,
            Type::Cmd,
            "{context}: runtime rebuild after artifact rollback failed: {error:#}"
        ),
    }
}

async fn apply_runtime() -> CmdResult {
    let outcome = CoreManager::global()
        .update_config_forced()
        .await
        .with_error_code("BUNDLE_APPLY_FAILED")?;
    if outcome.is_valid() {
        handle::Handle::refresh_clash();
        Ok(())
    } else {
        handle_validation_notice(&outcome, ValidationNoticeTarget::Runtime, "运行时配置");
        Err(coded_error("BUNDLE_APPLY_FAILED", outcome))
    }
}
