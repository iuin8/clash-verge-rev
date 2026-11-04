use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_yaml_ng::{Mapping, Value};

use clash_verge_logging::{Type, logging};

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ConflictEntry {
    pub field: String,
    pub name: String,
    pub source: String,
    pub reason: String,
}

fn push_conflict(
    conflicts: &mut Vec<ConflictEntry>,
    field: &str,
    name: impl Into<String>,
    source: &str,
    reason: impl Into<String>,
) {
    conflicts.push(ConflictEntry {
        field: field.into(),
        name: name.into(),
        source: source.into(),
        reason: reason.into(),
    });
}

/// 跨合并步骤共享的状态：renames 在 proxies/providers 步骤产出、在 groups/rules 步骤消费。
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
    merge_rule_providers,
    merge_proxy_groups,
    merge_rules,
    log_top_level_key_conflicts,
];

/// 返回去掉 `name` 键的浅拷贝，用于「重名但定义是否相同」的比较。
fn strip_name(mapping: &Mapping) -> Mapping {
    mapping
        .iter()
        .filter(|(k, _)| k.as_str() != Some("name"))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// 命中 `renames` 则返回重命名后的名字，否则原样返回。
fn renamed_target<'a>(renames: &'a HashMap<String, String>, name: &'a str) -> Cow<'a, str> {
    if let Some(new_name) = renames.get(name) {
        Cow::Borrowed(new_name.as_str())
    } else {
        Cow::Borrowed(name)
    }
}

/// 重写 `RULE-SET,<name>,...` 中的 rule-provider 名（若被重命名）。
fn rewrite_rule_set(rule: &str, renames: &HashMap<String, String>) -> String {
    if let Some(rest) = rule.strip_prefix("RULE-SET,")
        && let Some(provider_name) = rest.split(',').next()
    {
        return format!(
            "RULE-SET,{}{}",
            renamed_target(renames, provider_name),
            &rest[provider_name.len()..]
        );
    }
    rule.to_owned()
}

/// 重写 group 自身的 `proxies`/`use` 成员引用（用于全新 group，无需与 primary 合并）。
fn rewrite_group_members(group: &mut Value, renames: &HashMap<String, String>) {
    let Some(group_map) = group.as_mapping_mut() else {
        return;
    };
    for field in ["proxies", "use"] {
        let Some(Value::Sequence(members)) = group_map.get_mut(field) else {
            continue;
        };
        for member in members.iter_mut() {
            if let Some(name) = member.as_str() {
                let resolved = renamed_target(renames, name);
                if resolved != name {
                    *member = Value::String(resolved.into_owned());
                }
            }
        }
    }
}

fn ensure_sequence_field<'a>(
    base: &'a mut Mapping,
    field: &str,
    primary_name: &str,
    conflicts: &mut Vec<ConflictEntry>,
) -> &'a mut Vec<Value> {
    let base_value = base
        .entry(Value::String(field.into()))
        .or_insert_with(|| Value::Sequence(vec![]));
    if !matches!(base_value, Value::Sequence(_)) {
        push_conflict(
            conflicts,
            field,
            "<top-level>",
            primary_name,
            "primary field has invalid shape; replaced during merge",
        );
        *base_value = Value::Sequence(vec![]);
    }

    let Value::Sequence(sequence) = base_value else {
        unreachable!("sequence field should be normalized before merge")
    };
    sequence
}

fn ensure_mapping_field<'a>(
    base: &'a mut Mapping,
    field: &str,
    primary_name: &str,
    conflicts: &mut Vec<ConflictEntry>,
) -> &'a mut Mapping {
    let base_value = base
        .entry(Value::String(field.into()))
        .or_insert_with(|| Value::Mapping(Mapping::new()));
    if !matches!(base_value, Value::Mapping(_)) {
        push_conflict(
            conflicts,
            field,
            "<top-level>",
            primary_name,
            "primary field has invalid shape; replaced during merge",
        );
        *base_value = Value::Mapping(Mapping::new());
    }

    let Value::Mapping(mapping) = base_value else {
        unreachable!("mapping field should be normalized before merge")
    };
    mapping
}

fn merge_proxies(ctx: &mut MergeContext, base: &mut Mapping, supp: &Mapping) {
    if let Some(Value::Sequence(supp_proxies)) = supp.get("proxies") {
        if supp_proxies.is_empty() {
            return;
        }
        let base_seq = ensure_sequence_field(base, "proxies", &ctx.primary_name, &mut ctx.conflicts);
        let mut existing: HashMap<String, Mapping> = base_seq
            .iter()
            .filter_map(|p| p.as_mapping())
            .filter_map(|m| m.get("name").and_then(Value::as_str).map(|n| (n.to_owned(), m.clone())))
            .collect();

        let mut to_prepend: Vec<Value> = vec![];
        for proxy in supp_proxies {
            let Some(proxy_map) = proxy.as_mapping() else { continue };
            let Some(proxy_name) = proxy_map.get("name").and_then(Value::as_str) else {
                continue;
            };
            let existing_def = existing.get(proxy_name).cloned();
            match existing_def {
                None => {
                    existing.insert(proxy_name.to_owned(), proxy_map.clone());
                    to_prepend.push(proxy.clone());
                }
                Some(primary_def) => {
                    if strip_name(&primary_def) == strip_name(proxy_map) {
                        continue; // 真重复，安全丢弃
                    }
                    // 定义不同：重命名保底，避免 group 静默指向 primary 的同名但不同定义节点
                    let new_name = format!("{proxy_name} [{}]", ctx.supp_name);
                    ctx.renames.insert(proxy_name.to_owned(), new_name.clone());
                    existing.insert(new_name.clone(), proxy_map.clone());
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

fn filtered_group_settings(mapping: &Mapping) -> Mapping {
    mapping
        .iter()
        .filter(|(key, _)| {
            let field = key.as_str();
            field != Some("proxies") && field != Some("use")
        })
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn merge_group_member_list(
    renames: &HashMap<String, String>,
    existing_map: &mut Mapping,
    incoming_map: &Mapping,
    field: &str,
    group_name: &str,
    primary_name: &str,
    conflicts: &mut Vec<ConflictEntry>,
) {
    let supp_members: Vec<Value> = incoming_map
        .get(field)
        .and_then(|v| v.as_sequence())
        .cloned()
        .unwrap_or_default();
    if supp_members.is_empty() {
        return;
    }

    let existing_members = existing_map
        .entry(Value::String(field.into()))
        .or_insert_with(|| Value::Sequence(vec![]));
    if !matches!(existing_members, Value::Sequence(_)) {
        push_conflict(
            conflicts,
            "proxy-groups",
            group_name,
            primary_name,
            format!("existing group has invalid `{field}` members; replaced during merge"),
        );
        *existing_members = Value::Sequence(vec![]);
    }

    let Value::Sequence(existing_seq) = existing_members else {
        unreachable!("group member field should be normalized before merge")
    };
    let mut seen_members: HashSet<String> = existing_seq
        .iter()
        .filter_map(|v| v.as_str().map(String::from))
        .collect();
    let mut members_to_prepend: Vec<Value> = vec![];
    for member in &supp_members {
        let member_name = member.as_str().unwrap_or("");
        let resolved = renamed_target(renames, member_name);
        if seen_members.insert(resolved.to_string()) {
            members_to_prepend.push(Value::String(resolved.into_owned()));
        }
    }
    for item in members_to_prepend.into_iter().rev() {
        existing_seq.insert(0, item);
    }
}

fn merge_group_members(
    renames: &HashMap<String, String>,
    existing_group: &mut Value,
    incoming_group: &Value,
    primary_name: &str,
    conflicts: &mut Vec<ConflictEntry>,
) {
    let Some(existing_map) = existing_group.as_mapping_mut() else {
        return;
    };
    let Some(incoming_map) = incoming_group.as_mapping() else {
        return;
    };

    let group_name = existing_map
        .get("name")
        .and_then(Value::as_str)
        .or_else(|| incoming_map.get("name").and_then(Value::as_str))
        .unwrap_or("")
        .to_string();

    merge_group_member_list(
        renames,
        existing_map,
        incoming_map,
        "proxies",
        &group_name,
        primary_name,
        conflicts,
    );
    merge_group_member_list(
        renames,
        existing_map,
        incoming_map,
        "use",
        &group_name,
        primary_name,
        conflicts,
    );
}

fn has_non_empty_group_members(mapping: &Mapping, field: &str) -> bool {
    mapping
        .get(field)
        .and_then(Value::as_sequence)
        .is_some_and(|members| !members.is_empty())
}

fn has_invalid_group_members(mapping: &Mapping, field: &str) -> bool {
    matches!(mapping.get(field), Some(value) if !matches!(value, Value::Sequence(_)))
}

fn group_member_mode(mapping: &Mapping) -> &'static str {
    let has_proxies = has_non_empty_group_members(mapping, "proxies");
    let has_use = has_non_empty_group_members(mapping, "use");

    match (has_proxies, has_use) {
        (true, false) => "proxies",
        (false, true) => "use",
        (true, true) => "mixed",
        (false, false) => "none",
    }
}

fn can_merge_group_members(existing_group: &Value, incoming_group: &Value) -> bool {
    let Some(existing_map) = existing_group.as_mapping() else {
        return false;
    };
    let Some(incoming_map) = incoming_group.as_mapping() else {
        return false;
    };

    let incoming_contributes_members =
        has_non_empty_group_members(incoming_map, "proxies") || has_non_empty_group_members(incoming_map, "use");
    let leaves_invalid_existing_members_untouched = ["proxies", "use"].into_iter().any(|field| {
        has_invalid_group_members(existing_map, field) && !has_non_empty_group_members(incoming_map, field)
    });
    if incoming_contributes_members && leaves_invalid_existing_members_untouched {
        return false;
    }

    let existing_mode = group_member_mode(existing_map);
    let incoming_mode = group_member_mode(incoming_map);

    existing_mode == incoming_mode || existing_mode == "none" || incoming_mode == "none"
}

fn merge_proxy_groups(ctx: &mut MergeContext, base: &mut Mapping, supp: &Mapping) {
    if let Some(Value::Sequence(supp_groups)) = supp.get("proxy-groups") {
        if supp_groups.is_empty() {
            return;
        }
        let base_seq = ensure_sequence_field(base, "proxy-groups", &ctx.primary_name, &mut ctx.conflicts);
        let mut existing_group_names: HashSet<String> = base_seq
            .iter()
            .filter_map(|group| {
                group
                    .as_mapping()
                    .and_then(|m| m.get("name"))
                    .and_then(|v| v.as_str())
                    .map(String::from)
            })
            .collect();

        let mut groups_to_prepend: Vec<Value> = vec![];
        for group in supp_groups {
            let group_name = group
                .as_mapping()
                .and_then(|m| m.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if existing_group_names.insert(group_name.to_string()) {
                let mut new_group = group.clone();
                rewrite_group_members(&mut new_group, &ctx.renames);
                groups_to_prepend.push(new_group);
                continue;
            }

            let existing_group = base_seq.iter_mut().find(|existing| {
                existing
                    .as_mapping()
                    .and_then(|m| m.get("name"))
                    .and_then(|v| v.as_str())
                    == Some(group_name)
            });
            let pending_group = groups_to_prepend.iter_mut().find(|existing| {
                existing
                    .as_mapping()
                    .and_then(|m| m.get("name"))
                    .and_then(|v| v.as_str())
                    == Some(group_name)
            });

            let Some(target_group) = existing_group.or(pending_group) else {
                continue;
            };

            let existing_settings = target_group
                .as_mapping()
                .map(filtered_group_settings)
                .unwrap_or_default();
            let incoming_settings = group.as_mapping().map(filtered_group_settings).unwrap_or_default();
            if existing_settings != incoming_settings || !can_merge_group_members(target_group, group) {
                push_conflict(
                    &mut ctx.conflicts,
                    "proxy-groups",
                    group_name,
                    &ctx.supp_name,
                    format!(
                        "already exists in {} with incompatible settings; kept existing group",
                        ctx.primary_name
                    ),
                );
                continue;
            }

            merge_group_members(&ctx.renames, target_group, group, &ctx.primary_name, &mut ctx.conflicts);
        }

        for item in groups_to_prepend.into_iter().rev() {
            base_seq.insert(0, item);
        }
    }
}

fn merge_rules(ctx: &mut MergeContext, base: &mut Mapping, supp: &Mapping) {
    if let Some(Value::Sequence(supp_rules)) = supp.get("rules") {
        if supp_rules.is_empty() {
            return;
        }
        let base_seq = ensure_sequence_field(base, "rules", &ctx.primary_name, &mut ctx.conflicts);
        let mut existing_rules: HashSet<String> = base_seq
            .iter()
            .filter_map(|rule| rule.as_str().map(String::from))
            .collect();
        let mut rules_to_prepend: Vec<Value> = vec![];
        for rule in supp_rules {
            let rule_str = rule.as_str().unwrap_or("");
            let rewritten = rewrite_rule_set(rule_str, &ctx.renames);
            if existing_rules.insert(rewritten.clone()) {
                rules_to_prepend.push(Value::String(rewritten));
            }
        }
        for item in rules_to_prepend.into_iter().rev() {
            base_seq.insert(0, item);
        }
    }
}

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
                Some(_) => {
                    // 同名不同定义：重命名保底，避免 group `use` 静默指向 primary 版
                    let new_name = format!("{name_str} [{}]", ctx.supp_name);
                    ctx.renames.insert(name_str.clone(), new_name.clone());
                    base_map.insert(Value::String(new_name), value.clone());
                }
            }
        }
    }
}

fn merge_rule_providers(ctx: &mut MergeContext, base: &mut Mapping, supp: &Mapping) {
    merge_named_mapping(ctx, base, supp, "rule-providers");
}

fn merge_proxy_providers(ctx: &mut MergeContext, base: &mut Mapping, supp: &Mapping) {
    merge_named_mapping(ctx, base, supp, "proxy-providers");
}

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
            Some(_) => {
                push_conflict(
                    &mut ctx.conflicts,
                    "top-level",
                    key_str,
                    &ctx.supp_name,
                    format!("already exists in {}; kept primary value", ctx.primary_name),
                );
            }
            // FORK: the primary has no such key, so no value is kept at all. Reporting
            // "kept primary value" here was untrue and hid the fact that the key is dropped.
            None => {
                push_conflict(
                    &mut ctx.conflicts,
                    "top-level",
                    key_str,
                    &ctx.supp_name,
                    format!("not merged: only {} may define this key", ctx.primary_name),
                );
                logging!(
                    warn,
                    Type::Config,
                    "profile '{}' defines top-level '{}', which is not merged across profiles; primary '{}' does not define it, so it is ignored",
                    ctx.supp_name,
                    key_str,
                    ctx.primary_name
                );
            }
        }
    }
}

/// 合并完成后校验所有 group 的 proxies/use 成员引用是否存在于最终 config，
/// 不存在的记 conflict 并从成员中移除（降级，不阻断）。
fn verify_reference_integrity(base: &mut Mapping, conflicts: &mut Vec<ConflictEntry>) {
    let proxy_names: HashSet<String> = base
        .get("proxies")
        .and_then(Value::as_sequence)
        .into_iter()
        .flatten()
        .filter_map(|p| {
            p.as_mapping()
                .and_then(|m| m.get("name"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .collect();
    let provider_names: HashSet<String> = base
        .get("proxy-providers")
        .and_then(Value::as_mapping)
        .into_iter()
        .flat_map(|m| m.iter())
        .filter_map(|(k, _)| k.as_str().map(str::to_owned))
        .collect();
    let group_names: HashSet<String> = base
        .get("proxy-groups")
        .and_then(Value::as_sequence)
        .into_iter()
        .flatten()
        .filter_map(|g| {
            g.as_mapping()
                .and_then(|m| m.get("name"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .collect();

    let Some(Value::Sequence(groups)) = base.get_mut("proxy-groups") else {
        return;
    };
    for group in groups.iter_mut() {
        let Some(group_map) = group.as_mapping_mut() else {
            continue;
        };
        let group_name = group_map
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_default();
        for field in ["proxies", "use"] {
            let Some(Value::Sequence(members)) = group_map.get_mut(field) else {
                continue;
            };
            let valid: Vec<Value> = members
                .iter()
                .filter(|m| {
                    let Some(name) = m.as_str() else {
                        return true;
                    };
                    let ok = proxy_names.contains(name)
                        || provider_names.contains(name)
                        || group_names.contains(name)
                        || BUILTIN_POLICY_MEMBERS.contains(&name);
                    if !ok {
                        push_conflict(
                            conflicts,
                            "proxy-groups",
                            name,
                            &group_name,
                            format!("references missing proxy/provider/group {name}; dropped"),
                        );
                    }
                    ok
                })
                .cloned()
                .collect();
            *members = valid;
        }
    }
}

/// Names mihomo resolves from its built-in `proxies` map, not from a profile's
/// `proxies`/`proxy-providers` lists.
const BUILTIN_POLICY_MEMBERS: &[&str] = &["DIRECT", "REJECT", "REJECT-DROP", "COMPATIBLE", "PASS", "PASS-RULE"];

/// 自包含 mapping 字段，做递归合并（叶子 primary 赢，sequence/标量整块 primary 赢）。
const DEEP_MERGE_FIELDS: &[&str] = &["dns", "tun", "hosts", "profile"];

fn deep_merge_mapping(base: &mut Mapping, supp: &Mapping, ctx: &mut MergeContext) {
    for (k, v) in supp {
        match base.get_mut(k) {
            Some(Value::Mapping(b)) => {
                if let Value::Mapping(s) = v {
                    deep_merge_mapping(b, s, ctx);
                } else {
                    push_conflict(
                        &mut ctx.conflicts,
                        "top-level",
                        k.as_str().unwrap_or("<non-string>"),
                        &ctx.supp_name,
                        format!("already exists in {}; kept primary value", ctx.primary_name),
                    );
                }
            }
            Some(existing) => {
                if existing != v {
                    push_conflict(
                        &mut ctx.conflicts,
                        "top-level",
                        k.as_str().unwrap_or("<non-string>"),
                        &ctx.supp_name,
                        format!("already exists in {}; kept primary value", ctx.primary_name),
                    );
                }
            }
            None => {
                base.insert(k.clone(), v.clone());
            }
        }
    }
}

fn deep_merge_field(base: &mut Mapping, supp: &Mapping, field: &str, ctx: &mut MergeContext) {
    if let (Some(Value::Mapping(b)), Some(Value::Mapping(s))) = (base.get_mut(field), supp.get(field)) {
        deep_merge_mapping(b, s, ctx);
    }
}

/// Merge an ordered list of YAML configs.
/// Supplementary profiles can contribute proxies, proxy-providers, proxy-groups, rules, and rule-providers.
/// All other top-level keys come from primary (index 0).
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

    verify_reference_integrity(&mut base, &mut all_conflicts);

    (base, all_conflicts)
}

/// FORK: 最近一次配置生成得到的合并冲突快照（前端 ConflictViewer 读取）。
///
/// 与上游 `DISCARDED_KEYS_NOTICE` 的一次性 take 不同，这里保留快照：前端会重复拉取
/// 用来渲染冲突徽标与详情，take 掉第二次就空了。
static MERGE_CONFLICTS: parking_lot::Mutex<Vec<ConflictEntry>> = parking_lot::Mutex::new(Vec::new());

pub fn publish_conflicts(entries: Vec<ConflictEntry>) {
    *MERGE_CONFLICTS.lock() = entries;
}

pub fn conflicts() -> Vec<ConflictEntry> {
    MERGE_CONFLICTS.lock().clone()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
#[path = "multi_merge_tests.rs"]
mod tests;
