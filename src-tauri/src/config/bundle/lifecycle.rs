//! Orphan cleanup and teardown for the Profile Bundle.

use anyhow::Result;
use clash_verge_draft::Draft;
use clash_verge_logging::{Type, logging};
use tokio::fs;

use super::{
    BUNDLE_UID, IProfiles, TeardownOutcome, clear_dns_state, generated_path, meta_path, multi_merge, remove_artifacts,
};

/// What an orphaned bundle item (missing from profiles) still needs cleaned up.
/// DNS state is orphaned even when no artifacts remain.
pub const fn plan_orphan_cleanup(item_exists: bool, has_artifacts: bool) -> (bool, bool) {
    let orphaned = !item_exists;
    (orphaned && has_artifacts, orphaned)
}

pub async fn cleanup_orphans(profiles: &IProfiles) -> Result<bool> {
    let has_meta = fs::try_exists(meta_path()?).await.unwrap_or(false);
    let has_generated = fs::try_exists(generated_path()?).await.unwrap_or(false);
    let has_artifacts = has_meta || has_generated;
    let item_exists = profiles.get_item(BUNDLE_UID).is_ok();
    let (remove_needed, clear_dns_needed) = plan_orphan_cleanup(item_exists, has_artifacts);
    if !remove_needed && !clear_dns_needed {
        return Ok(false);
    }

    // DNS state is cleared even when artifact removal fails or there are no
    // artifacts left: a stale bundle-main DNS override is orphaned too.
    let removal = if remove_needed {
        remove_artifacts().await
    } else {
        Ok(())
    };
    let dns_removed = match clear_dns_state().await {
        Ok(removed) => removed,
        Err(error) => {
            logging!(
                warn,
                Type::Config,
                "failed to clear orphaned bundle DNS settings: {error:#}"
            );
            false
        }
    };

    if remove_needed {
        multi_merge::publish_conflicts(Vec::new());
        logging!(
            warn,
            Type::Config,
            "removed orphaned profile bundle artifacts because the bundle item is missing"
        );
    }
    removal?;
    Ok(remove_needed || dns_removed)
}

/// Remove the bundle item plus generated file and metadata, leaving members
/// untouched. Returns whether the active profile changed.
pub async fn teardown(profiles: &Draft<IProfiles>) -> Result<TeardownOutcome> {
    let snapshot = profiles.latest_arc();
    let was_current = snapshot.current.as_deref() == Some(BUNDLE_UID);
    let has_item = snapshot.get_item(BUNDLE_UID).is_ok();
    drop(snapshot);

    if has_item {
        let plan = profiles
            .with_data_modify(|mut candidate| async move {
                let plan = candidate.plan_delete_item(BUNDLE_UID)?;
                candidate.save_file().await?;
                Ok((candidate, plan.1))
            })
            .await?;
        plan.cleanup().await;
    }
    let removal = remove_artifacts().await;
    if let Err(error) = clear_dns_state().await {
        logging!(warn, Type::Config, "failed to clear bundle DNS settings: {error:#}");
    }
    removal?;
    multi_merge::publish_conflicts(Vec::new());
    Ok(TeardownOutcome {
        removed: has_item,
        runtime_changed: was_current && has_item,
    })
}
