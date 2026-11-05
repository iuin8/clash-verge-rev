use super::profile::restore_runtime_after_merge_rollback;
use super::{CmdResult, WithErrorCode as _, coded_error};
use crate::config::{Config, IProfiles, profiles::PROFILE_WRITE_LOCK};
use crate::core::{CoreManager, handle};
use clash_verge_logging::{Type, logging};
use smartstring::alias::String;

/// FORK: read the ordered merged profile UIDs.
#[tauri::command]
#[specta::specta]
pub async fn get_merged_profiles() -> CmdResult<Vec<std::string::String>> {
    let uids = crate::config::merged::load()
        .await
        .with_error_code("MERGED_PROFILE_LOAD_FAILED")?;
    Ok(uids.into_iter().map(Into::into).collect())
}

async fn commit_merged_profiles(uids: Option<Vec<String>>) -> CmdResult {
    let profile_write_guard = PROFILE_WRITE_LOCK.lock().await;
    let profiles = Config::profiles().await;
    let clearing = uids.is_none();
    let previous_merged = match crate::config::merged::load().await {
        Ok(previous) => previous,
        Err(error) if clearing => {
            logging!(warn, Type::Cmd, "clearing corrupt merged list: {error:#}");
            Vec::new()
        }
        Err(error) => return Err(coded_error("MERGED_PROFILE_LOAD_FAILED", error)),
    };
    let next: Vec<String> = uids.unwrap_or_default();

    {
        let snapshot = profiles.latest_arc();
        for uid in &next {
            let item = snapshot
                .get_item(uid.as_str())
                .with_error_code("MERGED_PROFILE_INVALID")?;
            if !matches!(item.itype.as_deref(), Some("local" | "remote")) || item.file.is_none() {
                return Err(coded_error(
                    "MERGED_PROFILE_INVALID",
                    format!("profile {uid} is not a subscription"),
                ));
            }
        }
    }

    let update_result = profiles
        .with_data_modify(|mut candidate| async move {
            let original = candidate.clone();
            if let Some(primary) = next.first().cloned() {
                candidate.patch_config(&IProfiles {
                    current: Some(primary),
                    items: None,
                });
            }
            if let Err(error) = crate::config::merged::save(&next).await {
                return Ok((original, Err(coded_error("MERGED_PROFILE_SAVE_FAILED", error))));
            }
            match CoreManager::global()
                .update_config_forced_with_profiles(&candidate, &original)
                .await?
            {
                Ok(guard) => Ok((candidate, Ok(Some(guard)))),
                Err(outcome) => Ok((original, Err(coded_error("MERGED_PROFILE_FAILED", outcome)))),
            }
        })
        .await;

    let result = match update_result {
        Ok(result) => result,
        Err(error) => {
            restore_runtime_after_merge_rollback(&previous_merged).await;
            return Err(coded_error("MERGED_PROFILE_FAILED", error));
        }
    };

    let config_update_guard = match result {
        Ok(guard) => guard,
        Err(error) => {
            restore_runtime_after_merge_rollback(&previous_merged).await;
            return Err(error);
        }
    };

    drop(config_update_guard);
    drop(profile_write_guard);
    handle::Handle::refresh_clash();
    Ok(())
}

/// FORK: set the ordered list of profile UIDs to merge.
#[tauri::command]
#[specta::specta]
pub async fn set_merged_profiles(uids: Vec<std::string::String>) -> CmdResult {
    commit_merged_profiles(Some(uids.into_iter().map(Into::into).collect())).await
}

/// FORK: clear the merged list and return to single-profile mode.
#[tauri::command]
#[specta::specta]
pub async fn clear_merged_profiles() -> CmdResult {
    commit_merged_profiles(None).await
}

// FORK: conflicts from the latest multi-profile merge.
