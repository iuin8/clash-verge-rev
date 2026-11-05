//! Isolated acceptance for the multi-profile merge lifecycle.
//!
//! The commands under test need a Tauri app handle, a running core, and the
//! user's data directory, so this module drives the same deterministic
//! transitions over the real [`Draft<IProfiles>`] layer, an in-memory
//! `merged.json` stand-in, and the production decision helpers.
//!
//! Reachable and exercised here: the copy-on-write draft transaction each
//! command uses (`with_data_modify` commits the candidate on success and the
//! untouched original on failure), `IProfiles::plan_delete_item`, member
//! validation, and the apply/clear/switch/delete/rollback decisions.
//!
//! Not reachable without an app handle or core process: `Config::profiles()`
//! (process-global `OnceCell`), `CoreManager::update_config_forced_with_profiles`
//! (needs the core validator/apply path), `handle::Handle::refresh_clash`, and
//! the tray/timer notifications. Those remain covered only by review.
//!
//! Run with `cargo test --lib merge_acceptance`.

#![allow(clippy::unwrap_used)]

use super::merge::{
    clears_merge_for_switch, plan_delete_merge, previous_merged_for_update, should_restore_merge_after_delete_failure,
    should_restore_merge_after_switch, validate_merged_members,
};
use super::{CmdResult, coded_error};
use crate::config::{IProfiles, PrfItem};
use anyhow::Result as AnyResult;
use clash_verge_draft::Draft;
use parking_lot::Mutex;
use smartstring::alias::String;
use std::sync::Arc;

#[derive(Debug, PartialEq, Eq)]
struct DeleteOutcome {
    needs_runtime_update: bool,
}

#[derive(Default)]
struct StoreState {
    uids: Vec<String>,
    corrupt: bool,
}

/// Mirrors the command state: the profiles draft and the merged.json store.
struct MergeLifecycle {
    profiles: Draft<IProfiles>,
    store: Arc<Mutex<StoreState>>,
}

impl MergeLifecycle {
    fn new(current: &str, items: Vec<PrfItem>) -> Self {
        Self {
            profiles: Draft::new(IProfiles {
                current: Some(current.into()),
                items: Some(items),
            }),
            store: Arc::new(Mutex::new(StoreState::default())),
        }
    }

    fn load(&self) -> AnyResult<Vec<String>> {
        let store = self.store.lock();
        if store.corrupt {
            Err(anyhow::anyhow!("invalid merged.json"))
        } else {
            Ok(store.uids.clone())
        }
    }

    fn save(&self, uids: Vec<String>) {
        let mut store = self.store.lock();
        store.uids = uids;
        store.corrupt = false;
    }

    fn merged(&self) -> Vec<String> {
        self.store.lock().uids.clone()
    }

    fn set_corrupt(&self) {
        self.store.lock().corrupt = true;
    }

    fn is_corrupt(&self) -> bool {
        self.store.lock().corrupt
    }

    /// Mirrors `set_merged_profiles`: validate, select the first member as
    /// primary, persist, then commit the runtime update or roll back.
    async fn apply(&self, uids: &[&str], runtime_ok: bool) -> CmdResult {
        let next: Vec<String> = uids.iter().map(|uid| (*uid).into()).collect();
        let previous = previous_merged_for_update(self.load(), false)
            .map_err(|error| coded_error("MERGED_PROFILE_LOAD_FAILED", error))?;
        validate_merged_members(self.profiles.latest_arc().as_ref(), &next)?;

        let primary = next.first().cloned();
        let store = Arc::clone(&self.store);
        let result = self
            .profiles
            .with_data_modify(move |mut candidate| async move {
                let original = candidate.clone();
                if let Some(primary) = primary {
                    candidate.patch_config(&IProfiles {
                        current: Some(primary),
                        items: None,
                    });
                }
                {
                    let mut store = store.lock();
                    store.uids = next;
                    store.corrupt = false;
                }
                if runtime_ok {
                    Ok((candidate, Ok(())))
                } else {
                    Ok((
                        original,
                        Err(coded_error("MERGED_PROFILE_FAILED", "runtime config rejected")),
                    ))
                }
            })
            .await;

        match result {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => {
                self.save(previous);
                Err(error)
            }
            Err(error) => {
                self.save(previous);
                Err(coded_error("MERGED_PROFILE_FAILED", error))
            }
        }
    }

    /// Mirrors `clear_merged_profiles`: an explicit clear recovers a corrupt
    /// store; a failed runtime update restores whatever was there before.
    async fn clear(&self, runtime_ok: bool) -> CmdResult {
        let previous = previous_merged_for_update(self.load(), true)
            .map_err(|error| coded_error("MERGED_PROFILE_LOAD_FAILED", error))?;
        validate_merged_members(self.profiles.latest_arc().as_ref(), &[])?;

        let store = Arc::clone(&self.store);
        let result = self
            .profiles
            .with_data_modify(|candidate| async move {
                let original = candidate.clone();
                {
                    let mut store = store.lock();
                    store.uids.clear();
                    store.corrupt = false;
                }
                if runtime_ok {
                    Ok((candidate, Ok(())))
                } else {
                    Ok((
                        original,
                        Err(coded_error("MERGED_PROFILE_FAILED", "runtime config rejected")),
                    ))
                }
            })
            .await;

        match result {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => {
                self.save(previous);
                Err(error)
            }
            Err(error) => {
                self.save(previous);
                Err(coded_error("MERGED_PROFILE_FAILED", error))
            }
        }
    }

    /// Mirrors `patch_profiles_config`: switching to a member keeps the merge;
    /// switching outside it clears the merge and restores it on failure.
    fn switch(&self, target: &str, runtime_ok: bool) -> CmdResult<bool> {
        let previous_profile = self.profiles.data_arc().current.clone();
        let previous_merged = self
            .load()
            .map_err(|error| coded_error("PROFILE_SWITCH_FAILED", error))?;
        let target: String = target.into();
        let clears_merge = clears_merge_for_switch(Some(&target), &previous_merged);
        if clears_merge {
            self.save(Vec::new());
        }

        self.profiles.edit_draft(|draft| {
            draft.patch_config(&IProfiles {
                current: Some(target),
                items: None,
            });
        });
        if !runtime_ok {
            // The command discards the draft and restores the previous current
            // profile when the runtime update is rejected.
            self.profiles.discard();
            if let Some(previous) = previous_profile {
                self.profiles.edit_draft(|draft| draft.current = Some(previous));
            }
        }
        self.profiles.apply();
        if should_restore_merge_after_switch(clears_merge, runtime_ok) {
            self.save(previous_merged);
        }
        Ok(runtime_ok)
    }

    /// Mirrors `delete_profile`: prune the merged list, repair `current` to a
    /// remaining member, and restore the list if the runtime update fails
    /// before the deletion commits.
    async fn delete(&self, uid: &str, runtime_ok: bool) -> CmdResult<DeleteOutcome> {
        let previous_merged = self
            .load()
            .map_err(|error| coded_error("MERGED_PROFILE_LOAD_FAILED", error))?;
        let fallback_merged = previous_merged.clone();
        let store = Arc::clone(&self.store);

        let result = self
            .profiles
            .with_data_modify(move |mut candidate| async move {
                let original = candidate.clone();
                let Ok((should_update, _files)) = candidate.plan_delete_item(uid) else {
                    return Ok((
                        original,
                        Err(coded_error("PROFILE_DELETE_FAILED", "cannot delete the profile")),
                    ));
                };

                let merge_plan = plan_delete_merge(candidate.current.as_ref(), uid, &previous_merged, should_update);
                {
                    let mut store = store.lock();
                    store.uids = merge_plan.remaining.clone();
                    store.corrupt = false;
                }
                if let Some(primary) = merge_plan.repaired_current.clone() {
                    candidate.patch_config(&IProfiles {
                        current: Some(primary),
                        items: None,
                    });
                }

                if merge_plan.needs_runtime_update && !runtime_ok {
                    // The deletion has not committed yet, so the new merged
                    // state is not authoritative and the old list must return.
                    return Ok((
                        original,
                        Err(coded_error("PROFILE_DELETE_FAILED", "runtime config rejected")),
                    ));
                }
                Ok((
                    candidate,
                    Ok(DeleteOutcome {
                        needs_runtime_update: merge_plan.needs_runtime_update,
                    }),
                ))
            })
            .await;

        match result {
            Ok(Ok(outcome)) => Ok(outcome),
            Ok(Err(error)) => {
                if should_restore_merge_after_delete_failure(false) {
                    self.save(fallback_merged);
                }
                Err(error)
            }
            Err(error) => {
                if should_restore_merge_after_delete_failure(false) {
                    self.save(fallback_merged);
                }
                Err(coded_error("PROFILE_DELETE_FAILED", error))
            }
        }
    }
}

fn profile(uid: &str, itype: &str, file: Option<&str>) -> PrfItem {
    PrfItem {
        uid: Some(uid.into()),
        itype: Some(itype.into()),
        file: file.map(Into::into),
        ..PrfItem::default()
    }
}

fn uids(list: &[&str]) -> Vec<String> {
    list.iter().map(|uid| (*uid).into()).collect()
}

fn current(state: &MergeLifecycle) -> Option<std::string::String> {
    state.profiles.data_arc().current.as_ref().map(ToString::to_string)
}

fn has_profile(state: &MergeLifecycle, uid: &str) -> bool {
    state.profiles.data_arc().get_item(uid).is_ok()
}

fn assert_no_pending_draft(state: &MergeLifecycle) {
    assert!(
        Arc::ptr_eq(&state.profiles.latest_arc(), &state.profiles.data_arc()),
        "a committed or rolled-back transition must not leave a draft behind"
    );
}

#[tokio::test]
async fn apply_selects_the_first_member_and_rejects_invalid_members() {
    let state = MergeLifecycle::new(
        "a",
        vec![
            profile("a", "remote", Some("a.yaml")),
            profile("b", "local", Some("b.yaml")),
            profile("chain", "merge", Some("chain.yaml")),
            profile("missing", "remote", None),
        ],
    );

    for rejected in [&["chain"][..], &["missing"][..], &["absent"][..]] {
        let error = state.apply(rejected, true).await.unwrap_err();
        assert_eq!(error.code.as_deref(), Some("MERGED_PROFILE_INVALID"));
    }
    assert!(state.merged().is_empty());
    assert_no_pending_draft(&state);

    state.apply(&["b", "a"], true).await.unwrap();
    assert_eq!(state.merged(), uids(&["b", "a"]));
    assert_eq!(current(&state).as_deref(), Some("b"));
    assert_no_pending_draft(&state);
}

#[tokio::test]
async fn apply_failure_rolls_back_the_draft_and_restores_the_previous_selection() {
    let state = MergeLifecycle::new(
        "a",
        vec![
            profile("a", "remote", Some("a.yaml")),
            profile("b", "remote", Some("b.yaml")),
        ],
    );
    state.apply(&["a", "b"], true).await.unwrap();

    let error = state.apply(&["b"], false).await.unwrap_err();
    assert_eq!(error.code.as_deref(), Some("MERGED_PROFILE_FAILED"));
    assert_eq!(state.merged(), uids(&["a", "b"]));
    assert_eq!(current(&state).as_deref(), Some("a"));
    assert_no_pending_draft(&state);

    state.set_corrupt();
    let error = state.apply(&["a"], true).await.unwrap_err();
    assert_eq!(error.code.as_deref(), Some("MERGED_PROFILE_LOAD_FAILED"));
    assert_eq!(state.merged(), uids(&["a", "b"]));
    assert_eq!(current(&state).as_deref(), Some("a"));
}

#[tokio::test]
async fn clear_drops_the_selection_and_recovers_a_corrupt_store() {
    let state = MergeLifecycle::new(
        "a",
        vec![
            profile("a", "remote", Some("a.yaml")),
            profile("b", "remote", Some("b.yaml")),
        ],
    );
    state.apply(&["a", "b"], true).await.unwrap();

    state.clear(true).await.unwrap();
    assert!(state.merged().is_empty());
    assert_eq!(current(&state).as_deref(), Some("a"));
    assert_no_pending_draft(&state);

    state.set_corrupt();
    assert_eq!(
        state.delete("b", true).await.unwrap_err().code.as_deref(),
        Some("MERGED_PROFILE_LOAD_FAILED")
    );
    assert_eq!(
        state.switch("b", true).unwrap_err().code.as_deref(),
        Some("PROFILE_SWITCH_FAILED")
    );
    // An empty set is not an explicit clear: it still fails closed.
    assert_eq!(
        state.apply(&[], true).await.unwrap_err().code.as_deref(),
        Some("MERGED_PROFILE_LOAD_FAILED")
    );

    state.clear(true).await.unwrap();
    assert!(!state.is_corrupt());
    assert!(state.merged().is_empty());
}

#[tokio::test]
async fn switch_keeps_membership_and_clears_only_outside_the_merge() {
    let state = MergeLifecycle::new(
        "a",
        vec![
            profile("a", "remote", Some("a.yaml")),
            profile("b", "remote", Some("b.yaml")),
            profile("c", "remote", Some("c.yaml")),
        ],
    );

    state.apply(&["a", "b"], true).await.unwrap();
    assert!(state.switch("b", true).unwrap());
    assert_eq!(state.merged(), uids(&["a", "b"]));
    assert_eq!(current(&state).as_deref(), Some("b"));

    assert!(state.switch("c", true).unwrap());
    assert!(state.merged().is_empty());
    assert_eq!(current(&state).as_deref(), Some("c"));

    state.apply(&["a", "b"], true).await.unwrap();
    assert!(!state.switch("c", false).unwrap());
    assert_eq!(state.merged(), uids(&["a", "b"]));
    assert_eq!(current(&state).as_deref(), Some("a"));
    assert_no_pending_draft(&state);

    state.apply(&["a", "b"], true).await.unwrap();
    assert!(!state.switch("b", false).unwrap());
    assert_eq!(state.merged(), uids(&["a", "b"]));
    assert_eq!(current(&state).as_deref(), Some("a"));
    assert_no_pending_draft(&state);
}

#[tokio::test]
async fn deleting_the_current_merged_member_repairs_current_to_a_remaining_member() {
    let state = MergeLifecycle::new(
        "a",
        vec![
            profile("a", "remote", Some("a.yaml")),
            // A subscription that is not part of the merge sits first after
            // the deleted primary; current must not fall onto it.
            profile("other", "remote", Some("other.yaml")),
            profile("b", "remote", Some("b.yaml")),
        ],
    );
    state.apply(&["a", "b"], true).await.unwrap();

    let outcome = state.delete("a", true).await.unwrap();
    assert!(outcome.needs_runtime_update);
    assert_eq!(state.merged(), uids(&["b"]));
    assert_eq!(current(&state).as_deref(), Some("b"));
    assert!(!has_profile(&state, "a"));
    assert_no_pending_draft(&state);

    let outcome = state.delete("b", true).await.unwrap();
    assert!(outcome.needs_runtime_update);
    assert!(state.merged().is_empty());
    assert_eq!(current(&state).as_deref(), Some("other"));
}

#[tokio::test]
async fn deleting_a_non_current_member_prunes_the_list_and_still_refreshes_runtime() {
    let state = MergeLifecycle::new(
        "a",
        vec![
            profile("a", "remote", Some("a.yaml")),
            profile("b", "remote", Some("b.yaml")),
            profile("other", "remote", Some("other.yaml")),
        ],
    );
    state.apply(&["a", "b"], true).await.unwrap();

    let outcome = state.delete("b", true).await.unwrap();
    assert!(outcome.needs_runtime_update);
    assert_eq!(state.merged(), uids(&["a"]));
    assert_eq!(current(&state).as_deref(), Some("a"));

    let outcome = state.delete("other", true).await.unwrap();
    assert!(!outcome.needs_runtime_update);
    assert_eq!(state.merged(), uids(&["a"]));
    assert_eq!(current(&state).as_deref(), Some("a"));
}

#[tokio::test]
async fn delete_failure_rolls_back_until_the_deletion_commits() {
    let state = MergeLifecycle::new(
        "a",
        vec![
            profile("a", "remote", Some("a.yaml")),
            profile("b", "remote", Some("b.yaml")),
        ],
    );
    state.apply(&["a", "b"], true).await.unwrap();

    let error = state.delete("b", false).await.unwrap_err();
    assert_eq!(error.code.as_deref(), Some("PROFILE_DELETE_FAILED"));
    assert_eq!(state.merged(), uids(&["a", "b"]));
    assert!(has_profile(&state, "b"));
    assert_eq!(current(&state).as_deref(), Some("a"));
    assert_no_pending_draft(&state);

    // A committed deletion is authoritative: restoring then would resurrect a
    // ghost uid in merged.json.
    assert!(should_restore_merge_after_delete_failure(false));
    assert!(!should_restore_merge_after_delete_failure(true));
}
