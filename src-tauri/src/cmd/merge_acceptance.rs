//! Isolated acceptance for the multi-profile merge lifecycle.
//!
//! The merge commands need a Tauri app handle, a running core, and the user's
//! data directory, so this module drives the same deterministic transitions
//! over an in-memory `merged.json` stand-in and the production decision
//! helpers. Reverting one of those decisions makes these tests fail.
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
use smartstring::alias::String;

#[derive(Debug, PartialEq, Eq)]
struct DeleteOutcome {
    needs_runtime_update: bool,
}

/// Mirrors the command state: profiles draft, the merged.json list, and
/// whether that file currently fails to parse.
struct MergeLifecycle {
    profiles: IProfiles,
    merged: Vec<String>,
    corrupt: bool,
}

impl MergeLifecycle {
    fn new(current: &str, items: Vec<PrfItem>) -> Self {
        Self {
            profiles: IProfiles {
                current: Some(current.into()),
                items: Some(items),
            },
            merged: Vec::new(),
            corrupt: false,
        }
    }

    fn load(&self) -> AnyResult<Vec<String>> {
        if self.corrupt {
            Err(anyhow::anyhow!("invalid merged.json"))
        } else {
            Ok(self.merged.clone())
        }
    }

    fn save(&mut self, uids: Vec<String>) {
        self.merged = uids;
        self.corrupt = false;
    }

    /// Mirrors `set_merged_profiles`: validate, select the first member as
    /// primary, persist, then commit the runtime update or roll back.
    fn apply(&mut self, uids: &[&str], runtime_ok: bool) -> CmdResult {
        let next: Vec<String> = uids.iter().map(|uid| (*uid).into()).collect();
        let previous = previous_merged_for_update(self.load(), false)
            .map_err(|error| coded_error("MERGED_PROFILE_LOAD_FAILED", error))?;
        validate_merged_members(&self.profiles, &next)?;

        let mut candidate = self.profiles.clone();
        if let Some(primary) = next.first() {
            candidate.patch_config(&IProfiles {
                current: Some(primary.clone()),
                items: None,
            });
        }
        self.save(next);
        if runtime_ok {
            self.profiles = candidate;
            Ok(())
        } else {
            self.save(previous);
            Err(coded_error("MERGED_PROFILE_FAILED", "runtime config rejected"))
        }
    }

    /// Mirrors `clear_merged_profiles`: an explicit clear recovers a corrupt
    /// store; a failed runtime update restores whatever was there before.
    fn clear(&mut self, runtime_ok: bool) -> CmdResult {
        let previous = previous_merged_for_update(self.load(), true)
            .map_err(|error| coded_error("MERGED_PROFILE_LOAD_FAILED", error))?;
        self.save(Vec::new());
        if runtime_ok {
            Ok(())
        } else {
            self.save(previous);
            Err(coded_error("MERGED_PROFILE_FAILED", "runtime config rejected"))
        }
    }

    /// Mirrors `patch_profiles_config`: switching to a member keeps the merge;
    /// switching outside it clears the merge and restores it on failure.
    fn switch(&mut self, target: &str, runtime_ok: bool) -> CmdResult<bool> {
        let previous_profile = self.profiles.current.clone();
        let previous_merged = self
            .load()
            .map_err(|error| coded_error("PROFILE_SWITCH_FAILED", error))?;
        let target: String = target.into();
        let clears_merge = clears_merge_for_switch(Some(&target), &previous_merged);
        if clears_merge {
            self.save(Vec::new());
        }

        let mut candidate = self.profiles.clone();
        candidate.patch_config(&IProfiles {
            current: Some(target),
            items: None,
        });
        if runtime_ok {
            self.profiles = candidate;
        } else {
            self.profiles.current = previous_profile;
        }
        if should_restore_merge_after_switch(clears_merge, runtime_ok) {
            self.save(previous_merged);
        }
        Ok(runtime_ok)
    }

    /// Mirrors `delete_profile`: prune the merged list, repair `current` to a
    /// remaining member, and restore the list if the runtime update fails
    /// before the deletion commits.
    fn delete(&mut self, uid: &str, runtime_ok: bool) -> CmdResult<DeleteOutcome> {
        let previous_merged = self
            .load()
            .map_err(|error| coded_error("MERGED_PROFILE_LOAD_FAILED", error))?;
        let mut candidate = self.profiles.clone();
        let (should_update, _files) = candidate
            .plan_delete_item(uid)
            .map_err(|error| coded_error("PROFILE_DELETE_FAILED", error))?;

        let merge_plan = plan_delete_merge(candidate.current.as_ref(), uid, &previous_merged, should_update);
        self.save(merge_plan.remaining.clone());
        if let Some(primary) = merge_plan.repaired_current.clone() {
            candidate.patch_config(&IProfiles {
                current: Some(primary),
                items: None,
            });
        }

        if merge_plan.needs_runtime_update && !runtime_ok {
            // The deletion has not committed yet, so the new merged state is
            // not authoritative and the previous list must come back.
            if should_restore_merge_after_delete_failure(false) {
                self.save(previous_merged);
            }
            return Err(coded_error("PROFILE_DELETE_FAILED", "runtime config rejected"));
        }
        self.profiles = candidate;
        Ok(DeleteOutcome {
            needs_runtime_update: merge_plan.needs_runtime_update,
        })
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

fn current(state: &MergeLifecycle) -> Option<&str> {
    state.profiles.current.as_deref()
}

#[test]
fn apply_selects_the_first_member_and_rejects_invalid_members() {
    let mut state = MergeLifecycle::new(
        "a",
        vec![
            profile("a", "remote", Some("a.yaml")),
            profile("b", "local", Some("b.yaml")),
            profile("chain", "merge", Some("chain.yaml")),
            profile("missing", "remote", None),
        ],
    );

    for rejected in [&["chain"][..], &["missing"][..], &["absent"][..]] {
        let error = state.apply(rejected, true).unwrap_err();
        assert_eq!(error.code.as_deref(), Some("MERGED_PROFILE_INVALID"));
    }
    assert!(state.merged.is_empty());

    state.apply(&["b", "a"], true).unwrap();
    assert_eq!(state.merged, uids(&["b", "a"]));
    assert_eq!(current(&state), Some("b"));
}

#[test]
fn apply_failure_restores_the_previous_selection_and_fails_closed_when_corrupt() {
    let mut state = MergeLifecycle::new(
        "a",
        vec![
            profile("a", "remote", Some("a.yaml")),
            profile("b", "remote", Some("b.yaml")),
        ],
    );
    state.apply(&["a", "b"], true).unwrap();

    let error = state.apply(&["b"], false).unwrap_err();
    assert_eq!(error.code.as_deref(), Some("MERGED_PROFILE_FAILED"));
    assert_eq!(state.merged, uids(&["a", "b"]));
    assert_eq!(current(&state), Some("a"));

    state.corrupt = true;
    let error = state.apply(&["a"], true).unwrap_err();
    assert_eq!(error.code.as_deref(), Some("MERGED_PROFILE_LOAD_FAILED"));
    assert_eq!(state.merged, uids(&["a", "b"]));
}

#[test]
fn clear_drops_the_selection_and_recovers_a_corrupt_store() {
    let mut state = MergeLifecycle::new(
        "a",
        vec![
            profile("a", "remote", Some("a.yaml")),
            profile("b", "remote", Some("b.yaml")),
        ],
    );
    state.apply(&["a", "b"], true).unwrap();

    state.clear(true).unwrap();
    assert!(state.merged.is_empty());
    assert_eq!(current(&state), Some("a"));

    state.corrupt = true;
    assert_eq!(
        state.delete("b", true).unwrap_err().code.as_deref(),
        Some("MERGED_PROFILE_LOAD_FAILED")
    );
    assert_eq!(
        state.switch("b", true).unwrap_err().code.as_deref(),
        Some("PROFILE_SWITCH_FAILED")
    );

    state.clear(true).unwrap();
    assert!(!state.corrupt);
    assert!(state.merged.is_empty());
}

#[test]
fn switch_keeps_membership_and_clears_only_outside_the_merge() {
    let mut state = MergeLifecycle::new(
        "a",
        vec![
            profile("a", "remote", Some("a.yaml")),
            profile("b", "remote", Some("b.yaml")),
            profile("c", "remote", Some("c.yaml")),
        ],
    );

    state.apply(&["a", "b"], true).unwrap();
    assert!(state.switch("b", true).unwrap());
    assert_eq!(state.merged, uids(&["a", "b"]));
    assert_eq!(current(&state), Some("b"));

    assert!(state.switch("c", true).unwrap());
    assert!(state.merged.is_empty());
    assert_eq!(current(&state), Some("c"));

    state.apply(&["a", "b"], true).unwrap();
    assert!(!state.switch("c", false).unwrap());
    assert_eq!(state.merged, uids(&["a", "b"]));
    assert_eq!(current(&state), Some("a"));

    state.apply(&["a", "b"], true).unwrap();
    assert!(!state.switch("b", false).unwrap());
    assert_eq!(state.merged, uids(&["a", "b"]));
    assert_eq!(current(&state), Some("a"));
}

#[test]
fn deleting_the_current_merged_member_repairs_current_to_a_remaining_member() {
    let mut state = MergeLifecycle::new(
        "a",
        vec![
            profile("a", "remote", Some("a.yaml")),
            // A subscription that is not part of the merge sits first after
            // the deleted primary; current must not fall onto it.
            profile("other", "remote", Some("other.yaml")),
            profile("b", "remote", Some("b.yaml")),
        ],
    );
    state.apply(&["a", "b"], true).unwrap();

    let outcome = state.delete("a", true).unwrap();
    assert!(outcome.needs_runtime_update);
    assert_eq!(state.merged, uids(&["b"]));
    assert_eq!(current(&state), Some("b"));
    assert!(state.profiles.get_item("a").is_err());

    let outcome = state.delete("b", true).unwrap();
    assert!(outcome.needs_runtime_update);
    assert!(state.merged.is_empty());
    assert_eq!(current(&state), Some("other"));
}

#[test]
fn deleting_a_non_current_member_prunes_the_list_and_still_refreshes_runtime() {
    let mut state = MergeLifecycle::new(
        "a",
        vec![
            profile("a", "remote", Some("a.yaml")),
            profile("b", "remote", Some("b.yaml")),
            profile("other", "remote", Some("other.yaml")),
        ],
    );
    state.apply(&["a", "b"], true).unwrap();

    let outcome = state.delete("b", true).unwrap();
    assert!(outcome.needs_runtime_update);
    assert_eq!(state.merged, uids(&["a"]));
    assert_eq!(current(&state), Some("a"));

    let outcome = state.delete("other", true).unwrap();
    assert!(!outcome.needs_runtime_update);
    assert_eq!(state.merged, uids(&["a"]));
    assert_eq!(current(&state), Some("a"));
}

#[test]
fn delete_failure_rolls_back_until_the_deletion_commits() {
    let mut state = MergeLifecycle::new(
        "a",
        vec![
            profile("a", "remote", Some("a.yaml")),
            profile("b", "remote", Some("b.yaml")),
        ],
    );
    state.apply(&["a", "b"], true).unwrap();

    let error = state.delete("b", false).unwrap_err();
    assert_eq!(error.code.as_deref(), Some("PROFILE_DELETE_FAILED"));
    assert_eq!(state.merged, uids(&["a", "b"]));
    assert!(state.profiles.get_item("b").is_ok());
    assert_eq!(current(&state), Some("a"));

    // A committed deletion is authoritative: restoring then would resurrect a
    // ghost uid in merged.json.
    assert!(should_restore_merge_after_delete_failure(false));
    assert!(!should_restore_merge_after_delete_failure(true));
}
