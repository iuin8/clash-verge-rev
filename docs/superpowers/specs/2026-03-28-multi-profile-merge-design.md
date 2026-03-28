# Multi-Profile Merge Activation — Design Spec

**Date:** 2026-03-28
**Updated:** 2026-03-29
**Status:** Approved (revised)
**Scope:** clash-verge-rev (fork)

---

## Overview

Allow users to select multiple subscription profiles and activate them as a merged configuration. The merge order (and therefore priority) is controlled by dragging profile cards within the existing list. Selections and order are persisted so they survive app restarts.

**Key UX principle:** No separate merge UI strip. Selected profiles reuse the same active-card visual as single-profile mode, minimising visual noise.

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
- Priority is left-to-right in the list (top = highest, index 0).
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
         for each member m in g.proxies (the members list, not the group itself):
           if base group does not contain m → append m to the matching base group
           else → log ConflictEntry { field: "proxy-groups", name: g.name+":"+m, ... }

   rules:
     For each rule r in supp.rules:
       if base.rules does not already contain r → prepend r to base.rules
       else → log ConflictEntry { field: "rules", name: r, ... }

3. Return merged Mapping + ConflictLog.
```

### Conflict Log

```rust
pub struct ConflictEntry {
    pub field: String,   // "proxies" | "proxy-groups" | "rules"
    pub name: String,    // duplicate item name
    pub source: String,  // profile name that was skipped
    pub reason: String,  // "Skip" etc.
}
```

Stored in `IRuntime` under key `"MultiMerge"` via the existing `chain_logs` mechanism.

---

## UI / Interaction Model

### Multi-Select Mode Entry

- A **batch-select icon button** (existing or new) in the profile list toolbar toggles multi-select mode.
- In multi-select mode each profile card shows a **checkbox**.

### Visual State of Selected Profiles

- Selected (merged) profiles display the **same active-card highlight** as single-profile activation.
- No separate `MergeOrderBar` strip is rendered.
- The primary profile (index 0 of `merged`) additionally shows a **small conflict-count badge** (warning colour) if `ConflictLog` is non-empty. Clicking the badge opens `ConflictViewer` dialog.

### List Ordering

- When multi-select mode is active and profiles are selected:
  - All **selected profiles float to the top** of the list, maintaining their relative order.
  - Drag-to-reorder is constrained: selected cards can only be dragged among other selected cards; unselected cards can only be dragged among unselected cards. The two groups cannot cross.
- Priority is top-to-bottom (topmost selected = primary, index 0).

### Activating the Merge

- A **"Merge Activate"** button appears in the toolbar (or as a contextual action) when ≥2 profiles are selected.
- Clicking it calls `set_merged_profiles([uid0, uid1, ...])` in the backend.

### Exiting Multi-Select / Merge Mode

- Re-enter multi-select mode via the same entry button.
- Uncheck individual profiles to deselect them.
- When **all profiles are deselected**, a **"Done"** button becomes available.
- Clicking "Done" calls `clear_merged_profiles()` → backend reactivates the previous `current` profile (original primary) → single-profile mode restored.

---

## Backend Commands

| Command                 | Signature                           | Effect                                              |
| ----------------------- | ----------------------------------- | --------------------------------------------------- |
| `set_merged_profiles`   | `(uids: Vec<String>) -> Result<()>` | Save `merged`, run enhance pipeline, reload mihomo  |
| `clear_merged_profiles` | `() -> Result<()>`                  | Clear `merged`, reactivate `current`, reload mihomo |
| `get_merge_conflicts`   | `() -> Result<Vec<ConflictEntry>>`  | Return stored conflict log                          |

---

## Conflict Viewer

Reuses `LogViewer` visual style (Dialog + Chip + Divider). Opened from badge on primary profile card. Each row:

```
[skip]  proxy "HK-01" from 机场B — already exists in 机场A
```

Chip colour: `warning` for skipped entries.

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
  → selected profile cards show active highlight; primary card shows conflict badge if needed
```

---

## What Is NOT Changed

- Single-profile activation path is untouched.
- Existing `use_merge` / `deep_merge` in `enhance/merge.rs` is untouched.
- No `MergeOrderBar` component (removed from scope).
- No new Tauri capabilities or permissions required.

---

## Open Questions (resolved)

| Question                                     | Decision                                                   |
| -------------------------------------------- | ---------------------------------------------------------- |
| Conflict resolution for proxies              | Skip duplicate, log it                                     |
| Primary config determination                 | Topmost in list (index 0 of `merged`)                      |
| Non-list top-level fields from supplementary | Ignored                                                    |
| Persistence                                  | `profiles.yaml` → `merged` field, restored on restart      |
| Temporary vs permanent                       | Permanent until user clicks "Done" after deselecting all   |
| Merge UI strip                               | **Removed** — selected cards reuse active-card style       |
| Conflict entry point                         | Small badge on primary profile card                        |
| Drag constraint                              | Selected ↔ selected only; unselected ↔ unselected only     |
| Exit merge mode                              | Deselect all → "Done" button → restores previous `current` |
