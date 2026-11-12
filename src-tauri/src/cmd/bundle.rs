//! FORK: Profile Bundle commands — one reusable, materialized local profile
//! assembled from an ordered list of subscription profiles.

use super::{CmdResult, WithErrorCode as _, coded_error};
use crate::cmd::validate::{ValidationNoticeTarget, handle_validation_notice};
use crate::config::{Config, PrfItem, bundle, profiles::PROFILE_WRITE_LOCK};
use crate::core::{CoreManager, handle, tray::Tray};
use crate::enhance::multi_merge::{self, ConflictEntry};
use clash_verge_logging::{Type, logging};
use smartstring::alias::String;

mod materialize;

pub(crate) use materialize::{begin_manual_materialize, end_manual_materialize, mark_bundle_dirty};

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

#[derive(serde::Serialize, specta::Type)]
pub struct BundleRefreshFailure {
    pub uid: std::string::String,
    pub message: std::string::String,
}

/// Outcome of a manual full refresh: remote member updates first, then one
/// regeneration from the updated member files.
#[derive(serde::Serialize, specta::Type)]
pub struct BundleRefreshReport {
    pub updated: Vec<std::string::String>,
    pub failed: Vec<BundleRefreshFailure>,
    pub regenerated: bool,
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

/// Remote subscriptions that belong to the bundle, in bundle order.
fn remote_bundle_members(meta: &bundle::BundleMeta, items: &[PrfItem]) -> Vec<String> {
    meta.members
        .iter()
        .filter(|uid| {
            items
                .iter()
                .find(|item| item.uid.as_deref() == Some(uid.as_str()))
                .is_some_and(|item| item.itype.as_deref() == Some("remote"))
        })
        .cloned()
        .collect()
}

fn build_bundle_refresh_report(
    updated: Vec<String>,
    failed: Vec<BundleRefreshFailure>,
    regenerated: bool,
) -> BundleRefreshReport {
    BundleRefreshReport {
        updated: updated.into_iter().map(Into::into).collect(),
        failed,
        regenerated,
    }
}

/// FORK: full manual bundle refresh — update every remote member subscription
/// sequentially (manual semantics, regardless of auto-update settings), continue
/// past per-member failures, then regenerate the bundle from the updated files
/// and apply it when the bundle is the active profile.
///
/// The profile write lock is not held across the network/update calls; it is
/// acquired only for the final regeneration/apply.
#[tauri::command]
#[specta::specta]
pub async fn refresh_bundle() -> CmdResult<BundleRefreshReport> {
    let remote_members = {
        let profiles = Config::profiles().await;
        let snapshot = profiles.latest_arc();
        if snapshot.get_item(bundle::BUNDLE_UID).is_err() {
            return Err(coded_error("BUNDLE_NOT_FOUND", "no profile bundle exists"));
        }
        let Some(meta) = bundle::load_meta().await.with_error_code("BUNDLE_META_INVALID")? else {
            return Err(coded_error("BUNDLE_NOT_FOUND", "no profile bundle exists"));
        };
        if !bundle::metadata_trusted(&meta) {
            return Err(coded_error(
                "BUNDLE_META_INVALID",
                "bundle metadata member/order fingerprint mismatch; recreate the bundle",
            ));
        }
        remote_bundle_members(&meta, snapshot.items.as_deref().unwrap_or_default())
    };

    refresh_bundle_flow(
        remote_members,
        |uid| async move {
            crate::feat::bundle_update::fetch_and_persist_profile(&uid, None, true)
                .await
                .map_err(|error| error.to_string())
                .map(|_persisted| ())
        },
        finalize_manual_bundle_refresh,
    )
    .await
}

/// Orchestration seam: fetch/persist every remote member sequentially, then call
/// `finalize` exactly once. No bundle regeneration/apply happens per member.
async fn refresh_bundle_flow<Update, UpdateFut, Finalize, FinalizeFut>(
    remote_members: Vec<String>,
    mut update: Update,
    finalize: Finalize,
) -> CmdResult<BundleRefreshReport>
where
    Update: FnMut(String) -> UpdateFut + Send,
    UpdateFut: std::future::Future<Output = std::result::Result<(), std::string::String>> + Send,
    Finalize: FnOnce() -> FinalizeFut + Send,
    FinalizeFut: std::future::Future<Output = CmdResult<bool>> + Send,
{
    let had_remote = !remote_members.is_empty();
    let mut updated: Vec<String> = Vec::with_capacity(remote_members.len());
    let mut failed: Vec<BundleRefreshFailure> = Vec::new();
    for uid in remote_members {
        match update(uid.clone()).await {
            Ok(()) => updated.push(uid),
            Err(message) => {
                logging!(warn, Type::Cmd, "failed to update bundle member '{uid}': {message}");
                failed.push(BundleRefreshFailure {
                    uid: uid.to_string(),
                    message,
                });
            }
        }
    }

    if had_remote && updated.is_empty() {
        // Nothing changed on disk; regeneration/apply would be a no-op.
        logging!(
            warn,
            Type::Cmd,
            "all {} remote bundle member update(s) failed; skipping regeneration",
            failed.len()
        );
        return Ok(build_bundle_refresh_report(updated, failed, false));
    }

    let regenerated = match finalize().await {
        Ok(regenerated) => regenerated,
        Err(error) => {
            logging!(
                warn,
                Type::Cmd,
                "bundle refresh finalization failed after updating {} member(s) ({} failed): {error}",
                updated.len(),
                failed.len()
            );
            return Err(error);
        }
    };
    Ok(build_bundle_refresh_report(updated, failed, regenerated))
}

/// Manual finalizer: a successful manual regeneration covers pending automatic
/// dirty marks that arrived before it started.
async fn finalize_manual_bundle_refresh() -> CmdResult<bool> {
    let generation = begin_manual_materialize();
    let result = finalize_bundle_refresh().await;
    if result.is_ok() {
        end_manual_materialize(generation, true);
    }
    result
}

/// Lock once, regenerate from the updated member files, and apply once when the
/// bundle is current. Restores artifacts on regeneration/apply failure.
async fn finalize_bundle_refresh() -> CmdResult<bool> {
    let _profile_write_guard = PROFILE_WRITE_LOCK.lock().await;
    let profiles = Config::profiles().await;
    let snapshot = profiles.latest_arc();
    if snapshot.get_item(bundle::BUNDLE_UID).is_err() {
        return Err(coded_error("BUNDLE_NOT_FOUND", "no profile bundle exists"));
    }

    let artifacts = bundle::snapshot_artifacts()
        .await
        .with_error_code("BUNDLE_REFRESH_FAILED")?;
    let Some(outcome) = (match bundle::refresh(&snapshot, true, true).await {
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
    Ok(outcome.regenerated)
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn item(uid: &str, itype: &str) -> PrfItem {
        PrfItem {
            uid: Some(uid.into()),
            itype: Some(itype.into()),
            file: Some(format!("{uid}.yaml").into()),
            ..PrfItem::default()
        }
    }

    fn meta(members: &[&str]) -> bundle::BundleMeta {
        bundle::BundleMeta {
            version: bundle::META_VERSION,
            members: members.iter().map(|uid| (*uid).into()).collect(),
            sources: Default::default(),
            members_sha256: bundle::members_fingerprint(
                bundle::META_VERSION,
                &members.iter().map(|uid| (*uid).into()).collect::<Vec<_>>(),
            ),
            generated_sha256: "generated".into(),
            validated: true,
            conflicts: Vec::new(),
            generated_at: 0,
        }
    }

    #[test]
    fn remote_members_are_selected_in_bundle_order() {
        let items = vec![
            item("remote-a", "remote"),
            item("local-b", "local"),
            item("remote-c", "remote"),
            item("outside", "remote"),
        ];
        let meta = meta(&["local-b", "remote-c", "missing", "remote-a"]);
        assert_eq!(
            remote_bundle_members(&meta, &items),
            vec![String::from("remote-c"), String::from("remote-a")]
        );
    }

    #[test]
    fn refresh_report_preserves_update_and_failure_lists() {
        let report = build_bundle_refresh_report(
            vec![String::from("remote-a"), String::from("remote-b")],
            vec![BundleRefreshFailure {
                uid: std::string::String::from("remote-c"),
                message: std::string::String::from("timeout"),
            }],
            true,
        );
        assert_eq!(
            report.updated,
            vec![
                std::string::String::from("remote-a"),
                std::string::String::from("remote-b")
            ]
        );
        assert_eq!(report.failed.len(), 1);
        assert_eq!(report.failed[0].uid, "remote-c");
        assert_eq!(report.failed[0].message, "timeout");
        assert!(report.regenerated);
    }

    #[tokio::test]
    async fn full_refresh_updates_each_member_and_finalizes_exactly_once() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };

        let updates = Arc::new(AtomicUsize::new(0));
        let finalizes = Arc::new(AtomicUsize::new(0));
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let update_counter = Arc::clone(&updates);
        let seen_uids = Arc::clone(&seen);
        let finalize_counter = Arc::clone(&finalizes);

        let report = refresh_bundle_flow(
            vec![
                String::from("remote-a"),
                String::from("remote-b"),
                String::from("remote-c"),
            ],
            move |uid: String| {
                let counter = Arc::clone(&update_counter);
                let seen = Arc::clone(&seen_uids);
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    seen.lock().unwrap().push(uid.to_string());
                    if uid.as_str() == "remote-b" {
                        Err(std::string::String::from("timeout"))
                    } else {
                        Ok(())
                    }
                }
            },
            move || async move {
                finalize_counter.fetch_add(1, Ordering::SeqCst);
                Ok(true)
            },
        )
        .await
        .unwrap();

        assert_eq!(updates.load(Ordering::SeqCst), 3);
        assert_eq!(
            finalizes.load(Ordering::SeqCst),
            1,
            "bundle regeneration/apply must happen exactly once"
        );
        assert_eq!(
            seen.lock().unwrap().clone(),
            vec![
                std::string::String::from("remote-a"),
                std::string::String::from("remote-b"),
                std::string::String::from("remote-c")
            ]
        );
        assert_eq!(
            report.updated,
            vec![
                std::string::String::from("remote-a"),
                std::string::String::from("remote-c")
            ]
        );
        assert_eq!(report.failed[0].uid, "remote-b");
        assert!(report.regenerated);
    }

    #[tokio::test]
    async fn all_remote_failures_skip_finalize_but_keep_the_report() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };

        let finalizes = Arc::new(AtomicUsize::new(0));
        let finalize_counter = Arc::clone(&finalizes);
        let report = refresh_bundle_flow(
            vec![String::from("remote-a"), String::from("remote-b")],
            |_uid: String| async move { Err(std::string::String::from("offline")) },
            move || async move {
                finalize_counter.fetch_add(1, Ordering::SeqCst);
                Ok(false)
            },
        )
        .await
        .unwrap();

        assert_eq!(
            finalizes.load(Ordering::SeqCst),
            0,
            "all remote members failed: no files changed, so no finalize/apply"
        );
        assert!(report.updated.is_empty());
        assert_eq!(report.failed.len(), 2);
        assert!(!report.regenerated);
    }

    #[tokio::test]
    async fn local_only_refresh_still_finalizes_once() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };

        let finalizes = Arc::new(AtomicUsize::new(0));
        let finalize_counter = Arc::clone(&finalizes);
        let report = refresh_bundle_flow(
            Vec::new(),
            |_uid: String| async move { Ok(()) },
            move || async move {
                finalize_counter.fetch_add(1, Ordering::SeqCst);
                Ok(true)
            },
        )
        .await
        .unwrap();

        assert_eq!(
            finalizes.load(Ordering::SeqCst),
            1,
            "a bundle without remote members still regenerates local files"
        );
        assert!(report.updated.is_empty());
        assert!(report.failed.is_empty());
        assert!(report.regenerated);
    }
}
