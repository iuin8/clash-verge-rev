use crate::core::notify::NoticeStatus;
use crate::core::notify::{Refresh, announce};
use crate::{
    cmd,
    config::{Config, PrfItem, PrfOption, profiles::profiles_update_item_safe},
    core::{CoreManager, handle, tray, validate::ValidationOutcome},
    utils::help::{mask_err, mask_url},
};
use anyhow::{Result, bail};
use clash_verge_logging::{Type, logging};
use smartstring::alias::String;

/// Toggle proxy profile
pub async fn toggle_proxy_profile(profile_index: String) {
    if let Err(err) = cmd::patch_profiles_config_by_profile_index(profile_index.clone()).await {
        logging!(
            error,
            Type::Config,
            "toggle proxy profile {profile_index} failed: {err}"
        );
    }
}

/// Tell the profile which node this group is on now.
///
/// The core is already switched by the time this runs, so failing to record is not a reason to
/// report the switch as failed — but it is worth a log line, because what the profile holds is
/// what gets re-applied the next time a core starts.
async fn record_switched_node(group_name: &str, proxy_name: &str) {
    if let Err(error) = crate::config::profiles::record_selected_node(group_name, proxy_name).await {
        logging!(
            warn,
            Type::Config,
            "切换代理成功但未能记录到配置: {} -> {}, 错误: {error:#}",
            group_name,
            proxy_name
        );
    }
}

pub async fn switch_proxy_node(group_name: &str, proxy_name: &str) {
    match handle::Handle::mihomo()
        .select_node_for_group(group_name, proxy_name)
        .await
    {
        Ok(_) => {
            record_switched_node(group_name, proxy_name).await;
            announce(Refresh::Proxies);
            let _ = tray::Tray::global().update_menu().await;
            return;
        }
        Err(err) => {
            logging!(error, Type::Tray, "切换代理失败: {err:?}");
        }
    }

    match handle::Handle::mihomo()
        .select_node_for_group(group_name, proxy_name)
        .await
    {
        Ok(_) => {
            logging!(info, Type::Tray, "代理切换回退成功: {} -> {}", group_name, proxy_name);
            record_switched_node(group_name, proxy_name).await;
            let _ = tray::Tray::global().update_menu().await;
        }
        Err(err) => {
            logging!(error, Type::Tray, "代理切换最终失败: {err:?}");
        }
    }
}

async fn should_update_profile(uid: &String, ignore_auto_update: bool) -> Result<Option<(String, Option<PrfOption>)>> {
    let profiles = Config::profiles().await;
    let profiles = profiles.latest_arc();
    let item = profiles.get_item(uid)?;
    let is_remote = item.itype.as_ref().is_some_and(|s| s == "remote");

    if !is_remote {
        logging!(info, Type::Config, "[订阅更新] 不是远程订阅，跳过更新");
        Ok(None)
    } else if item.url.is_none() {
        logging!(warn, Type::Config, "[订阅更新] 缺少URL，无法更新");
        bail!("failed to get the profile item url");
    } else if !ignore_auto_update && !item.option.as_ref().and_then(|o| o.allow_auto_update).unwrap_or(true) {
        logging!(info, Type::Config, "[订阅更新] 禁止自动更新，跳过更新");
        Ok(None)
    } else {
        logging!(
            info,
            Type::Config,
            "[订阅更新] 远程订阅，URL: {}",
            mask_url(
                item.url
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Profile URL is None"))?
            )
        );
        Ok(Some((
            item.url.clone().ok_or_else(|| anyhow::anyhow!("Profile URL is None"))?,
            item.option.clone(),
        )))
    }
}

#[tracing::instrument(skip_all, level = "info", fields(uid = %uid, strategy = tracing::field::Empty, configured_elapsed = tracing::field::Empty, clash_elapsed = tracing::field::Empty, system_elapsed = tracing::field::Empty))]
async fn perform_profile_update(
    uid: &String,
    url: &String,
    opt: Option<&PrfOption>,
    option: Option<&PrfOption>,
    is_mannual_trigger: bool,
) -> Result<()> {
    let merged_opt = PrfOption::merge(opt, option);
    let profiles = Config::profiles().await;
    let profiles_arc = profiles.latest_arc();
    let profile_name = profiles_arc
        .get_item(uid)
        .ok()
        .and_then(|item| item.name.clone())
        .unwrap_or_else(|| String::from("UnKnown Profile"));

    #[derive(Clone, Copy)]
    enum Strategy {
        Configured,
        Clash,
        System,
    }

    let result =
        crate::utils::retry::try_strategies(Strategy::Configured, [Strategy::Clash, Strategy::System], |strategy| {
            let mut options = merged_opt.clone();
            async move {
                let (name, success, failure) = match strategy {
                    Strategy::Configured => (
                        "configured",
                        "[订阅更新] 更新订阅配置成功",
                        "[订阅更新] 直接更新失败: {}，尝试使用Clash代理更新",
                    ),
                    Strategy::Clash => (
                        "clash",
                        "[订阅更新] 使用 Clash代理 更新订阅配置成功",
                        "[订阅更新] Clash代理更新失败: {}，尝试使用系统代理更新",
                    ),
                    Strategy::System => (
                        "system",
                        "[订阅更新] 使用 系统代理 更新订阅配置成功",
                        "[订阅更新] 系统代理更新失败: {}，所有重试均已失败",
                    ),
                };
                match strategy {
                    Strategy::Configured => {}
                    Strategy::Clash | Strategy::System => {
                        let option = options.get_or_insert_with(PrfOption::default);
                        option.self_proxy = Some(matches!(strategy, Strategy::Clash));
                        option.with_proxy = Some(matches!(strategy, Strategy::System));
                    }
                }
                tracing::Span::current().record("strategy", name);
                let started = std::time::Instant::now();
                let result = PrfItem::from_url(url, None, None, options.as_ref()).await;
                let elapsed_field = match strategy {
                    Strategy::Configured => "configured_elapsed",
                    Strategy::Clash => "clash_elapsed",
                    Strategy::System => "system_elapsed",
                };
                tracing::Span::current().record(elapsed_field, tracing::field::debug(started.elapsed()));
                match &result {
                    Ok(_) => logging!(info, Type::Config, "{success}"),
                    Err(error) => logging!(
                        warn,
                        Type::Config,
                        "{}",
                        failure.replace("{}", &mask_err(&format!("{error:#}")))
                    ),
                }
                result
            }
        })
        .await;
    let last_err = match result {
        Ok((strategy, mut item)) => {
            // A persistence error must not replay a successful download with another transport.
            profiles_update_item_safe(uid, &mut item).await?;
            if !matches!(strategy, Strategy::Configured) {
                handle::Handle::notice(NoticeStatus::UpdateWithClashProxy, profile_name.as_str());
            }
            return Ok(());
        }
        Err(error) => error,
    };

    let last_err = mask_err(&last_err.to_string());
    if is_mannual_trigger {
        handle::Handle::notice(
            NoticeStatus::UpdateFailedEvenWithClash,
            format!("{profile_name} - {last_err}"),
        );
    }
    bail!(last_err)
}

/// Fetch and persist a remote profile **without** regenerating or applying the
/// bundle/runtime. Used by batch flows (Profile Bundle refresh) that must apply
/// exactly once after all members are updated.
///
/// Returns whether new profile content was persisted; `false` means the update was
/// intentionally skipped (e.g. auto-update disabled, non-remote, exiting).
pub async fn fetch_and_persist_profile(
    uid: &String,
    option: Option<&PrfOption>,
    is_mannual_trigger: bool,
) -> Result<bool> {
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
    /// regeneration to the coalescing worker.
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

#[tracing::instrument(skip_all, level = "info", fields(uid = %uid, manual = is_mannual_trigger))]
pub async fn update_profile(uid: &String, option: Option<&PrfOption>, is_mannual_trigger: bool) -> Result<()> {
    let profile_persisted = fetch_and_persist_profile(uid, option, is_mannual_trigger).await?;
    let profiles = Config::profiles().await;
    let bundle_member = match crate::config::bundle::is_member(uid).await {
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
    let mut bundle_artifacts = None;
    let (bundle_active, policy) = {
        let snapshot = profiles.latest_arc();
        let active = snapshot.current.as_deref() == Some(crate::config::bundle::BUNDLE_UID);
        let is_current = snapshot.current.as_ref() == Some(uid);
        let policy = profile_update_policy(is_mannual_trigger, bundle_member, profile_persisted, is_current, active);
        if bundle_member && !policy.defer_bundle_materialize {
            let _profile_write = crate::config::profiles::PROFILE_WRITE_LOCK.lock().await;
            bundle_artifacts = Some(crate::config::bundle::snapshot_artifacts().await?);
            if let Err(error) = crate::config::bundle::refresh(snapshot.as_ref(), true, active).await {
                if let Some(artifacts) = bundle_artifacts.as_ref()
                    && let Err(error) = crate::config::bundle::restore_artifacts(artifacts).await
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
        (active, policy)
    };
    if policy.mark_bundle_dirty {
        // Automatic member updates: one coalesced regeneration/apply later.
        crate::cmd::bundle::mark_bundle_dirty();
    }
    let should_refresh = policy.refresh_runtime;

    if should_refresh {
        logging!(debug, Type::Config, "[订阅更新] 更新内核配置");
        match CoreManager::global().update_config_with_force(is_mannual_trigger).await {
            Ok(outcome) if outcome.is_valid() => {
                announce(Refresh::Clash);
                if bundle_active
                    && bundle_member
                    && let Err(error) = crate::config::bundle::mark_validated_if_current().await
                {
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
                if let Some(artifacts) = bundle_artifacts.as_ref()
                    && let Err(error) = crate::config::bundle::restore_artifacts(artifacts).await
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
    }

    Ok(())
}

/// 增强配置
pub async fn enhance_profiles() -> Result<ValidationOutcome> {
    let outcome = CoreManager::global().update_config_forced().await?;
    if outcome.is_valid()
        && let Err(error) = crate::config::bundle::mark_validated_if_current().await
    {
        logging!(
            warn,
            Type::Config,
            "[增强配置] failed to mark the active bundle validated: {error:#}"
        );
    }
    Ok(outcome)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod update_policy_tests {
    use super::{ProfileUpdatePolicy, profile_update_policy};

    #[test]
    fn automatic_bundle_member_updates_defer_to_the_materializer() {
        // Active bundle: member update is persisted, dirty-marked, no synchronous
        // runtime refresh.
        assert_eq!(
            profile_update_policy(false, true, true, false, true),
            ProfileUpdatePolicy {
                defer_bundle_materialize: true,
                mark_bundle_dirty: true,
                refresh_runtime: false,
            }
        );
        // Inactive bundle, updated member is not current: same coalescing.
        assert_eq!(
            profile_update_policy(false, true, true, false, false),
            ProfileUpdatePolicy {
                defer_bundle_materialize: true,
                mark_bundle_dirty: true,
                refresh_runtime: false,
            }
        );
        // The updated member is the current profile: keep applying the member
        // runtime normally, while the inactive bundle materializes separately.
        assert_eq!(
            profile_update_policy(false, true, true, true, false),
            ProfileUpdatePolicy {
                defer_bundle_materialize: true,
                mark_bundle_dirty: true,
                refresh_runtime: true,
            }
        );
        // No new bytes persisted: nothing to materialize.
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
        // Manual fallback when nothing was persisted (e.g. forced refresh).
        assert_eq!(
            profile_update_policy(true, true, false, false, true),
            ProfileUpdatePolicy {
                defer_bundle_materialize: false,
                mark_bundle_dirty: false,
                refresh_runtime: true,
            }
        );
        // Automatic update of the current non-member still applies immediately.
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
