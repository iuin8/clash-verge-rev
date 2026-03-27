# Multi-Profile Merge Activation — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Allow users to select multiple subscription profiles and activate them as a merged configuration, with drag-to-reorder priority control and persistent selection across restarts.

**Architecture:** Add a `merged: Option<Vec<String>>` field to `IProfiles` (persisted in `profiles.yaml`). When `merged` is set, a new `multi_profile_merge()` function in the enhance layer combines proxies/proxy-groups/rules from each profile (prepend, dedup by name) while keeping top-level fields from the primary (index 0). Conflicts are stored in `IRuntime::chain_logs` under the key `"MultiMerge"` and surfaced in a new `ConflictViewer` dialog in the frontend.

**Tech Stack:** Rust (serde_yaml_ng, smartstring), Tauri 2 IPC commands, React 19, MUI 7, @dnd-kit/sortable

---

## File Map

### New files
- `src-tauri/src/enhance/multi_merge.rs` — merge logic + ConflictEntry type
- `src/components/profile/merge-order-bar.tsx` — horizontal drag-to-reorder strip
- `src/components/profile/conflict-viewer.tsx` — conflict log dialog

### Modified files
- `src-tauri/src/config/profiles.rs` — add `merged` field to `IProfiles`, add `patch_merged`
- `src-tauri/src/enhance/mod.rs` — branch on `profiles.merged` in generate path
- `src-tauri/src/cmd/profile.rs` — new `set_merged_profiles` and `clear_merged_profiles` commands
- `src-tauri/src/cmd/runtime.rs` — new `get_merge_conflicts` command
- `src-tauri/src/cmd/mod.rs` — register new commands
- `src-tauri/src/lib.rs` — register new commands in invoke_handler
- `src/services/cmds.ts` — add TS wrappers for new commands
- `src/pages/profiles.tsx` — wire MergeOrderBar + batch-activate button
- `src/locales/en.json` and `src/locales/zh.json` — i18n keys

---

## Task 1: Add `merged` field to `IProfiles`

**Files:**
- Modify: `src-tauri/src/config/profiles.rs`

- [ ] **Step 1: Add field to struct**

In `src-tauri/src/config/profiles.rs`, change the `IProfiles` struct:

```rust
#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct IProfiles {
    /// same as PrfConfig.current
    pub current: Option<String>,

    /// ordered uid list for multi-merge activation (index 0 = primary)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merged: Option<Vec<String>>,

    /// profile list
    pub items: Option<Vec<PrfItem>>,
}
```

- [ ] **Step 2: Add `patch_merged` method**

In the `impl IProfiles` block (after the existing `patch_config` method), add:

```rust
pub fn patch_merged(&mut self, merged: Option<Vec<String>>) {
    self.merged = merged;
}
```

- [ ] **Step 3: Verify compilation**

```bash
cd src-tauri && cargo check 2>&1 | head -30
```
Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/config/profiles.rs
git commit -m "feat(config): add merged field to IProfiles for multi-profile merge"
```

---

## Task 2: Implement `multi_profile_merge`

**Files:**
- Create: `src-tauri/src/enhance/multi_merge.rs`
- Modify: `src-tauri/src/enhance/mod.rs`

- [ ] **Step 1: Write failing test first**

Create `src-tauri/src/enhance/multi_merge.rs` with the test:

```rust
use serde_yaml_ng::{Mapping, Value};
use smartstring::alias::String as SmartString;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConflictEntry {
    pub field: SmartString,
    pub name: SmartString,
    pub source: SmartString,
    pub reason: SmartString,
}

/// Merge multiple profile configs. `names[i]` is the display name for `configs[i]`.
/// configs[0] is primary: its top-level fields (dns, tun, etc.) are kept as-is.
/// configs[1..] contribute only proxies, proxy-groups, rules (prepend, dedup by name).
pub fn multi_profile_merge(
    configs: &[Mapping],
    names: &[&str],
) -> (Mapping, Vec<ConflictEntry>) {
    // placeholder
    (Mapping::new(), vec![])
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_yaml_ng::from_str;

    #[test]
    fn merges_proxies_from_supplementary() {
        let primary: Mapping = from_str("\
proxies:\n\
  - name: NodeA\n    type: ss\n    server: a.com\n    port: 443\n    cipher: aes-256-gcm\n    password: x\n\
dns:\n  enable: true\n\
").unwrap();
        let supp: Mapping = from_str("\
proxies:\n\
  - name: NodeB\n    type: ss\n    server: b.com\n    port: 443\n    cipher: aes-256-gcm\n    password: y\n\
").unwrap();
        let (merged, conflicts) = multi_profile_merge(&[primary, supp], &["Primary", "Supp"]);
        let proxies = merged.get("proxies").and_then(|v| v.as_sequence()).unwrap();
        // NodeB prepended, NodeA follows
        assert_eq!(proxies.len(), 2);
        assert_eq!(proxies[0].get("name").and_then(|v| v.as_str()), Some("NodeB"));
        assert_eq!(proxies[1].get("name").and_then(|v| v.as_str()), Some("NodeA"));
        assert!(conflicts.is_empty());
        // dns from primary preserved
        assert!(merged.contains_key("dns"));
    }

    #[test]
    fn skips_duplicate_proxy_and_logs_conflict() {
        let primary: Mapping = from_str("\
proxies:\n\
  - name: NodeA\n    type: ss\n    server: a.com\n    port: 443\n    cipher: aes-256-gcm\n    password: x\n\
").unwrap();
        let supp: Mapping = from_str("\
proxies:\n\
  - name: NodeA\n    type: ss\n    server: b.com\n    port: 443\n    cipher: aes-256-gcm\n    password: y\n\
").unwrap();
        let (merged, conflicts) = multi_profile_merge(&[primary, supp], &["P", "S"]);
        let proxies = merged.get("proxies").and_then(|v| v.as_sequence()).unwrap();
        assert_eq!(proxies.len(), 1);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].field.as_str(), "proxies");
        assert_eq!(conflicts[0].name.as_str(), "NodeA");
    }

    #[test]
    fn prepends_rules_from_supplementary() {
        let primary: Mapping = from_str("rules:\n  - MATCH,DIRECT\n").unwrap();
        let supp: Mapping = from_str("rules:\n  - IP-CIDR,10.0.0.0/24,MY_PROXY\n").unwrap();
        let (merged, _) = multi_profile_merge(&[primary, supp], &["P", "S"]);
        let rules = merged.get("rules").and_then(|v| v.as_sequence()).unwrap();
        assert_eq!(rules[0].as_str(), Some("IP-CIDR,10.0.0.0/24,MY_PROXY"));
        assert_eq!(rules[1].as_str(), Some("MATCH,DIRECT"));
    }

    #[test]
    fn supplementary_top_level_fields_ignored() {
        let primary: Mapping = from_str("dns:\n  enable: false\n").unwrap();
        let supp: Mapping = from_str("dns:\n  enable: true\nproxies: []\n").unwrap();
        let (merged, _) = multi_profile_merge(&[primary, supp], &["P", "S"]);
        let dns_enable = merged
            .get("dns")
            .and_then(|v| v.as_mapping())
            .and_then(|m| m.get("enable"))
            .and_then(|v| v.as_bool());
        assert_eq!(dns_enable, Some(false));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cd src-tauri && cargo test multi_merge 2>&1 | tail -20
```
Expected: 4 FAILED (placeholder returns empty mapping).

- [ ] **Step 3: Implement `multi_profile_merge`**

Replace the placeholder `multi_profile_merge` function body:

```rust
pub fn multi_profile_merge(
    configs: &[Mapping],
    names: &[&str],
) -> (Mapping, Vec<ConflictEntry>) {
    if configs.is_empty() {
        return (Mapping::new(), vec![]);
    }

    let mut base = configs[0].clone();
    let mut conflicts: Vec<ConflictEntry> = Vec::new();

    // Collect existing proxy names from primary
    let mut known_proxies: std::collections::HashSet<std::string::String> = base
        .get("proxies")
        .and_then(Value::as_sequence)
        .map(|seq| {
            seq.iter()
                .filter_map(|v| v.as_mapping())
                .filter_map(|m| m.get("name").and_then(Value::as_str))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();

    for (i, supp) in configs[1..].iter().enumerate() {
        let source_name = names.get(i + 1).copied().unwrap_or("unknown");

        // --- proxies ---
        if let Some(Value::Sequence(supp_proxies)) = supp.get("proxies") {
            let base_proxies = base
                .entry(Value::String("proxies".into()))
                .or_insert_with(|| Value::Sequence(vec![]));
            if let Value::Sequence(ref mut base_seq) = base_proxies {
                for proxy in supp_proxies.iter().rev() {
                    let name = proxy
                        .as_mapping()
                        .and_then(|m| m.get("name").and_then(Value::as_str))
                        .unwrap_or("");
                    if known_proxies.contains(name) {
                        conflicts.push(ConflictEntry {
                            field: "proxies".into(),
                            name: name.into(),
                            source: source_name.into(),
                            reason: format!("already exists, skipped").into(),
                        });
                    } else {
                        known_proxies.insert(name.to_owned());
                        base_seq.insert(0, proxy.clone());
                    }
                }
            }
        }

        // --- proxy-groups ---
        if let Some(Value::Sequence(supp_groups)) = supp.get("proxy-groups") {
            let base_groups = base
                .entry(Value::String("proxy-groups".into()))
                .or_insert_with(|| Value::Sequence(vec![]));
            if let Value::Sequence(ref mut base_seq) = base_groups {
                for group in supp_groups.iter().rev() {
                    let gname = group
                        .as_mapping()
                        .and_then(|m| m.get("name").and_then(Value::as_str))
                        .unwrap_or("");
                    if let Some(existing) = base_seq.iter_mut().find(|g| {
                        g.as_mapping()
                            .and_then(|m| m.get("name").and_then(Value::as_str))
                            == Some(gname)
                    }) {
                        // merge members into existing group
                        if let (Value::Mapping(ref mut ex_map), Value::Mapping(supp_map)) =
                            (existing, group)
                        {
                            let ex_members = ex_map
                                .entry(Value::String("proxies".into()))
                                .or_insert_with(|| Value::Sequence(vec![]));
                            if let Value::Sequence(ref mut ex_list) = ex_members {
                                let ex_set: std::collections::HashSet<std::string::String> =
                                    ex_list
                                        .iter()
                                        .filter_map(|v| v.as_str().map(str::to_owned))
                                        .collect();
                                if let Some(Value::Sequence(supp_members)) =
                                    supp_map.get("proxies")
                                {
                                    for m in supp_members.iter().rev() {
                                        if m.as_str().map(|s| !ex_set.contains(s)).unwrap_or(false) {
                                            ex_list.insert(0, m.clone());
                                        }
                                    }
                                }
                            }
                        }
                    } else {
                        base_seq.insert(0, group.clone());
                    }
                }
            }
        }

        // --- rules ---
        if let Some(Value::Sequence(supp_rules)) = supp.get("rules") {
            let base_rules = base
                .entry(Value::String("rules".into()))
                .or_insert_with(|| Value::Sequence(vec![]));
            if let Value::Sequence(ref mut base_seq) = base_rules {
                let existing_set: std::collections::HashSet<std::string::String> = base_seq
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect();
                for rule in supp_rules.iter().rev() {
                    if rule.as_str().map(|s| !existing_set.contains(s)).unwrap_or(false) {
                        base_seq.insert(0, rule.clone());
                    }
                }
            }
        }
        // all other top-level keys from supp are ignored
    }

    (base, conflicts)
}
```

- [ ] **Step 4: Register module in `enhance/mod.rs`**

Add at the top of `src-tauri/src/enhance/mod.rs`:

```rust
pub mod multi_merge;
```

- [ ] **Step 5: Run tests to verify they pass**

```bash
cd src-tauri && cargo test multi_merge 2>&1 | tail -20
```
Expected: 4 tests PASSED.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/enhance/multi_merge.rs src-tauri/src/enhance/mod.rs
git commit -m "feat(enhance): implement multi_profile_merge with conflict logging"
```

---

## Task 3: Wire merge path in enhance pipeline

**Files:**
- Modify: `src-tauri/src/enhance/mod.rs`
- Modify: `src-tauri/src/config/runtime.rs`

- [ ] **Step 1: Add `merge_conflicts` field to `IRuntime`**

In `src-tauri/src/config/runtime.rs`, update the struct:

```rust
#[derive(Default, Clone)]
pub struct IRuntime {
    pub config: Option<Mapping>,
    pub exists_keys: HashSet<String>,
    pub chain_logs: HashMap<String, Vec<(String, String)>>,
    pub merge_conflicts: Vec<crate::enhance::multi_merge::ConflictEntry>, // NEW
}
```

- [ ] **Step 2: Branch in enhance `generate` on `profiles.merged`**

In `src-tauri/src/enhance/mod.rs`, in the main `generate` (or `use_generate`) function that builds `IRuntime`, add a branch before the normal single-profile path:

Find the location where `profiles.current` is resolved to load the active profile YAML. Add above it:

```rust
// Multi-merge path
if let Some(merged_uids) = &profiles.merged {
    if !merged_uids.is_empty() {
        let mut configs: Vec<serde_yaml_ng::Mapping> = Vec::new();
        let mut names: Vec<String> = Vec::new();
        for uid in merged_uids {
            if let Ok(item) = profiles_config.get_item(uid) {
                let file = item.file.as_deref().unwrap_or("");
                let path = dirs::app_profiles_dir()?.join(file);
                if let Ok(content) = tokio::fs::read_to_string(&path).await {
                    if let Ok(mapping) = serde_yaml_ng::from_str::<serde_yaml_ng::Mapping>(&content) {
                        names.push(item.name.as_deref().unwrap_or(uid).to_owned());
                        configs.push(mapping);
                    }
                }
            }
        }
        if !configs.is_empty() {
            let name_refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
            let (merged_config, conflicts) = crate::enhance::multi_merge::multi_profile_merge(&configs, &name_refs);
            // store conflicts in runtime
            // continue normal pipeline with merged_config as base
            return Ok((merged_config, conflicts));
        }
    }
}
// ... existing single-profile path continues
```

> Note: The exact integration point depends on how `generate` is structured. The key is: load each uid's YAML file, call `multi_profile_merge`, use the result as the base config feeding into the rest of the enhance pipeline (tun injection, field normalization). Conflicts go into `IRuntime::merge_conflicts`.

- [ ] **Step 3: Store conflicts in runtime**

In `src-tauri/src/core/manager/config.rs`, in `use_default_config` and wherever `IRuntime` is constructed after a multi-merge, set:

```rust
Config::runtime().await.edit_draft(|d| {
    d.config = Some(merged_config);
    d.merge_conflicts = conflicts;
    // exists_keys, chain_logs as normal
});
```

- [ ] **Step 4: Verify compilation**

```bash
cd src-tauri && cargo check 2>&1 | head -40
```
Expected: no errors.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/enhance/mod.rs src-tauri/src/config/runtime.rs src-tauri/src/core/manager/config.rs
git commit -m "feat(enhance): wire multi-merge path in config generate pipeline"
```

---

## Task 4: New Tauri commands

**Files:**
- Modify: `src-tauri/src/cmd/profile.rs`
- Modify: `src-tauri/src/cmd/runtime.rs`
- Modify: `src-tauri/src/cmd/mod.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Add `set_merged_profiles` and `clear_merged_profiles` commands**

Append to `src-tauri/src/cmd/profile.rs`:

```rust
/// Set the ordered list of uids for multi-profile merge activation.
/// Passing an empty vec clears merge mode.
#[tauri::command]
pub async fn set_merged_profiles(uids: Vec<String>) -> CmdResult<bool> {
    logging!(info, Type::Cmd, "设置合并配置列表: {:?}", uids);
    let merged = if uids.is_empty() { None } else { Some(uids.into_iter().map(SmartString::from).collect()) };
    Config::profiles().await.edit_draft(|d| d.patch_merged(merged));
    Config::profiles().await.latest_arc().save_file().await.stringify_err()?;
    Ok(true)
}

/// Clear merge mode (equivalent to set_merged_profiles with empty list).
#[tauri::command]
pub async fn clear_merged_profiles() -> CmdResult {
    Config::profiles().await.edit_draft(|d| d.patch_merged(None));
    Config::profiles().await.latest_arc().save_file().await.stringify_err()?;
    Ok(())
}
```

Note: `SmartString` is already imported in that file as `use smartstring::alias::String;` — use `String::from(uid)` for the conversion.

- [ ] **Step 2: Add `get_merge_conflicts` command**

Append to `src-tauri/src/cmd/runtime.rs`:

```rust
/// Get conflict entries from the last multi-profile merge.
#[tauri::command]
pub async fn get_merge_conflicts() -> CmdResult<Vec<crate::enhance::multi_merge::ConflictEntry>> {
    Ok(Config::runtime().await.latest_arc().merge_conflicts.clone())
}
```

- [ ] **Step 3: Register in `cmd/mod.rs`**

In `src-tauri/src/cmd/mod.rs`, ensure the new commands are re-exported. Check if the file uses wildcard re-exports or explicit ones and add accordingly:

```rust
pub use profile::set_merged_profiles;
pub use profile::clear_merged_profiles;
pub use runtime::get_merge_conflicts;
```

- [ ] **Step 4: Register in `lib.rs` invoke_handler**

In `src-tauri/src/lib.rs`, find the `tauri::generate_handler![...]` macro call and add:

```rust
cmd::set_merged_profiles,
cmd::clear_merged_profiles,
cmd::get_merge_conflicts,
```

- [ ] **Step 5: Verify compilation**

```bash
cd src-tauri && cargo check 2>&1 | head -40
```
Expected: no errors.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/cmd/profile.rs src-tauri/src/cmd/runtime.rs src-tauri/src/cmd/mod.rs src-tauri/src/lib.rs
git commit -m "feat(cmd): add set_merged_profiles, clear_merged_profiles, get_merge_conflicts commands"
```

---

## Task 5: Frontend service wrappers + types

**Files:**
- Modify: `src/services/cmds.ts`
- Modify: `src/types/` (add to existing interfaces file or create `src/types/profile-merge.ts`)

- [ ] **Step 1: Add type**

Add to `src/types/index.d.ts` (or the file where `IProfilesConfig` is defined):

```typescript
interface IConflictEntry {
  field: string
  name: string
  source: string
  reason: string
}

// Extend IProfilesConfig
interface IProfilesConfig {
  current: string | null
  merged?: string[] | null  // NEW
  items: IProfileItem[]
}
```

- [ ] **Step 2: Add service functions**

Append to `src/services/cmds.ts`:

```typescript
export async function setMergedProfiles(uids: string[]) {
  return invoke<boolean>('set_merged_profiles', { uids })
}

export async function clearMergedProfiles() {
  return invoke<void>('clear_merged_profiles')
}

export async function getMergeConflicts() {
  return invoke<IConflictEntry[]>('get_merge_conflicts')
}
```

- [ ] **Step 3: Typecheck**

```bash
pnpm typecheck 2>&1 | head -30
```
Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add src/services/cmds.ts src/types/
git commit -m "feat(frontend): add TS wrappers for merge commands"
```

---

## Task 6: `MergeOrderBar` component

**Files:**
- Create: `src/components/profile/merge-order-bar.tsx`

- [ ] **Step 1: Create component**

Create `src/components/profile/merge-order-bar.tsx`:

```tsx
import {
  closestCenter,
  DndContext,
  DragEndEvent,
  KeyboardSensor,
  PointerSensor,
  useSensor,
  useSensors,
} from '@dnd-kit/core'
import {
  arrayMove,
  horizontalListSortingStrategy,
  SortableContext,
  sortableKeyboardCoordinates,
  useSortable,
} from '@dnd-kit/sortable'
import { CSS } from '@dnd-kit/utilities'
import { ClearRounded, WarningAmberRounded } from '@mui/icons-material'
import { Badge, Box, Chip, IconButton, Tooltip } from '@mui/material'
import { useTranslation } from 'react-i18next'

interface Props {
  mergedUids: string[]
  profileNames: Record<string, string> // uid -> name
  conflictCount: number
  onReorder: (newUids: string[]) => void
  onClear: () => void
  onShowConflicts: () => void
}

function SortableChip({ uid, label }: { uid: string; label: string }) {
  const { attributes, listeners, setNodeRef, transform, transition } =
    useSortable({ id: uid })
  return (
    <Chip
      ref={setNodeRef}
      label={label}
      size="small"
      sx={{
        cursor: 'grab',
        transform: CSS.Transform.toString(transform),
        transition,
        '&:first-of-type': { fontWeight: 700 },
      }}
      {...attributes}
      {...listeners}
    />
  )
}

export function MergeOrderBar({
  mergedUids,
  profileNames,
  conflictCount,
  onReorder,
  onClear,
  onShowConflicts,
}: Props) {
  const { t } = useTranslation()
  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 4 } }),
    useSensor(KeyboardSensor, {
      coordinateGetter: sortableKeyboardCoordinates,
    }),
  )

  const handleDragEnd = (event: DragEndEvent) => {
    const { active, over } = event
    if (over && active.id !== over.id) {
      const oldIndex = mergedUids.indexOf(active.id as string)
      const newIndex = mergedUids.indexOf(over.id as string)
      onReorder(arrayMove(mergedUids, oldIndex, newIndex))
    }
  }

  return (
    <Box
      sx={{
        display: 'flex',
        alignItems: 'center',
        gap: 0.5,
        px: 1,
        py: 0.5,
        overflowX: 'auto',
        bgcolor: 'action.hover',
        borderRadius: 1,
        mx: '10px',
        mb: 0.5,
      }}
    >
      <DndContext
        sensors={sensors}
        collisionDetection={closestCenter}
        onDragEnd={handleDragEnd}
      >
        <SortableContext
          items={mergedUids}
          strategy={horizontalListSortingStrategy}
        >
          {mergedUids.map((uid) => (
            <SortableChip
              key={uid}
              uid={uid}
              label={profileNames[uid] ?? uid}
            />
          ))}
        </SortableContext>
      </DndContext>

      {conflictCount > 0 && (
        <Tooltip title={t('profiles.merge.conflicts.badge')}>
          <IconButton size="small" color="warning" onClick={onShowConflicts}>
            <Badge badgeContent={conflictCount} color="warning">
              <WarningAmberRounded fontSize="small" />
            </Badge>
          </IconButton>
        </Tooltip>
      )}

      <IconButton size="small" onClick={onClear} sx={{ ml: 'auto' }}>
        <ClearRounded fontSize="small" />
      </IconButton>
    </Box>
  )
}
```

- [ ] **Step 2: Typecheck**

```bash
pnpm typecheck 2>&1 | head -20
```
Expected: no errors.

- [ ] **Step 3: Commit**

```bash
git add src/components/profile/merge-order-bar.tsx
git commit -m "feat(ui): add MergeOrderBar component with horizontal drag-to-reorder"
```

---

## Task 7: `ConflictViewer` component

**Files:**
- Create: `src/components/profile/conflict-viewer.tsx`

- [ ] **Step 1: Create component**

Create `src/components/profile/conflict-viewer.tsx`:

```tsx
import {
  Button,
  Chip,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  Divider,
  Typography,
} from '@mui/material'
import { Fragment } from 'react'
import { useTranslation } from 'react-i18next'
import { BaseEmpty } from '@/components/base'

interface Props {
  open: boolean
  conflicts: IConflictEntry[]
  onClose: () => void
}

export function ConflictViewer({ open, conflicts, onClose }: Props) {
  const { t } = useTranslation()
  return (
    <Dialog open={open} onClose={onClose}>
      <DialogTitle>{t('profiles.merge.conflicts.title')}</DialogTitle>
      <DialogContent sx={{ width: 440, height: 300, overflowX: 'hidden', userSelect: 'text', pb: 1 }}>
        {conflicts.map((c, i) => (
          <Fragment key={i}>
            <Typography color="text.secondary" component="div">
              <Chip
                label={c.field}
                size="small"
                variant="outlined"
                color="warning"
                sx={{ mr: 1 }}
              />
              <strong>{c.name}</strong> from <em>{c.source}</em> — {c.reason}
            </Typography>
            <Divider sx={{ my: 0.5 }} />
          </Fragment>
        ))}
        {conflicts.length === 0 && <BaseEmpty />}
      </DialogContent>
      <DialogActions>
        <Button onClick={onClose} variant="outlined">{t('shared.actions.close')}</Button>
      </DialogActions>
    </Dialog>
  )
}
```

- [ ] **Step 2: Commit**

```bash
git add src/components/profile/conflict-viewer.tsx
git commit -m "feat(ui): add ConflictViewer dialog for merge conflicts"
```

---

## Task 8: Wire everything in `profiles.tsx`

**Files:**
- Modify: `src/pages/profiles.tsx`

- [ ] **Step 1: Import new services and components**

Add to imports in `src/pages/profiles.tsx`:

```typescript
import { MergeOrderBar } from '@/components/profile/merge-order-bar'
import { ConflictViewer } from '@/components/profile/conflict-viewer'
import {
  setMergedProfiles,
  clearMergedProfiles,
  getMergeConflicts,
} from '@/services/cmds'
```

- [ ] **Step 2: Add state**

Inside `ProfilePage`, add:

```typescript
const [conflictOpen, setConflictOpen] = useState(false)
const [conflicts, setConflicts] = useState<IConflictEntry[]>([])
```

- [ ] **Step 3: Add merge-activate handler**

```typescript
const onMergeActivate = useLockFn(async () => {
  const uids = [...selectedProfiles]
  if (uids.length < 2) return
  await setMergedProfiles(uids)
  await mutateProfiles()
  const c = await getMergeConflicts()
  setConflicts(c)
  setBatchMode(false)
  setSelectedProfiles(new Set())
  showNotice.success('profiles.merge.activated')
})

const onMergeClear = useLockFn(async () => {
  await clearMergedProfiles()
  await mutateProfiles()
  setConflicts([])
})

const onMergeReorder = useLockFn(async (newUids: string[]) => {
  await setMergedProfiles(newUids)
  await mutateProfiles()
})
```

- [ ] **Step 4: Add merge-activate button in batch toolbar**

In the batch mode toolbar JSX (near the delete button around line 893), add:

```tsx
<Button
  size="small"
  variant="contained"
  disabled={selectedProfiles.size < 2}
  onClick={onMergeActivate}
>
  {t('profiles.merge.activate')}
</Button>
```

- [ ] **Step 5: Render MergeOrderBar above profile grid**

After the URL input Stack and before the profile Grid, add:

```tsx
{profiles.merged && profiles.merged.length >= 2 && (
  <MergeOrderBar
    mergedUids={profiles.merged}
    profileNames={Object.fromEntries(
      (profiles.items ?? []).map((i) => [i.uid, i.name ?? i.uid])
    )}
    conflictCount={conflicts.length}
    onReorder={onMergeReorder}
    onClear={onMergeClear}
    onShowConflicts={() => setConflictOpen(true)}
  />
)}
<ConflictViewer
  open={conflictOpen}
  conflicts={conflicts}
  onClose={() => setConflictOpen(false)}
/>
```

- [ ] **Step 6: Typecheck + lint**

```bash
pnpm typecheck 2>&1 | head -20
pnpm lint 2>&1 | head -20
```
Expected: no errors.

- [ ] **Step 7: Commit**

```bash
git add src/pages/profiles.tsx
git commit -m "feat(ui): wire multi-merge activation, order bar, and conflict viewer into profiles page"
```

---

## Task 9: i18n keys

**Files:**
- Modify: `src/locales/en.json`
- Modify: `src/locales/zh.json`

- [ ] **Step 1: Add English keys**

In `src/locales/en.json`, under `profiles`, add:

```json
"merge": {
  "activate": "Merge Activate",
  "activated": "Profiles merged and activated",
  "conflicts": {
    "title": "Merge Conflicts",
    "badge": "Show merge conflicts"
  }
}
```

- [ ] **Step 2: Add Chinese keys**

In `src/locales/zh.json`, under `profiles`, add:

```json
"merge": {
  "activate": "合并激活",
  "activated": "订阅已合并激活",
  "conflicts": {
    "title": "合并冲突",
    "badge": "显示合并冲突"
  }
}
```

- [ ] **Step 3: Regenerate i18n types**

```bash
pnpm i18n:types
```

- [ ] **Step 4: Commit**

```bash
git add src/locales/
git commit -m "feat(i18n): add merge activation i18n keys"
```

---

## Task 10: End-to-end smoke test

- [ ] Start dev server: `pnpm dev`
- [ ] Add 2+ remote subscription profiles
- [ ] Click batch-select icon → select 2 profiles → click "Merge Activate"
- [ ] Verify `MergeOrderBar` appears above profile list with both profiles as chips
- [ ] Drag chips to reorder → verify order persists after app restart
- [ ] If conflicts exist, verify yellow badge and conflict dialog list them correctly
- [ ] Click × on order bar → verify single-profile mode restored
- [ ] Run `pnpm lint && pnpm typecheck` — both must pass

