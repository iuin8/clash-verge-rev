# multi_merge 重构实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 消除 multi_merge 的静默错误（重名 proxy/provider 导致 group 指向错误版本、引用不存在）并引入 A 类字段 deep merge。

**Architecture:** 把 `multi_profile_merge` 重构为声明式步骤链（`MergeContext` + `STEPS`），引入三层引用完整性（定义比较去重 → 重命名保底+引用重写 → 引用存在性校验），并对自包含 mapping 字段做递归 deep merge。

**Tech Stack:** Rust 1.91、serde_yaml_ng、cargo test

**Spec:** `docs/superpowers/specs/2026-08-14-multi-merge-refactor-design.md`

## Global Constraints

- 重名不同定义的重命名后缀：`X [supp_name]`
- 引用完整性校验失败：记 `ConflictEntry` + 降级移除引用，**不阻断启动**
- 定义比较：仅忽略 `name` 字段，其余全比
- deep merge 范围：仅 `dns`/`tun`/`hosts`/`profile`
- 所有合并语义保持「primary 赢 + conflict 记录」；只有 I/O 错误才返回 Err
- 命名/注释风格与 `multi_merge.rs` 现有代码一致（snake_case 函数、`//` 注释）

---

## 文件结构

- Modify: `src-tauri/src/config/profiles.rs` — 阶段 1 治本
- Modify: `src-tauri/src/enhance/multi_merge.rs` — 阶段 2、3 全部逻辑 + 测试（文件内 `#[cfg(test)] mod tests`）

---

### Task 1: 治本（delete_item 同步清理 merged）

**Files:**
- Modify: `src-tauri/src/config/profiles.rs:341-396`（`delete_item`）
- Test: `src-tauri/src/config/profiles.rs`（`mod tests` 内）

**Interfaces:**
- Produces: `fn retain_merged_after_delete(merged: Option<Vec<String>>, removed: &HashSet<String>) -> Option<Vec<String>>` — 供 `delete_item` 调用，且被测试

- [ ] **Step 1: 写失败测试**

在 `src-tauri/src/config/profiles.rs` 的 `mod tests` 内新增：

```rust
#[test]
fn retain_merged_after_delete_removes_deleted_uids() {
    use std::collections::HashSet;

    let merged = Some(vec!["a".into(), "b".into(), "c".into()]);
    let removed: HashSet<String> = ["a".into(), "c".into()].into_iter().collect();
    assert_eq!(
        retain_merged_after_delete(merged, &removed),
        Some(vec!["b".to_string()])
    );
}

#[test]
fn retain_merged_after_delete_nones_when_all_removed() {
    use std::collections::HashSet;

    let merged = Some(vec!["a".into(), "b".into()]);
    let removed: HashSet<String> = ["a".into(), "b".into()].into_iter().collect();
    assert_eq!(retain_merged_after_delete(merged, &removed), None);
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd src-tauri && cargo test --lib retain_merged_after_delete`
Expected: FAIL（`retain_merged_after_delete` 未定义）

- [ ] **Step 3: 实现纯函数**

在 `src-tauri/src/config/profiles.rs` 的 `impl IProfiles` 块外（文件级）新增：

```rust
/// 删除 profile 后同步清理 merged：移除被删 uid 及其关联 uid，全空时置 None。
fn retain_merged_after_delete(
    merged: Option<Vec<String>>,
    removed: &HashSet<String>,
) -> Option<Vec<String>> {
    let mut merged = merged?;
    merged.retain(|u| !removed.contains(u));
    if merged.is_empty() {
        None
    } else {
        Some(merged)
    }
}
```

- [ ] **Step 4: 在 delete_item 中调用**

在 `delete_item`（341-396）里，`self.items = Some(items);` 之前插入：

```rust
// 同步清理 merged，避免残留已删除 profile 的 uid
let removed: HashSet<String> = std::iter::once(uid.clone())
    .chain(delete_uids.iter().filter_map(|u| u.clone()))
    .collect();
self.merged = retain_merged_after_delete(self.merged.take(), &removed);
```

（`delete_uids` 是 `Vec<Option<String>>`，需 `filter_map` 去 None。）

- [ ] **Step 5: 跑测试确认通过 + 回归**

Run: `cd src-tauri && cargo test --lib retain_merged_after_delete && cargo test --lib config::profiles`
Expected: 新增测试 PASS，profiles 现有测试全绿

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/config/profiles.rs
git commit -m "fix: clean merged list when deleting a profile"
```

---

### Task 2: 声明式管线重构（行为等价）

**Files:**
- Modify: `src-tauri/src/enhance/multi_merge.rs:457-477`（`multi_profile_merge`）及 6 个 merge 函数签名

**Interfaces:**
- Produces:
  - `struct MergeContext { primary_name: String, supp_name: String, conflicts: Vec<ConflictEntry>, renames: HashMap<String, String> }`
  - `type MergeStep = fn(&mut MergeContext, &mut Mapping, &Mapping);`
  - `const MERGE_STEPS: &[MergeStep]` — 只含 6 个 merge 步骤，不含层 3
- Consumes: 现有 `ConflictEntry`、`Mapping`（serde_yaml_ng）、`push_conflict`

- [ ] **Step 1: 定义 MergeContext 与 MergeStep**

在 `multi_merge.rs` 顶部（`push_conflict` 之后）新增：

```rust
/// 跨合并步骤共享的状态：重命名映射在 proxies/providers 步骤产出、在 groups/rules 步骤消费。
struct MergeContext {
    primary_name: String,
    supp_name: String,
    conflicts: Vec<ConflictEntry>,
    renames: HashMap<String, String>,
}

type MergeStep = fn(&mut MergeContext, &mut Mapping, &Mapping);

const MERGE_STEPS: &[MergeStep] = &[
    merge_proxies,
    merge_proxy_providers,
    merge_proxy_groups,
    merge_rules,
    merge_rule_providers,
    log_top_level_key_conflicts,
];
```

- [ ] **Step 2: 改 6 个 merge 函数签名（机械替换）**

把每个函数签名从 `(base, supp, supp_name, primary_name, conflicts)` 改为 `(ctx, base, supp)`，函数体内 `primary_name`→`&ctx.primary_name`、`supp_name`→`&ctx.supp_name`、`conflicts`→`&mut ctx.conflicts`。

例如 `merge_proxies` 改为：

```rust
fn merge_proxies(ctx: &mut MergeContext, base: &mut Mapping, supp: &Mapping) {
    if let Some(Value::Sequence(supp_proxies)) = supp.get("proxies") {
        if supp_proxies.is_empty() {
            return;
        }
        let base_seq = ensure_sequence_field(base, "proxies", &ctx.primary_name, &mut ctx.conflicts);
        // ... 其余逻辑照旧，conflicts 引用改为 &mut ctx.conflicts，primary_name 改为 &ctx.primary_name
    }
}
```

其余 5 个函数同理。`merge_named_mapping` 的 `field` 参数保留（它被 merge_rule_providers/merge_proxy_providers 复用）：

```rust
fn merge_named_mapping(
    ctx: &mut MergeContext,
    base: &mut Mapping,
    supp: &Mapping,
    field: &str,
) {
    // ... 内部 primary_name/supp_name/conflicts 同上替换
}
```

`merge_rule_providers` / `merge_proxy_providers` 变为：

```rust
fn merge_rule_providers(ctx: &mut MergeContext, base: &mut Mapping, supp: &Mapping) {
    merge_named_mapping(ctx, base, supp, "rule-providers");
}
fn merge_proxy_providers(ctx: &mut MergeContext, base: &mut Mapping, supp: &Mapping) {
    merge_named_mapping(ctx, base, supp, "proxy-providers");
}
```

- [ ] **Step 3: 改写 multi_profile_merge 主循环**

```rust
pub fn multi_profile_merge(configs: &[Mapping], names: &[&str]) -> (Mapping, Vec<ConflictEntry>) {
    if configs.is_empty() {
        return (Mapping::new(), vec![]);
    }

    let mut base = configs[0].clone();
    let primary_name = names.first().copied().unwrap_or("primary");
    let mut all_conflicts: Vec<ConflictEntry> = vec![];

    for (i, supp) in configs[1..].iter().enumerate() {
        let supp_name = names.get(i + 1).copied().unwrap_or("unknown");
        let mut ctx = MergeContext {
            primary_name: primary_name.into(),
            supp_name: supp_name.into(),
            conflicts: vec![],
            renames: HashMap::new(),
        };
        for step in MERGE_STEPS {
            step(&mut ctx, &mut base, supp);
        }
        all_conflicts.extend(ctx.conflicts);
    }

    (base, all_conflicts)
}
```

（`HashMap` 已在文件顶部 `use std::collections::HashSet;` 处补 `HashMap`。）

- [ ] **Step 4: 跑现有测试确认行为等价**

Run: `cd src-tauri && cargo test --lib enhance::multi_merge`
Expected: 现有全部测试 PASS（行为未变）

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/enhance/multi_merge.rs
git commit -m "refactor: declarative merge step pipeline in multi_merge"
```

---

### Task 3: 层 1+2（定义比较去重 + 重命名保底 + 引用重写）

**Files:**
- Modify: `src-tauri/src/enhance/multi_merge.rs` — `merge_proxies`、`merge_named_mapping`、`merge_proxy_groups`、`merge_rules`
- Test: 同文件 `mod tests`

**Interfaces:**
- Produces:
  - `fn strip_name(mapping: &Mapping) -> Mapping` — 返回去掉 `name` 键的浅拷贝，供定义比较
  - `fn renamed_target<'a>(ctx: &'a MergeContext, name: &'a str) -> Cow<'a, str>` — 返回重命名后的引用名（命中 `ctx.renames` 则返回映射值，否则原样）
- Consumes: `MergeContext.renames`（Task 2 已定义）

- [ ] **Step 1: 写失败测试（proxies 重名不同定义 → 重命名）**

```rust
#[test]
fn duplicate_proxy_name_with_different_definition_is_renamed_not_dropped() {
    let primary = mapping("proxies:\n  - name: dup\n    type: ss\n    server: a.com");
    let supp = mapping(
        "proxies:\n  - name: dup\n    type: vmess\n    server: b.com\nproxy-groups:\n  - name: g\n    type: select\n    proxies:\n      - dup",
    );
    let (result, _conflicts) = multi_profile_merge(&[primary, supp], &["primary", "supp"]);

    let proxies = result.get("proxies").unwrap().as_sequence().unwrap();
    assert_eq!(proxies.len(), 2);
    let renamed = proxies[1].as_mapping().unwrap();
    assert_eq!(renamed.get("name").unwrap().as_str().unwrap(), "dup [supp]");

    // group g 的引用应指向重命名后的节点
    let groups = result.get("proxy-groups").unwrap().as_sequence().unwrap();
    let g = groups[0].as_mapping().unwrap();
    let members = g.get("proxies").unwrap().as_sequence().unwrap();
    assert!(members.iter().any(|m| m.as_str() == Some("dup [supp]")));
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd src-tauri && cargo test --lib duplicate_proxy_name_with_different_definition_is_renamed_not_dropped`
Expected: FAIL（当前重名即丢弃）

- [ ] **Step 3: 实现 strip_name 与 renamed_target**

```rust
fn strip_name(mapping: &Mapping) -> Mapping {
    mapping
        .iter()
        .filter(|(k, _)| k.as_str() != Some("name"))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

fn renamed_target<'a>(ctx: &'a MergeContext, name: &'a str) -> Cow<'a, str> {
    if let Some(new_name) = ctx.renames.get(name) {
        Cow::Borrowed(new_name.as_str())
    } else {
        Cow::Borrowed(name)
    }
}
```

（文件顶部补 `use std::borrow::Cow;`。）

- [ ] **Step 4: 改 merge_proxies（层 1 + 层 2 产出）**

```rust
fn merge_proxies(ctx: &mut MergeContext, base: &mut Mapping, supp: &Mapping) {
    if let Some(Value::Sequence(supp_proxies)) = supp.get("proxies") {
        if supp_proxies.is_empty() {
            return;
        }
        let base_seq = ensure_sequence_field(base, "proxies", &ctx.primary_name, &mut ctx.conflicts);
        let existing: HashMap<String, Mapping> = base_seq
            .iter()
            .filter_map(|p| p.as_mapping())
            .filter_map(|m| m.get("name").and_then(Value::as_str).map(|n| (n.to_owned(), m.clone())))
            .collect();

        let mut to_prepend: Vec<Value> = vec![];
        for proxy in supp_proxies {
            let Some(proxy_map) = proxy.as_mapping() else { continue };
            let Some(proxy_name) = proxy_map.get("name").and_then(Value::as_str) else { continue };
            match existing.get(proxy_name) {
                None => to_prepend.push(proxy.clone()),
                Some(primary_def) => {
                    if strip_name(primary_def) == strip_name(proxy_map) {
                        continue; // 真重复，安全丢弃
                    }
                    // 定义不同：重命名保底
                    let new_name = format!("{proxy_name} [{}]", ctx.supp_name);
                    ctx.renames.insert(proxy_name.to_owned(), new_name.clone());
                    let mut renamed = proxy_map.clone();
                    renamed.insert(Value::String("name".into()), Value::String(new_name));
                    to_prepend.push(Value::Mapping(renamed));
                }
            }
        }

        for item in to_prepend.into_iter().rev() {
            base_seq.insert(0, item);
        }
    }
}
```

- [ ] **Step 5: 改 merge_named_mapping（providers 层 1+2）**

```rust
fn merge_named_mapping(ctx: &mut MergeContext, base: &mut Mapping, supp: &Mapping, field: &str) {
    if let Some(Value::Mapping(supp_map)) = supp.get(field) {
        if supp_map.is_empty() {
            return;
        }
        let base_map = ensure_mapping_field(base, field, &ctx.primary_name, &mut ctx.conflicts);
        for (name, value) in supp_map {
            let name_str = name.as_str().map(str::to_owned).unwrap_or_else(|| format!("{name:?}"));
            match base_map.get(name) {
                None => {
                    base_map.insert(name.clone(), value.clone());
                }
                Some(existing) if existing == value => {}
                Some(existing) => {
                    // 同名不同定义：重命名保底（provider 可含内联 payload，比较整块 value）
                    if existing == value {
                        continue;
                    }
                    let new_name = format!("{name_str} [{}]", ctx.supp_name);
                    ctx.renames.insert(name_str.clone(), new_name.clone());
                    base_map.insert(Value::String(new_name), value.clone());
                }
            }
        }
    }
}
```

- [ ] **Step 6: 改 merge_proxy_groups 与 merge_rules 消费 renames**

`merge_group_member_list` 的成员插入前，把成员名经 `renamed_target` 重写。`merge_rules` 里对 `RULE-SET,<name>,...` 前缀的 rule，把 `<name>` 经 `renamed_target` 重写后写回。

`merge_proxy_groups` 中 `merge_group_member_list` 调用处，成员 `member_name` 改为：

```rust
let resolved = renamed_target(ctx, member_name);
if seen_members.insert(resolved.to_string()) {
    members_to_prepend.push(Value::String(resolved.into_owned()));
}
```

`merge_rules` 中 rule 处理，对 `rule_str` 若以 `"RULE-SET,"` 开头，取逗号分隔第 2 段，经 `renamed_target` 替换：

```rust
fn rewrite_rule_set(rule: &str, ctx: &MergeContext) -> String {
    if let Some(rest) = rule.strip_prefix("RULE-SET,") {
        if let Some(provider_name) = rest.split(',').next() {
            return format!("RULE-SET,{},{}", renamed_target(ctx, provider_name), &rest[provider_name.len()..]);
        }
    }
    rule.to_owned()
}
```

- [ ] **Step 7: 更新现有测试 duplicate_proxy_name_logged_as_conflict**

该测试断言「重名 → 记 conflict + 丢弃」，现行为改为「重名不同定义 → 重命名」。改为断言重命名结果：

```rust
#[test]
fn duplicate_proxy_name_logged_as_conflict() {
    let primary = mapping("proxies:\n  - name: dup\n    type: ss");
    let supp = mapping("proxies:\n  - name: dup\n    type: vmess");
    let (result, _conflicts) = multi_profile_merge(&[primary, supp], &["primary", "supp"]);
    let proxies = result.get("proxies").unwrap().as_sequence().unwrap();
    assert_eq!(proxies.len(), 2);
    assert_eq!(proxies[1].as_mapping().unwrap().get("name").unwrap().as_str().unwrap(), "dup [supp]");
}
```

- [ ] **Step 8: 跑测试 + 回归**

Run: `cd src-tauri && cargo test --lib enhance::multi_merge`
Expected: 新增/更新测试 PASS，其余现有测试全绿（注意：若其他测试依赖「重名即丢弃」的旧行为，需一并更新为断言重命名）

- [ ] **Step 9: Commit**

```bash
git add src-tauri/src/enhance/multi_merge.rs
git commit -m "feat: definition-aware dedup with rename fallback for proxies/providers"
```

---

### Task 4: 层 3（引用完整性校验）

**Files:**
- Modify: `src-tauri/src/enhance/multi_merge.rs` — 新增 `verify_reference_integrity`，并在 `multi_profile_merge` 末尾调用
- Test: 同文件 `mod tests`

**Interfaces:**
- Produces: `fn verify_reference_integrity(base: &mut Mapping, conflicts: &mut Vec<ConflictEntry>)`
- Consumes: `ConflictEntry`、`push_conflict`

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn group_referencing_missing_proxy_is_dropped_with_conflict() {
    let primary = mapping("proxies:\n  - name: p1\n    type: ss\nproxy-groups:\n  - name: g\n    type: select\n    proxies:\n      - p1\n      - ghost");
    let supp = mapping("rules:\n  - MATCH,DIRECT");
    let (result, conflicts) = multi_profile_merge(&[primary, supp], &["primary", "supp"]);

    assert!(conflicts.iter().any(|c| c.name == "ghost"));
    let groups = result.get("proxy-groups").unwrap().as_sequence().unwrap();
    let members = groups[0].as_mapping().unwrap().get("proxies").unwrap().as_sequence().unwrap();
    assert!(!members.iter().any(|m| m.as_str() == Some("ghost")));
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd src-tauri && cargo test --lib group_referencing_missing_proxy_is_dropped_with_conflict`
Expected: FAIL（当前不校验，ghost 保留）

- [ ] **Step 3: 实现 verify_reference_integrity**

```rust
/// 合并完成后校验所有 group 的 proxies/use 成员引用是否存在于最终 config，
/// 不存在的记 conflict 并从成员中移除（降级，不阻断）。
fn verify_reference_integrity(base: &mut Mapping, conflicts: &mut Vec<ConflictEntry>) {
    let proxy_names: HashSet<String> = base
        .get("proxies").and_then(Value::as_sequence).into_iter().flatten()
        .filter_map(|p| p.as_mapping().and_then(|m| m.get("name")).and_then(Value::as_str).map(str::to_owned))
        .collect();
    let provider_names: HashSet<String> = base
        .get("proxy-providers").and_then(Value::as_mapping).into_iter()
        .filter_map(|(k, _)| k.as_str().map(str::to_owned))
        .collect();
    let group_names: HashSet<String> = base
        .get("proxy-groups").and_then(Value::as_sequence).into_iter().flatten()
        .filter_map(|g| g.as_mapping().and_then(|m| m.get("name")).and_then(Value::as_str).map(str::to_owned))
        .collect();

    let Some(Value::Sequence(groups)) = base.get_mut("proxy-groups") else { return };
    for group in groups.iter_mut() {
        let Some(group_map) = group.as_mapping_mut() else { continue };
        let group_name = group_map.get("name").and_then(Value::as_str).map(str::to_owned).unwrap_or_default();
        for field in ["proxies", "use"] {
            let Some(Value::Sequence(members)) = group_map.get_mut(field) else { continue };
            let valid: Vec<Value> = members.iter().filter(|m| {
                let Some(name) = m.as_str() else { return true };
                let ok = proxy_names.contains(name) || provider_names.contains(name) || group_names.contains(name) || name == "DIRECT";
                if !ok {
                    push_conflict(conflicts, "proxy-groups", name, &group_name, format!("references missing proxy/provider/group {name}; dropped"));
                }
                ok
            }).cloned().collect();
            *members = valid;
        }
    }
}
```

- [ ] **Step 4: 在 multi_profile_merge 末尾调用**

在 `(base, all_conflicts)` 返回前插入：

```rust
verify_reference_integrity(&mut base, &mut all_conflicts);
```

- [ ] **Step 5: 跑测试 + 回归**

Run: `cd src-tauri && cargo test --lib enhance::multi_merge`
Expected: 新增测试 PASS，其余全绿

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/enhance/multi_merge.rs
git commit -m "feat: reference integrity check drops dangling group members"
```

---

### Task 5: deep merge（A 类自包含字段）

**Files:**
- Modify: `src-tauri/src/enhance/multi_merge.rs` — 新增 `deep_merge_field`，在 `log_top_level_key_conflicts` 中调用
- Test: 同文件 `mod tests`

**Interfaces:**
- Produces: `fn deep_merge_field(base: &mut Mapping, supp: &Mapping, field: &str, ctx: &mut MergeContext)`
- Consumes: `MergeContext`、`push_conflict`

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn dns_nested_fields_are_deep_merged() {
    let primary = mapping("dns:\n  enable: true\n  nameserver:\n    - 1.1.1.1\nproxies:\n  - name: p\n    type: ss");
    let supp = mapping("dns:\n  enable: false\n  fallback:\n    - 8.8.8.8\nproxies:\n  - name: p2\n    type: vmess");
    let (result, _conflicts) = multi_profile_merge(&[primary, supp], &["primary", "supp"]);

    let dns = result.get("dns").unwrap().as_mapping().unwrap();
    // enable 是叶子，primary 赢
    assert_eq!(dns.get("enable").unwrap().as_bool().unwrap(), true);
    // nameserver 是 primary 独有，保留
    assert_eq!(dns.get("nameserver").unwrap().as_sequence().unwrap().len(), 1);
    // fallback 是 supp 独有，合入
    assert_eq!(dns.get("fallback").unwrap().as_sequence().unwrap().len(), 1);
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd src-tauri && cargo test --lib dns_nested_fields_are_deep_merged`
Expected: FAIL（当前 `dns` 整块 primary 赢，fallback 被丢）

- [ ] **Step 3: 实现 deep_merge_field**

```rust
const DEEP_MERGE_FIELDS: &[&str] = &["dns", "tun", "hosts", "profile"];

fn deep_merge_mapping(base: &mut Mapping, supp: &Mapping, ctx: &mut MergeContext) {
    for (k, v) in supp {
        match (base.get_mut(k), v) {
            (Some(Value::Mapping(b)), Value::Mapping(s)) => deep_merge_mapping(b, s, ctx),
            (Some(existing), _) if existing == v => {}
            (Some(_), _) => {
                push_conflict(&mut ctx.conflicts, "top-level", k.as_str().unwrap_or("<non-string>"), &ctx.supp_name, format!("already exists in {}; kept primary value", ctx.primary_name));
            }
            (None, _) => {
                base.insert(k.clone(), v.clone());
            }
        }
    }
}

fn deep_merge_field(base: &mut Mapping, supp: &Mapping, field: &str, ctx: &mut MergeContext) {
    match (base.get_mut(field), supp.get(field)) {
        (Some(Value::Mapping(b)), Some(Value::Mapping(s))) => deep_merge_mapping(b, s, ctx),
        _ => {}
    }
}
```

- [ ] **Step 4: 在 log_top_level_key_conflicts 中接入**

将 `log_top_level_key_conflicts`（Task 2 重构后签名 `fn log_top_level_key_conflicts(ctx: &mut MergeContext, base: &mut Mapping, supp: &Mapping)`）的遍历改为完整如下（注意 `base` 需从 `&Mapping` 改为 `&mut Mapping`）：

```rust
fn log_top_level_key_conflicts(ctx: &mut MergeContext, base: &mut Mapping, supp: &Mapping) {
    const MERGED_FIELDS: [&str; 5] = ["proxies", "proxy-providers", "proxy-groups", "rules", "rule-providers"];

    for (key, value) in supp {
        let Some(key_str) = key.as_str() else {
            continue;
        };
        if MERGED_FIELDS.contains(&key_str) {
            continue;
        }
        if DEEP_MERGE_FIELDS.contains(&key_str) {
            deep_merge_field(base, supp, key_str, ctx);
            continue;
        }
        match base.get(key) {
            Some(existing) if existing == value => {}
            _ => {
                push_conflict(&mut ctx.conflicts, "top-level", key_str, &ctx.supp_name, format!("already exists in {}; kept primary value", ctx.primary_name));
            }
        }
    }
}
```

- [ ] **Step 5: 跑测试 + 回归**

Run: `cd src-tauri && cargo test --lib enhance::multi_merge`
Expected: 新增测试 PASS，现有测试全绿（注意 `top_level_key_conflicts_are_logged_and_primary_wins` 测试用了 `dns` 字段，若其断言依赖「dns 整块 primary 赢」需确认：该测试的 supp `dns.enable=false` 与 primary `dns.enable=true` 冲突，deep merge 后叶子 primary 赢，结果仍是 `enable=true`，断言不变）

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/enhance/multi_merge.rs
git commit -m "feat: deep merge for self-contained dns/tun/hosts/profile fields"
```

---

## Self-Review 记录

- Spec 覆盖：阶段 1→Task 1；阶段 2 三层→Task 2/3/4；阶段 3→Task 5。无遗漏。
- 类型一致性：`MergeContext`（Task 2 定义）在 Task 3/4/5 复用，字段名 `primary_name/supp_name/conflicts/renames` 一致；`renamed_target` 返回 `Cow<str>`，`strip_name` 返回 `Mapping`，消费点签名匹配。
- 已知行为变化：`duplicate_proxy_name_logged_as_conflict` 由「丢弃」改「重命名」，Task 3 Step 7 已更新。
