//! FORK: Bundle-aware profile update wrapper.
//!
//! Upstream `feat::profile::update_profile` stays as close to upstream as
//! possible. The two update call sites (command + timer) go through this module
//! so every Profile Bundle policy lives in one fork-only file.

use anyhow::{Result, bail};
use clash_verge_logging::{Type, logging};
use smartstring::alias::String;

use crate::{
    config::{Config, PrfOption, bundle},
    core::{
        CoreManager, handle,
        notify::{NoticeStatus, Refresh, announce},
        validate::ValidationOutcome,
    },
    utils::help::mask_err,
};

use super::profile::{perform_profile_update, should_update_profile};
use crate::core::openp2p::subscriptions::is_subscription_profile_uid;

/// Which update path a profile uid takes: OpenP2P subscriptions own their
/// source, bundle members go through bundle policy, and everything else is an
/// ordinary upstream profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpdateRoute {
    OpenP2PSubscription,
    Ordinary,
}

fn update_route(uid: &str) -> UpdateRoute {
    if is_subscription_profile_uid(uid) {
        UpdateRoute::OpenP2PSubscription
    } else {
        UpdateRoute::Ordinary
    }
}

fn openp2p_failure(error: crate::cmd::CommandFailure) -> anyhow::Error {
    anyhow::anyhow!("{error}")
}

/// Whether the uid resolves to a live OpenP2P subscription store entry. Orphan
/// cards keep their prefix but must fall back to ordinary profile behavior.
fn has_live_openp2p_subscription(uid: &str) -> Result<bool> {
    if update_route(uid) != UpdateRoute::OpenP2PSubscription {
        return Ok(false);
    }
    Ok(crate::cmd::openp2p::find_managed_subscription(uid)
        .map_err(openp2p_failure)?
        .is_some())
}

/// Fetch and persist a remote profile **without** regenerating or applying the
/// bundle/runtime. Used by batch bundle refresh, which applies exactly once after
/// every member is updated.
pub async fn fetch_and_persist_profile(
    uid: &String,
    option: Option<&PrfOption>,
    is_mannual_trigger: bool,
) -> Result<bool> {
    if has_live_openp2p_subscription(uid)? {
        let updated = crate::cmd::openp2p::update_subscription(
            uid,
            option,
            crate::cmd::openp2p::SubscriptionUpdateTrigger::Batch,
        )
        .await
        .map_err(openp2p_failure)?;
        return Ok(updated.is_some());
    }
    let url_opt = should_update_profile(uid, is_mannual_trigger).await?;
    match url_opt {
        Some((url, opt)) => {
            perform_profile_update(uid, &url, opt.as_ref(), option, is_mannual_trigger).await?;
            Ok(true)
        }
        None => Ok(false),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ProfileUpdatePolicy {
    /// Automatic bundle-member updates only fetch/persist and leave the bundle
    /// regeneration to the coalescing materializer.
    defer_bundle_materialize: bool,
    mark_bundle_dirty: bool,
    refresh_runtime: bool,
}

const fn profile_update_policy(
    is_manual: bool,
    bundle_member: bool,
    profile_persisted: bool,
    is_current: bool,
    bundle_active: bool,
) -> ProfileUpdatePolicy {
    // Manual updates keep the immediate/synchronous behavior; only automatic
    // (timer) member updates are coalesced.
    let defer_bundle_materialize = !is_manual && bundle_member;
    ProfileUpdatePolicy {
        defer_bundle_materialize,
        mark_bundle_dirty: defer_bundle_materialize && profile_persisted,
        refresh_runtime: is_current
            || (!defer_bundle_materialize && bundle_member && bundle_active)
            || (!profile_persisted && is_manual),
    }
}

/// Shared runtime apply after a bundle member update: announce, mark the active
/// bundle validated, and restore artifacts on failure.
async fn apply_runtime_after_bundle_update(
    profile_persisted: bool,
    bundle_active: bool,
    bundle_artifacts: Option<&bundle::BundleArtifacts>,
    is_mannual_trigger: bool,
) -> Result<()> {
    logging!(debug, Type::Config, "[订阅更新] 更新内核配置");
    match CoreManager::global().update_config_with_force(is_mannual_trigger).await {
        Ok(outcome) if outcome.is_valid() => {
            announce(Refresh::Clash);
            if bundle_active && let Err(error) = bundle::mark_validated_if_current().await {
                logging!(
                    warn,
                    Type::Config,
                    "[订阅更新] failed to mark the applied bundle validated: {error:#}"
                );
            }
        }
        Ok(outcome @ (ValidationOutcome::Skipped { .. } | ValidationOutcome::Busy)) if !is_mannual_trigger => {
            logging!(info, Type::Config, "[订阅更新] 本次配置刷新已跳过: {}", outcome);
        }
        result => {
            if let Some(artifacts) = bundle_artifacts
                && let Err(error) = bundle::restore_artifacts(artifacts).await
            {
                logging!(
                    warn,
                    Type::Config,
                    "[订阅更新] failed to restore bundle artifacts: {error:#}"
                );
            }
            let message = match result {
                Ok(outcome) => outcome.to_string(),
                Err(err) => err.to_string(),
            };
            let message = mask_err(&message);
            let message = if profile_persisted {
                format!("订阅已保存，但未能确认已应用到内核: {message}")
            } else {
                message
            };
            logging!(error, Type::Config, "[订阅更新] 更新失败: {}", message);
            handle::Handle::notice(NoticeStatus::UpdateFailed, message.as_str());
            bail!(message);
        }
    }
    Ok(())
}

/// A subscription card that is not a bundle member. It mirrors the ordinary
/// current-profile runtime apply, but also converges the sidecar when its own
/// source is the active one.
async fn update_direct_subscription(uid: &String, option: Option<&PrfOption>, is_mannual_trigger: bool) -> Result<()> {
    let trigger = if is_mannual_trigger {
        crate::cmd::openp2p::SubscriptionUpdateTrigger::Manual
    } else {
        crate::cmd::openp2p::SubscriptionUpdateTrigger::Automatic
    };
    let Some(_) = crate::cmd::openp2p::update_subscription(uid, option, trigger)
        .await
        .map_err(openp2p_failure)?
    else {
        return Ok(());
    };
    let is_current = Config::profiles().await.latest_arc().current.as_ref() == Some(uid);
    if is_current {
        crate::cmd::openp2p::reconcile_active_sidecar().await;
        apply_runtime_after_bundle_update(true, false, None, is_mannual_trigger).await?;
    }
    Ok(())
}

/// The effects of a bundle-member update after its policy was chosen.
async fn apply_member_update_policy(
    openp2p: bool,
    profile_persisted: bool,
    bundle_active: bool,
    is_current: bool,
    policy: ProfileUpdatePolicy,
    bundle_artifacts: Option<&bundle::BundleArtifacts>,
    is_mannual_trigger: bool,
) -> Result<()> {
    if policy.mark_bundle_dirty {
        // Automatic member update: one coalesced regeneration/apply later.
        crate::cmd::bundle::mark_bundle_dirty();
    }

    if openp2p && profile_persisted && (is_current || bundle_active) {
        // Sidecar accounts run from the reimported config; the bundle/runtime
        // policy above remains the only owner of the mihomo apply.
        crate::cmd::openp2p::reconcile_active_sidecar().await;
    }

    // A skipped OpenP2P update (local source or auto-update off) changed nothing,
    // so it must not trigger a runtime rebuild either.
    if policy.refresh_runtime && !(openp2p && !profile_persisted) {
        apply_runtime_after_bundle_update(profile_persisted, bundle_active, bundle_artifacts, is_mannual_trigger)
            .await?;
    }
    Ok(())
}

/// Bundle-aware wrapper used by the manual update command and the timer.
///
/// Non-members delegate to the unchanged upstream `update_profile`, preserving
/// upstream/manual/timer behavior. Bundle members keep the exact fork behavior:
/// manual = fetch + immediate bundle refresh/apply; automatic = fetch/persist +
/// coalesced dirty mark; inactive bundle never applies the bundle runtime, while
/// a current member still applies normally.
pub async fn update_profile(uid: &String, option: Option<&PrfOption>, is_mannual_trigger: bool) -> Result<()> {
    let openp2p = has_live_openp2p_subscription(uid)?;
    let bundle_member = match bundle::is_member(uid).await {
        Ok(is_member) => is_member,
        Err(error) => {
            logging!(
                warn,
                Type::Config,
                "[订阅更新] failed to read bundle metadata: {error:#}"
            );
            false
        }
    };
    if !bundle_member {
        if openp2p {
            return update_direct_subscription(uid, option, is_mannual_trigger).await;
        }
        return super::profile::update_profile(uid, option, is_mannual_trigger).await;
    }

    let profile_persisted = if openp2p {
        let trigger = if is_mannual_trigger {
            crate::cmd::openp2p::SubscriptionUpdateTrigger::Manual
        } else {
            crate::cmd::openp2p::SubscriptionUpdateTrigger::Automatic
        };
        crate::cmd::openp2p::update_subscription(uid, option, trigger)
            .await
            .map_err(openp2p_failure)?
            .is_some()
    } else {
        fetch_and_persist_profile(uid, option, is_mannual_trigger).await?
    };
    let profiles = Config::profiles().await;
    let manual_generation = is_mannual_trigger.then(crate::cmd::bundle::begin_manual_materialize);
    let mut bundle_artifacts = None;
    let mut bundle_refresh_ok = false;
    let (bundle_active, is_current, policy) = {
        let snapshot = profiles.latest_arc();
        let active = snapshot.current.as_deref() == Some(bundle::BUNDLE_UID);
        let is_current = snapshot.current.as_ref() == Some(uid);
        let policy = profile_update_policy(is_mannual_trigger, bundle_member, profile_persisted, is_current, active);
        if !policy.defer_bundle_materialize {
            let _profile_write = crate::config::profiles::PROFILE_WRITE_LOCK.lock().await;
            bundle_artifacts = Some(bundle::snapshot_artifacts().await?);
            match bundle::refresh(snapshot.as_ref(), true, active).await {
                Ok(_) => bundle_refresh_ok = true,
                Err(error) => {
                    if let Some(artifacts) = bundle_artifacts.as_ref()
                        && let Err(error) = bundle::restore_artifacts(artifacts).await
                    {
                        logging!(
                            warn,
                            Type::Config,
                            "[订阅更新] failed to restore bundle artifacts: {error:#}"
                        );
                    }
                    if active {
                        logging!(
                            error,
                            Type::Config,
                            "[订阅更新] failed to regenerate the profile bundle: {error:#}"
                        );
                        bail!("profile updated but the profile bundle could not be regenerated: {error:#}");
                    }
                    logging!(
                        warn,
                        Type::Config,
                        "[订阅更新] bundle stays stale until it is activated or refreshed: {error:#}"
                    );
                }
            }
        }
        (active, is_current, policy)
    };
    apply_member_update_policy(
        openp2p,
        profile_persisted,
        bundle_active,
        is_current,
        policy,
        bundle_artifacts.as_ref(),
        is_mannual_trigger,
    )
    .await?;

    if let Some(generation) = manual_generation
        && bundle_refresh_ok
    {
        // The successful manual refresh already covered any pending automatic
        // mark when no newer member update arrived while it ran.
        crate::cmd::bundle::end_manual_materialize(generation, true);
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{ProfileUpdatePolicy, UpdateRoute, profile_update_policy, update_route};

    #[test]
    fn openp2p_subscription_uids_route_to_the_openp2p_update() {
        assert_eq!(
            update_route("op2p-sub-7bab4a2044fddf4e"),
            UpdateRoute::OpenP2PSubscription,
            "the generated subscription card must not go through the ordinary remote fetch"
        );
        assert_eq!(update_route("bundle-main"), UpdateRoute::Ordinary);
        assert_eq!(update_route("R1a2b3c"), UpdateRoute::Ordinary);
    }

    #[test]
    fn automatic_bundle_member_updates_defer_to_the_materializer() {
        assert_eq!(
            profile_update_policy(false, true, true, false, true),
            ProfileUpdatePolicy {
                defer_bundle_materialize: true,
                mark_bundle_dirty: true,
                refresh_runtime: false,
            }
        );
        assert_eq!(
            profile_update_policy(false, true, true, false, false),
            ProfileUpdatePolicy {
                defer_bundle_materialize: true,
                mark_bundle_dirty: true,
                refresh_runtime: false,
            }
        );
        assert_eq!(
            profile_update_policy(false, true, true, true, false),
            ProfileUpdatePolicy {
                defer_bundle_materialize: true,
                mark_bundle_dirty: true,
                refresh_runtime: true,
            }
        );
        assert_eq!(
            profile_update_policy(false, true, false, false, true),
            ProfileUpdatePolicy {
                defer_bundle_materialize: true,
                mark_bundle_dirty: false,
                refresh_runtime: false,
            }
        );
    }

    #[test]
    fn manual_updates_and_non_members_keep_immediate_behavior() {
        assert_eq!(
            profile_update_policy(true, true, true, false, true),
            ProfileUpdatePolicy {
                defer_bundle_materialize: false,
                mark_bundle_dirty: false,
                refresh_runtime: true,
            }
        );
        assert_eq!(
            profile_update_policy(true, true, false, false, true),
            ProfileUpdatePolicy {
                defer_bundle_materialize: false,
                mark_bundle_dirty: false,
                refresh_runtime: true,
            }
        );
        assert_eq!(
            profile_update_policy(false, false, true, true, false),
            ProfileUpdatePolicy {
                defer_bundle_materialize: false,
                mark_bundle_dirty: false,
                refresh_runtime: true,
            }
        );
    }
}
