use super::profile::restore_runtime_after_merge_rollback;
use super::{CmdResult, WithErrorCode as _, coded_error};
use crate::config::{Config, IProfiles, profiles::PROFILE_WRITE_LOCK};
use crate::core::{CoreManager, handle};
use clash_verge_logging::{Type, logging};
use smartstring::alias::String;

/// Pure outcome of deleting one member from the merged list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeleteMergePlan {
    pub(crate) remaining: Vec<String>,
    pub(crate) repaired_current: Option<String>,
    pub(crate) needs_runtime_update: bool,
}

pub(crate) fn plan_delete_merge(
    current: Option<&String>,
    deleted: &str,
    merged: &[String],
    should_update: bool,
) -> DeleteMergePlan {
    let remaining: Vec<String> = merged.iter().filter(|uid| uid.as_str() != deleted).cloned().collect();
    DeleteMergePlan {
        repaired_current: repaired_current(current, &remaining),
        needs_runtime_update: should_update || merged.iter().any(|uid| uid == deleted),
        remaining,
    }
}

/// A single-profile switch clears merge mode only when the target is not one
/// of its members.
pub(crate) fn clears_merge_for_switch(target: Option<&String>, merged: &[String]) -> bool {
    target.is_some_and(|target| !merged.is_empty() && !merged.iter().any(|uid| uid == target))
}

/// After deleting a merged member, keep `current` on a remaining member rather
/// than a profile outside the merge.
pub(crate) fn repaired_current(current: Option<&String>, remaining: &[String]) -> Option<String> {
    if remaining.is_empty() {
        return None;
    }
    if current.is_some_and(|current| remaining.iter().any(|uid| uid == current)) {
        return None;
    }
    remaining.first().cloned()
}

pub(crate) fn validate_merged_members(profiles: &IProfiles, uids: &[String]) -> CmdResult<()> {
    for uid in uids {
        let item = profiles
            .get_item(uid.as_str())
            .with_error_code("MERGED_PROFILE_INVALID")?;
        if !matches!(item.itype.as_deref(), Some("local" | "remote")) || item.file.is_none() {
            return Err(coded_error(
                "MERGED_PROFILE_INVALID",
                format!("profile {uid} is not a subscription"),
            ));
        }
    }
    Ok(())
}

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

    validate_merged_members(profiles.latest_arc().as_ref(), &next)?;

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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::{clears_merge_for_switch, plan_delete_merge, repaired_current};

    fn uids(list: &[&str]) -> Vec<smartstring::alias::String> {
        list.iter().map(|uid| (*uid).into()).collect()
    }

    #[test]
    fn switch_clears_only_when_target_is_outside_merge() {
        let merged = uids(&["a", "b"]);
        assert!(!clears_merge_for_switch(Some(&"a".into()), &merged));
        assert!(clears_merge_for_switch(Some(&"c".into()), &merged));
        assert!(!clears_merge_for_switch(None, &merged));
        assert!(!clears_merge_for_switch(Some(&"a".into()), &[]));
    }

    #[test]
    fn delete_repairs_current_when_it_left_the_merged_members() {
        let remaining = uids(&["b", "c"]);
        assert_eq!(repaired_current(Some(&"a".into()), &remaining), Some("b".into()));
        assert_eq!(repaired_current(Some(&"b".into()), &remaining), None);
        assert_eq!(repaired_current(None, &remaining), Some("b".into()));
        assert_eq!(repaired_current(Some(&"a".into()), &[]), None);
    }

    #[test]
    fn merge_member_validation_rejects_chain_and_accepts_subscriptions() {
        use crate::config::{IProfiles, PrfItem};

        let item = |uid: &str, itype: &str, file: Option<&str>| PrfItem {
            uid: Some(uid.into()),
            itype: Some(itype.into()),
            file: file.map(Into::into),
            ..PrfItem::default()
        };
        let profiles = IProfiles {
            current: Some("local".into()),
            items: Some(vec![
                item("local", "local", Some("local.yaml")),
                item("remote", "remote", Some("remote.yaml")),
                item("merge", "merge", Some("merge.yaml")),
                item("missing-file", "remote", None),
            ]),
        };
        let uids = |list: &[&str]| list.iter().map(|uid| (*uid).into()).collect::<Vec<_>>();

        assert!(super::validate_merged_members(&profiles, &uids(&["local", "remote"])).is_ok());
        for rejected in [uids(&["merge"]), uids(&["missing-file"]), uids(&["absent"])] {
            let error = super::validate_merged_members(&profiles, &rejected).unwrap_err();
            assert_eq!(error.code.as_deref(), Some("MERGED_PROFILE_INVALID"));
        }
    }

    #[test]
    fn delete_plan_handles_current_and_runtime_decisions() {
        let merged = uids(&["a", "b"]);
        let current = "a".into();

        let plan = plan_delete_merge(Some(&current), "a", &merged, false);
        assert_eq!(plan.remaining, uids(&["b"]));
        assert_eq!(plan.repaired_current, Some("b".into()));
        assert!(plan.needs_runtime_update);

        let plan = plan_delete_merge(Some(&current), "b", &merged, false);
        assert_eq!(plan.remaining, uids(&["a"]));
        assert_eq!(plan.repaired_current, None);
        assert!(plan.needs_runtime_update);

        let plan = plan_delete_merge(Some(&current), "other", &merged, false);
        assert_eq!(plan.remaining, merged);
        assert_eq!(plan.repaired_current, None);
        assert!(!plan.needs_runtime_update);

        let plan = plan_delete_merge(Some(&current), "a", &uids(&["a"]), false);
        assert!(plan.remaining.is_empty());
        assert_eq!(plan.repaired_current, None);
    }
}
