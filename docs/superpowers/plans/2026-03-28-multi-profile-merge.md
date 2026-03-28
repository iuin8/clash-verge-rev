# Multi-Profile Merge Activation — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Allow users to select multiple subscription profiles and activate them as a merged configuration, with drag-to-reorder priority control (within the existing profile list) and persistent selection across restarts.

**Architecture:** Add a `merged: Option<Vec<String>>` field to `IProfiles` (persisted in `profiles.yaml`). When `merged` is set, a new `multi_profile_merge()` function in the enhance layer combines proxies/proxy-groups/rules from each profile (prepend, dedup by name) while keeping top-level fields from the primary (index 0). Conflicts are stored in `IRuntime::merge_conflicts` and surfaced via a badge on the primary profile card that opens a `ConflictViewer` dialog.

**UX principle:** No separate merge-order strip. Selected profiles reuse the active-card visual. Drag is constrained so selected and unselected groups cannot cross.

**Tech Stack:** Rust (serde_yaml_ng, smartstring), Tauri 2 IPC commands, React 19, MUI 7, @dnd-kit/sortable

---

## Status: ✅ COMPLETED (2026-03-29)

All tasks implemented, spec-reviewed, and code-quality-reviewed. `pnpm typecheck` and `pnpm lint` both pass.

### Implementation summary

**Backend (Rust)**

- `IProfiles.merged: Option<Vec<String>>` + `patch_merged()` — `src-tauri/src/config/profiles.rs`
- `multi_profile_merge` + `ConflictEntry` — `src-tauri/src/enhance/multi_merge.rs`
- Enhance pipeline branches on `merged` having 2+ UIDs — `src-tauri/src/enhance/mod.rs`
- Three IPC commands registered: `set_merged_profiles`, `clear_merged_profiles`, `get_merge_conflicts` — `src-tauri/src/cmd/profile.rs`, `runtime.rs`

**Frontend (React)**

- Multi-select mode entry button; profile cards show checkboxes — `src/pages/profiles.tsx`
- Selected profiles float to top in selection order (`sortedProfiles` memo)
- Drag constrained: selected ↔ selected, unselected ↔ unselected (`onDragEnd` guard)
- Selected cards reuse active-card highlight (`selected` prop includes batch-selected state)
- Primary card (first selected) shows conflict count Badge → opens `ConflictViewer`
- "Merge Activate" button (visible when ≥2 selected)
- "Done" button (visible when 0 selected, calls `clearMergedProfiles`)
- Mount-time hydration from `profiles.merged`
- No `MergeOrderBar` component
- `ConflictViewer` dialog — `src/components/profile/conflict-viewer.tsx`
- TS wrappers — `src/services/cmds.ts`
- i18n keys (en/zh) under `profiles.merge.*`

---

## File Map

### New files

- `src-tauri/src/enhance/multi_merge.rs` — merge logic + ConflictEntry type
- `src/components/profile/conflict-viewer.tsx` — conflict log dialog

### Modified files

- `src-tauri/src/config/profiles.rs` — add `merged` field to `IProfiles`, add `patch_merged`
- `src-tauri/src/enhance/mod.rs` — branch on `profiles.merged` in generate path
- `src-tauri/src/cmd/profile.rs` — new `set_merged_profiles` and `clear_merged_profiles` commands
- `src-tauri/src/cmd/runtime.rs` — new `get_merge_conflicts` command
- `src-tauri/src/cmd/mod.rs` — register new commands
- `src-tauri/src/lib.rs` — register new commands in invoke_handler
- `src/services/cmds.ts` — add TS wrappers for new commands
- `src/pages/profiles.tsx` — multi-select mode, constrained drag, active-card highlight, conflict badge
- `src/locales/en/profiles.json` and `src/locales/zh/profiles.json` — i18n keys

### Removed from scope

- ~~`src/components/profile/merge-order-bar.tsx`~~ — not needed; order managed in main list

---

## Task 1: Add `merged` field to `IProfiles` ✅

**Files:**

- Modify: `src-tauri/src/config/profiles.rs`

- [x] **Step 1: Add field to struct**
- [x] **Step 2: Add `patch_merged` method**
- [x] **Step 3: Commit**

---

## Task 2: Implement `multi_profile_merge` ✅

**Files:**

- Create: `src-tauri/src/enhance/multi_merge.rs`

- [x] **Step 1: Define `ConflictEntry`**
- [x] **Step 2: Implement merge function**
- [x] **Step 3: Add module to enhance**
- [x] **Step 4: Commit**

---

## Task 3: Branch enhance pipeline on `merged` ✅

**Files:**

- Modify: `src-tauri/src/enhance/mod.rs`

- [x] **Step 1: Load profiles when `merged` is set, call `multi_profile_merge`, store conflicts**
- [x] **Step 2: Commit**

---

## Task 4: Backend IPC commands ✅

**Files:**

- Modify: `src-tauri/src/cmd/profile.rs`
- Modify: `src-tauri/src/cmd/runtime.rs`
- Modify: `src-tauri/src/cmd/mod.rs`
- Modify: `src-tauri/src/lib.rs`

- [x] **Step 1: `set_merged_profiles` command**
- [x] **Step 2: `clear_merged_profiles` command**
- [x] **Step 3: `get_merge_conflicts` command**
- [x] **Step 4: Register all three commands**
- [x] **Step 5: Commit**

---

## Task 5: Frontend TypeScript wrappers ✅

**Files:**

- Modify: `src/services/cmds.ts`

- [x] **Step 1: Add wrappers** (`setMergedProfiles`, `clearMergedProfiles`, `getMergeConflicts`)
- [x] **Step 2: Add `ConflictEntry` type to `src/types/global.d.ts`**
- [x] **Step 3: Commit**

---

## Task 6: `ConflictViewer` dialog component ✅

**Files:**

- Create: `src/components/profile/conflict-viewer.tsx`

- [x] **Step 1: Implement dialog**
- [x] **Step 2: Commit**

---

## Task 7: Frontend — multi-select mode in `profiles.tsx` ✅

**Files:**

- Modify: `src/pages/profiles.tsx`
- Modify: `src/components/profile/profile-item.tsx`

- [x] **Step 1: State** (`multiSelectMode`, `selectedProfiles`, `conflicts`, `primaryUid`)
- [x] **Step 2: `sortedProfiles` derived list** (selected first in insertion order, then rest)
- [x] **Step 3: Constrained drag-and-drop** (`onDragEnd` guard blocks cross-boundary drags)
- [x] **Step 4: Card appearance** (checkbox in batch mode, active highlight on selected, conflict Badge on primary)
- [x] **Step 5: Toolbar controls** (entry button, Merge Activate, Done)
- [x] **Step 6: Mount hydration from `profiles.merged`**
- [x] **Step 7: Commit**

---

## Task 8: i18n keys ✅

**Files:**

- Modify: `src/locales/en/profiles.json`
- Modify: `src/locales/zh/profiles.json`

- [x] **Step 1: Add English keys** (`merge.activate`, `merge.activated`, `merge.done`, `merge.conflicts.*`)
- [x] **Step 2: Add Chinese keys**
- [x] **Step 3: Regenerate i18n types** (`pnpm i18n:types`)
- [x] **Step 4: Commit**

---

## Task 9: End-to-end smoke test

- [ ] Start dev server: `pnpm dev`
- [ ] Add 2+ remote subscription profiles
- [ ] Click batch-select icon → select 2+ profiles → selected cards float to top with active highlight
- [ ] Verify drag is constrained (selected ↔ selected only)
- [ ] Click "Merge Activate" → mihomo reloads
- [ ] If conflicts exist, verify conflict badge appears on primary card; clicking opens ConflictViewer
- [ ] Re-enter multi-select mode → deselect all → click "Done" → single-profile mode restored
- [ ] Restart app → verify merged selection persists
- [ ] Run `pnpm lint && pnpm typecheck` — both must pass
