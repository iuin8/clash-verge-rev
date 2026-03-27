# Multi-Profile Merge Activation — Design Spec

**Date:** 2026-03-28
**Status:** Approved
**Scope:** clash-verge-rev (fork)

---

## Overview

Allow users to select multiple subscription profiles and activate them as a merged configuration. The merge order (and therefore priority) is controlled by a draggable horizontal sort bar. Selections and order are persisted so they survive app restarts.

---

## Data Model

### Backend: `IProfiles` (`src-tauri/src/config/profiles.rs`)

Add one field:

```rust
pub struct IProfiles {
    pub current: Option<String>,      // existing: single-profile activation
    pub merged: Option<Vec<String>>,  // NEW: ordered uid list for multi-merge mode
    pub items: Option<Vec<PrfItem>>,
}
```

**Semantics:**
- `merged` is `None` or empty → existing single-profile path, no behaviour change.
- `merged` has values → ignore `current`, activate the merged result.
- `merged[0]` is the **primary profile**: top-level fields (`dns`, `tun`, `bind-address`, etc.) come from it.
- `merged[1..n]` are **supplementary profiles**: only their `proxies`, `proxy-groups`, and `rules` are contributed.
- Priority is left-to-right (index 0 = highest).
- Persisted to `profiles.yaml`; restored automatically on restart.

---

## Merge Logic

A new function `multi_profile_merge` in `src-tauri/src/enhance/` (separate from the existing `use_merge` which replaces arrays). The existing `use_merge` / `deep_merge` is **not** reused for this feature because it replaces arrays rather than unioning them.

### Algorithm (mirrors the reference JS script)

```
input: ordered list of resolved YAML Mappings [primary, supp1, supp2, ...]

output: merged Mapping + ConflictLog

1. Start with primary as base.

2. For each supplementary config (in order):

   proxies:
     For each proxy p in supp.proxies:
       if base.proxies has no entry with p.name → prepend p to base.proxies
       else → log ConflictEntry { field: "proxies", name: p.name, source: supp_name, reason: Skip }

   proxy-groups:
     For each group g in supp.proxy-groups:
       if base.proxy-groups has no group with g.name:
         → prepend entire group to base.proxy-groups
       else:
         for each member m in g.proxies:
           if not already in existing_group.proxies → prepend m
         (no conflict log for member merging, it is always additive)

   rules:
     For each rule r in supp.rules:
       if r not already in base.rules → prepend r
       (duplicate rules silently skipped, not logged)

   all other top-level keys: ignored from supplementary configs

3. Return (merged_mapping, conflict_log)
```

### `ConflictEntry` type

```rust
struct ConflictEntry {
    field: String,      // "proxies" | "proxy-groups"
    name: String,       // conflicting item name
    source: String,     // supplementary profile name
    reason: String,     // e.g. "already exists in <primary_name>"
}
```

Conflict log is returned alongside the merged config and stored in runtime state so the frontend can display it on demand.

---

## Backend Changes

### New files / additions

| Location | Change |
|----------|--------|
| `src-tauri/src/enhance/multi_merge.rs` | New: `multi_profile_merge(profiles: &[Mapping], names: &[&str]) -> (Mapping, Vec<ConflictEntry>)` |
| `src-tauri/src/config/profiles.rs` | Add `merged: Option<Vec<String>>` to `IProfiles` |
| `src-tauri/src/config/runtime.rs` | Store `conflict_log: Vec<ConflictEntry>` in `IRuntime` |
| `src-tauri/src/core/manager/config.rs` | Branch on `profiles.merged` when building runtime config |
| `src-tauri/src/cmd/profile.rs` | New command `set_merged_profiles(uids: Vec<String>)` |
| `src-tauri/src/cmd/runtime.rs` | New command `get_merge_conflicts() -> Vec<ConflictEntry>` |

All changes are additive. The single-profile code path is untouched.

---

## Frontend Changes

### Profiles page (`src/pages/profiles.tsx`)

The batch-selection skeleton (`batchMode`, `selectedProfiles`) already exists. Extensions needed:

1. **Merge-activate button** — shown in batch toolbar alongside the existing delete button. Calls `set_merged_profiles(orderedUids)`.
2. **Sorted-order bar** — a horizontal `@dnd-kit/sortable` strip that appears when `merged` has ≥ 2 entries (or when selecting for merge). Each chip shows the profile name; drag to reorder. Reorder calls `set_merged_profiles(newOrder)` immediately.
3. **Conflict badge** — a small `Badge` on the order bar. Yellow when `conflicts.length > 0`. Clicking opens a `ConflictViewer` dialog.
4. **Clear merge** — an × button on the order bar to deactivate merge mode and fall back to single-profile `current`.

### New component: `MergeOrderBar` (`src/components/profile/merge-order-bar.tsx`)

```
Props:
  mergedUids: string[]
  profiles: IProfileItem[]
  conflictCount: number
  onReorder: (newUids: string[]) => void
  onClear: () => void
  onShowConflicts: () => void
```

Rendered as a horizontal scrollable strip above the profile grid, visible only when merge mode is active.

### New component: `ConflictViewer` (`src/components/profile/conflict-viewer.tsx`)

Reuses the `LogViewer` visual style (Dialog + Chip + Divider). Each row:

```
[skip]  proxy "HK-01" from 机场B — already exists in 机场A
```

Chip color: `warning` for skipped entries.

---

## Activation Flow

```
User selects profiles in batch mode
  → clicks "Merge Activate"
  → frontend calls set_merged_profiles([uid_primary, uid_supp1, ...])
  → backend saves merged[] to profiles.yaml
  → backend loads each profile's YAML
  → multi_profile_merge() produces (merged_config, conflicts)
  → conflicts stored in IRuntime
  → merged_config fed into existing enhance pipeline (tun injection, field normalization, etc.)
  → mihomo reloaded
  → frontend receives profile-changed event
  → MergeOrderBar appears, ConflictBadge shows count
```

---

## What Is NOT Changed

- Single-profile activation path is untouched.
- Existing `use_merge` / `deep_merge` in `enhance/merge.rs` is untouched.
- `ProfileItem` drag-to-reorder for the main profile list is untouched.
- No new Tauri capabilities or permissions required.

---

## Open Questions (resolved)

| Question | Decision |
|----------|----------|
| Conflict resolution for proxies | Skip duplicate, log it |
| Primary config determination | Leftmost in order bar (index 0) |
| Non-list top-level fields from supplementary | Ignored |
| Persistence | `profiles.yaml` → `merged` field, restored on restart |
| Temporary vs permanent | Permanent until user clicks × to clear |
