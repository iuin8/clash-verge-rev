# Multi-Profile Merge — Rule Provider Fix Checklist

**Date:** 2026-04-18
**Status:** Phase 1 shipped, follow-up plan remains open
**Scope:** clash-verge-rev (fork)
**Related docs:**

- `docs/superpowers/specs/2026-03-28-multi-profile-merge-design.md`
- `docs/superpowers/specs/2026-04-18-multi-profile-merge-rule-provider-strategy.md`
- `docs/repro/local-lan-rule-set/README.md`

---

## Goal

Phase 1 already fixed multi-profile merge so supplementary `rule-providers` are handled correctly.
This checklist now tracks the shipped scope plus follow-up work that is still worth doing.

That shipped phase means:

1. non-conflicting provider definitions merge normally
2. identical provider definitions dedupe safely
3. same-name, different-value definitions are surfaced as explicit conflicts
4. the historical `Local-LAN` missing-provider repro is covered by hermetic regression tests

The remaining milestones are explainability and optional smart rename.

## Original phase-1 target

Fix multi-profile merge so that supplementary `rule-providers` are handled correctly.

That means:

1. non-conflicting provider definitions merge normally
2. identical provider definitions dedupe safely
3. same-name, different-value definitions are surfaced as explicit conflicts
4. optional smart rename can auto-resolve those conflicts later

The first shipping milestone is correctness, not automation.

---

## Root cause to keep in view

Before the fix, `src-tauri/src/enhance/multi_merge.rs` kept supplementary:

- `proxies`
- `proxy-groups`
- `rules`

But dropped supplementary:

- `rule-providers`

That could leave `RULE-SET,<name>,...` references in the merged config with no matching `rule-providers.<name>` definition.

That was the bug fixed in phase 1.

What still matters now is preserving that guarantee while improving conflict UX and optional rename behavior.

---

## Scope guard

Keep the first implementation focused.

### In scope

- `rule-providers` merge behavior
- conflict detection for provider keys
- conflict log improvements needed to explain provider issues
- regression coverage for the `Local-LAN` merge failure

### Out of scope for the first pass

- automatic renaming for every name-bearing config section
- broad merge behavior redesign for all top-level mappings
- UI redesign of the whole conflict viewer
- unrelated merge cleanup

---

## File map to inspect before coding

### Backend merge path

- `src-tauri/src/enhance/multi_merge.rs`
- `src-tauri/src/enhance/mod.rs`

### Runtime conflict plumbing

- `src-tauri/src/cmd/runtime.rs`
- `src-tauri/src/cmd/profile.rs`
- `src/services/cmds.ts`
- `src/pages/profiles.tsx`
- `src/components/profile/conflict-viewer.tsx`

### Profile model

- `src-tauri/src/config/profiles.rs`

### Repro references

- `docs/repro/local-lan-rule-set/10-merge-primary-valid.yaml`
- `docs/repro/local-lan-rule-set/11-merge-supplement-valid.yaml`
- `docs/repro/local-lan-rule-set/12-expected-broken-merged-output.yaml`

---

## Phase 1: Correctness fix

Status: shipped.

The implementation now merges supplementary `rule-providers`, preserves earlier winners on conflicting definitions, and keeps regression coverage for the historical Local-LAN failure.

The checklist below is preserved as a record of what phase 1 needed to accomplish.

### Task 1. Extend conflict entry model if needed

- [ ] Inspect `ConflictEntry` in `src-tauri/src/enhance/multi_merge.rs`
- [ ] Decide whether existing fields are enough for provider conflicts
- [ ] If not, add fields that support provider-specific messaging without over-designing
  - likely candidates: `field`, `name`, `source`, `reason`
  - optional later candidates: `category`, `winner`, `renamed_to`
- [ ] Keep backward compatibility with current conflict viewer if possible

### Task 2. Merge supplementary `rule-providers`

- [ ] Add merge handling for top-level `rule-providers` mappings in `multi_profile_merge`
- [ ] For each supplementary provider key:
  - [ ] if base has no same key, copy it into base
  - [ ] if base has same key and same normalized value, keep one copy
  - [ ] if base has same key and different value, record conflict and keep earlier definition for now
- [ ] Do not silently drop non-conflicting provider keys anymore

### Task 3. Normalize equality check

- [ ] Decide how to compare provider definitions for "same value"
- [ ] Prefer structured `serde_yaml_ng::Value` comparison or a small normalization helper
- [ ] Avoid stringifying full YAML if structured comparison is sufficient
- [ ] Make sure order-sensitive fields are handled intentionally, not accidentally

### Task 4. Preserve current merge order semantics

- [ ] Keep current primary/supplementary ordering model intact
- [ ] Keep current `rules` prepend behavior intact unless a targeted fix is required
- [ ] Make sure provider merge does not reorder unrelated top-level fields

### Task 5. Verify no missing-dependency regression remains

- [ ] Re-run the merge repro from `docs/repro/local-lan-rule-set/README.md`
- [ ] Confirm this order becomes valid after the fix:
  1. `10-merge-primary-valid.yaml`
  2. `11-merge-supplement-valid.yaml`
- [ ] Confirm `rule-providers.Local-LAN` now exists in the merged output

---

## Phase 2: Conflict explainability

### Task 6. Add provider conflict categories or reason strings

- [ ] Ensure conflict logs can distinguish at least these cases:
  - [ ] duplicate same value
  - [ ] duplicate different value
  - [ ] missing dependency, if still possible anywhere in the pipeline
- [ ] Keep the representation simple enough for the current UI to render

### Task 7. Improve conflict viewer wording

- [ ] Check how `src/components/profile/conflict-viewer.tsx` renders current entries
- [ ] Add clear wording for provider conflicts, for example:
  - [ ] same key, same definition -> low-noise info or hidden
  - [ ] same key, different definition -> warning with source profiles
- [ ] Do not tell users to rename keys when the issue was only missing merge support

### Task 8. Improve action guidance

- [ ] For real naming conflicts, show guidance that manual rename is an option
- [ ] For missing dependency scenarios, explain it as a merge result problem, not user error
- [ ] Keep the UI short and human-readable

---

## Phase 3: Smart rename, optional and opt-in

Do this only after correctness is solid.

### Task 9. Add opt-in config or runtime toggle

- [ ] Decide where the toggle lives
  - [ ] profile merge setting
  - [ ] verge config setting
  - [ ] temporary experiment flag
- [ ] Recommended default: `false`
- [ ] Name clearly, for example: `smartRenameRuleProviders`

### Task 10. Implement structured rename flow

- [ ] When enabled, detect same-name, different-value provider conflicts
- [ ] Rewrite the later profile's provider key before merging it into base
- [ ] Rewrite matching `RULE-SET,<name>,...` references from that same source profile
- [ ] Do not run a blind global text replacement across the final YAML

### Task 11. Choose stable rename suffixes

- [ ] Prefer stable names over raw `_1`, `_2` if practical
- [ ] Candidate formats:
  - [ ] `Local-LAN__p2`
  - [ ] `Local-LAN__<profile_slug>`
  - [ ] `Local-LAN__<short_uid>`
- [ ] Ensure rename mapping is visible in logs/UI

### Task 12. Show rename mapping in conflict output

- [ ] Surface at least:
  - [ ] original name
  - [ ] new name
  - [ ] source profile
  - [ ] count of updated `RULE-SET` references if available

---

## Tests and fixtures

### Task 13. Add Rust-level coverage for merge semantics

- [ ] Add or extend tests near `src-tauri/src/enhance/multi_merge.rs`
- [ ] Cover these scenarios:
  - [ ] non-conflicting provider merge
  - [ ] same key, same value dedupe
  - [ ] same key, different value conflict
  - [ ] smart rename off behavior
  - [ ] smart rename on behavior, if phase 3 is included

### Task 14. Keep docs fixtures aligned

- [ ] Keep `docs/repro/local-lan-rule-set/README.md` up to date
- [ ] If merged output changes after the fix, add a new expected-valid merged fixture instead of overwriting the broken baseline without explanation
- [ ] Preserve the broken-baseline fixture because it documents the original bug

---

## Suggested work order for one implementation session

If doing the smallest useful shipment first:

1. update `multi_profile_merge` to merge non-conflicting `rule-providers`
2. add equality check for same-key/same-value dedupe
3. log same-key/different-value conflicts conservatively
4. add regression tests for the Local-LAN repro
5. verify the current broken merge repro now passes
6. improve conflict wording if needed
7. leave smart rename for a second pass unless there is still time

That gets the bug fixed without turning one bug fix into a mini config language project.

---

## Definition of done for Phase 1

Phase 1 is done when all of these are true:

- [ ] merging `10-merge-primary-valid.yaml` + `11-merge-supplement-valid.yaml` no longer drops `rule-providers.Local-LAN`
- [ ] merged config validates where the current implementation fails
- [ ] same-key/same-value provider definitions do not produce noisy conflicts
- [ ] same-key/different-value provider definitions are explicit and traceable
- [ ] regression tests exist and fail without the fix
- [ ] docs/repro still explain the original failure and the new expected behavior

---

## Notes for the implementer

- Do not confuse baseline correctness with smart rename. They are not the same feature.
- The user-facing story should remain honest: merge bugs are ours, naming conflicts may be theirs.
- Keep the first diff small. This is a config correctness fix, not a merge-framework rewrite.
