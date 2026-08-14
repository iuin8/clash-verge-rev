use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_yaml_ng::{Mapping, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
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
            _ => {
                push_conflict(
                    &mut ctx.conflicts,
                    "top-level",
                    key_str,
                    &ctx.supp_name,
                    format!("already exists in {}; kept primary value", ctx.primary_name),
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
                        || name == "DIRECT";
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const LOCAL_LAN_PRIMARY_YAML: &str =
        include_str!("../../../docs/repro/local-lan-rule-set/10-merge-primary-valid.yaml");
    const LOCAL_LAN_SUPPLEMENT_YAML: &str =
        include_str!("../../../docs/repro/local-lan-rule-set/11-merge-supplement-valid.yaml");
    const LOCAL_LAN_BROKEN_BASELINE_YAML: &str =
        include_str!("../../../docs/repro/local-lan-rule-set/12-expected-broken-merged-output.yaml");

    fn mapping(yaml: &str) -> Mapping {
        serde_yaml_ng::from_str(yaml).expect("test YAML is valid")
    }

    /// 注入占位符 proxy/provider 定义，让 group 测试里的 `a`/`b`/`provider-a` 引用真实存在，
    /// 避免层 3 引用完整性校验把它们当作无效引用移除。
    fn group_mapping(yaml: &str) -> Mapping {
        mapping(&format!(
            "proxies:\n  - name: a\n    type: ss\n  - name: b\n    type: ss\nproxy-providers:\n  provider-a:\n    type: http\n    url: https://a.example\n  provider-b:\n    type: http\n    url: https://b.example\n{yaml}"
        ))
    }

    #[test]
    fn dns_nested_fields_are_deep_merged() {
        let primary =
            mapping("dns:\n  enable: true\n  nameserver:\n    - 1.1.1.1\nproxies:\n  - name: p\n    type: ss");
        let supp =
            mapping("dns:\n  enable: false\n  fallback:\n    - 8.8.8.8\nproxies:\n  - name: p2\n    type: vmess");
        let (result, _conflicts) = multi_profile_merge(&[primary, supp], &["primary", "supp"]);

        let dns = result.get("dns").unwrap().as_mapping().unwrap();
        assert!(dns.get("enable").unwrap().as_bool().unwrap());
        assert_eq!(dns.get("nameserver").unwrap().as_sequence().unwrap().len(), 1);
        assert_eq!(dns.get("fallback").unwrap().as_sequence().unwrap().len(), 1);
    }

    #[test]
    fn duplicate_proxy_name_with_different_definition_is_renamed_not_dropped() {
        let primary = mapping("proxies:\n  - name: dup\n    type: ss\n    server: a.com");
        let supp = mapping(
            "proxies:\n  - name: dup\n    type: vmess\n    server: b.com\nproxy-groups:\n  - name: g\n    type: select\n    proxies:\n      - dup",
        );
        let (result, _conflicts) = multi_profile_merge(&[primary, supp], &["primary", "supp"]);

        let proxies = result.get("proxies").unwrap().as_sequence().unwrap();
        assert_eq!(proxies.len(), 2);
        assert_eq!(
            proxies[0].as_mapping().unwrap().get("name").unwrap().as_str().unwrap(),
            "dup [supp]"
        );

        let groups = result.get("proxy-groups").unwrap().as_sequence().unwrap();
        let members = groups[0]
            .as_mapping()
            .unwrap()
            .get("proxies")
            .unwrap()
            .as_sequence()
            .unwrap();
        assert!(members.iter().any(|m| m.as_str() == Some("dup [supp]")));
    }

    #[test]
    fn empty_configs_returns_empty_mapping() {
        let (result, conflicts) = multi_profile_merge(&[], &[]);
        assert!(result.is_empty());
        assert!(conflicts.is_empty());
    }

    #[test]
    fn single_config_returned_as_is() {
        let cfg = mapping("proxies:\n  - name: px1\n    type: ss");
        let (result, conflicts) = multi_profile_merge(std::slice::from_ref(&cfg), &["primary"]);
        assert_eq!(result, cfg);
        assert!(conflicts.is_empty());
    }

    #[test]
    fn proxies_from_supplementary_are_prepended() {
        let primary = mapping("proxies:\n  - name: px-primary\n    type: ss");
        let supp = mapping("proxies:\n  - name: px-supp\n    type: vmess");
        let (result, conflicts) = multi_profile_merge(&[primary, supp], &["primary", "supp"]);
        assert!(conflicts.is_empty());
        let proxies = result.get("proxies").unwrap().as_sequence().unwrap();
        assert_eq!(proxies.len(), 2);
        assert_eq!(
            proxies[0].as_mapping().unwrap().get("name").unwrap().as_str().unwrap(),
            "px-supp"
        );
    }

    #[test]
    fn duplicate_proxy_name_with_same_definition_is_dropped() {
        let primary = mapping("proxies:\n  - name: dup\n    type: ss");
        let supp = mapping("proxies:\n  - name: dup\n    type: ss");
        let (result, _conflicts) = multi_profile_merge(&[primary, supp], &["primary", "supp"]);
        let proxies = result.get("proxies").unwrap().as_sequence().unwrap();
        assert_eq!(proxies.len(), 1);
    }

    #[test]
    fn group_referencing_missing_proxy_is_dropped_with_conflict() {
        let primary = mapping(
            "proxies:\n  - name: p1\n    type: ss\nproxy-groups:\n  - name: g\n    type: select\n    proxies:\n      - p1\n      - ghost",
        );
        let supp = mapping("rules:\n  - MATCH,DIRECT");
        let (result, conflicts) = multi_profile_merge(&[primary, supp], &["primary", "supp"]);

        assert!(conflicts.iter().any(|c| c.name == "ghost"));
        let groups = result.get("proxy-groups").unwrap().as_sequence().unwrap();
        let members = groups[0]
            .as_mapping()
            .unwrap()
            .get("proxies")
            .unwrap()
            .as_sequence()
            .unwrap();
        assert!(!members.iter().any(|m| m.as_str() == Some("ghost")));
    }

    #[test]
    fn rules_from_supplementary_are_prepended_deduped() {
        let primary = mapping("rules:\n  - DOMAIN,example.com,DIRECT");
        let supp = mapping("rules:\n  - DOMAIN,example.com,DIRECT\n  - DOMAIN,other.com,PROXY");
        let (result, _conflicts) = multi_profile_merge(&[primary, supp], &["primary", "supp"]);
        let rules = result.get("rules").unwrap().as_sequence().unwrap();
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].as_str().unwrap(), "DOMAIN,other.com,PROXY");
    }

    #[test]
    fn top_level_key_conflicts_are_logged_and_primary_wins() {
        let primary = mapping("dns:\n  enable: true\nproxies:\n  - name: p1\n    type: ss");
        let supp = mapping("dns:\n  enable: false\ninterface-name: utun9\nproxies:\n  - name: p2\n    type: vmess");
        let (result, conflicts) = multi_profile_merge(&[primary, supp], &["primary", "supp"]);
        let dns = result.get("dns").unwrap().as_mapping().unwrap();
        assert!(dns.get("enable").unwrap().as_bool().unwrap());
        assert!(result.get("interface-name").is_none());
        assert_eq!(conflicts.len(), 2);
        assert_eq!(conflicts[0].field, "top-level");
        assert_eq!(conflicts[0].name, "enable");
        assert_eq!(conflicts[0].source, "supp");
        assert_eq!(conflicts[1].field, "top-level");
        assert_eq!(conflicts[1].name, "interface-name");
        assert_eq!(conflicts[1].source, "supp");
    }

    #[test]
    fn supplementary_rule_providers_are_merged_into_result() {
        let primary = mapping("rules:\n  - MATCH,DIRECT");
        let supp = mapping(
            "rule-providers:\n  Local-LAN:\n    type: inline\n    behavior: classical\n    payload:\n      - IP-CIDR,192.168.0.0/16,DIRECT\nrules:\n  - RULE-SET,Local-LAN,DIRECT",
        );

        let (result, conflicts) = multi_profile_merge(&[primary, supp], &["primary", "supp"]);

        assert!(conflicts.is_empty());
        let rule_providers = result.get("rule-providers").unwrap().as_mapping().unwrap();
        assert!(rule_providers.get("Local-LAN").is_some());
    }

    #[test]
    fn local_lan_merge_repro_fixtures_keep_rule_provider() {
        let primary = mapping(LOCAL_LAN_PRIMARY_YAML);
        let supp = mapping(LOCAL_LAN_SUPPLEMENT_YAML);

        let (result, conflicts) = multi_profile_merge(&[primary, supp], &["primary", "supp"]);

        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].field, "top-level");
        assert_eq!(conflicts[0].name, "mixed-port");
        let rule_providers = result.get("rule-providers").unwrap().as_mapping().unwrap();
        let local_lan = rule_providers.get("Local-LAN").unwrap().as_mapping().unwrap();
        let payload = local_lan.get("payload").unwrap().as_sequence().unwrap();
        let rules = result.get("rules").unwrap().as_sequence().unwrap();

        assert_eq!(payload.len(), 2);
        assert_eq!(rules[0].as_str().unwrap(), "IP-CIDR,10.0.0.0/8,DIRECT");
        assert_eq!(rules[1].as_str().unwrap(), "RULE-SET,Local-LAN,DIRECT");
        assert_eq!(rules[2].as_str().unwrap(), "MATCH,DIRECT");
    }

    #[test]
    fn local_lan_merge_repro_reverse_order_keeps_primary_definition() {
        let primary = mapping(LOCAL_LAN_SUPPLEMENT_YAML);
        let supp = mapping(LOCAL_LAN_PRIMARY_YAML);

        let (result, conflicts) = multi_profile_merge(&[primary, supp], &["primary", "supp"]);

        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].field, "top-level");
        assert_eq!(conflicts[0].name, "mixed-port");
        let rule_providers = result.get("rule-providers").unwrap().as_mapping().unwrap();
        assert!(rule_providers.get("Local-LAN").is_some());
    }

    #[test]
    fn broken_merge_repro_fixture_stays_documented_as_broken_baseline() {
        let broken = mapping(LOCAL_LAN_BROKEN_BASELINE_YAML);

        assert!(broken.get("rule-providers").is_none());
        let rules = broken.get("rules").unwrap().as_sequence().unwrap();
        assert_eq!(rules[1].as_str().unwrap(), "RULE-SET,Local-LAN,DIRECT");
    }

    #[test]
    fn identical_rule_provider_definitions_are_deduped_without_conflict() {
        let primary = mapping(
            "rule-providers:\n  Local-LAN:\n    type: inline\n    behavior: classical\n    payload:\n      - IP-CIDR,192.168.0.0/16,DIRECT",
        );
        let supp = mapping(
            "rule-providers:\n  Local-LAN:\n    type: inline\n    behavior: classical\n    payload:\n      - IP-CIDR,192.168.0.0/16,DIRECT",
        );

        let (result, conflicts) = multi_profile_merge(&[primary, supp], &["primary", "supp"]);

        assert!(conflicts.is_empty());
        let rule_providers = result.get("rule-providers").unwrap().as_mapping().unwrap();
        assert_eq!(rule_providers.len(), 1);
    }

    #[test]
    fn conflicting_rule_provider_definitions_are_renamed() {
        let primary = mapping(
            "rule-providers:\n  Local-LAN:\n    type: inline\n    behavior: classical\n    payload:\n      - IP-CIDR,192.168.0.0/16,DIRECT",
        );
        let supp = mapping(
            "rule-providers:\n  Local-LAN:\n    type: inline\n    behavior: classical\n    payload:\n      - IP-CIDR,10.0.0.0/8,DIRECT",
        );

        let (result, conflicts) = multi_profile_merge(&[primary, supp], &["primary", "supp"]);

        assert!(conflicts.is_empty());
        let rule_providers = result.get("rule-providers").unwrap().as_mapping().unwrap();
        assert_eq!(rule_providers.len(), 2);
        assert!(rule_providers.get("Local-LAN").is_some());
        let renamed = rule_providers.get("Local-LAN [supp]").unwrap().as_mapping().unwrap();
        let payload = renamed.get("payload").unwrap().as_sequence().unwrap();
        assert_eq!(payload[0].as_str().unwrap(), "IP-CIDR,10.0.0.0/8,DIRECT");
    }

    #[test]
    fn duplicate_rules_inside_same_supplement_are_deduped() {
        let primary = mapping("rules:\n  - MATCH,DIRECT");
        let supp = mapping("rules:\n  - DOMAIN,dup.example,DIRECT\n  - DOMAIN,dup.example,DIRECT");

        let (result, _) = multi_profile_merge(&[primary, supp], &["primary", "supp"]);

        let rules = result.get("rules").unwrap().as_sequence().unwrap();
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].as_str().unwrap(), "DOMAIN,dup.example,DIRECT");
        assert_eq!(rules[1].as_str().unwrap(), "MATCH,DIRECT");
    }

    #[test]
    fn duplicate_proxies_inside_same_supplement_are_deduped() {
        let primary = mapping("proxies: []");
        let supp = mapping("proxies:\n  - name: dup\n    type: ss\n  - name: dup\n    type: ss");

        let (result, conflicts) = multi_profile_merge(&[primary, supp], &["primary", "supp"]);

        let proxies = result.get("proxies").unwrap().as_sequence().unwrap();
        assert_eq!(proxies.len(), 1);
        assert!(conflicts.is_empty());
    }

    #[test]
    fn duplicate_proxy_groups_inside_same_supplement_are_deduped() {
        let primary = group_mapping("proxy-groups: []");
        let supp = group_mapping(
            "proxy-groups:\n  - name: auto\n    type: select\n    proxies:\n      - a\n  - name: auto\n    type: select\n    proxies:\n      - a",
        );

        let (result, conflicts) = multi_profile_merge(&[primary, supp], &["primary", "supp"]);

        let groups = result.get("proxy-groups").unwrap().as_sequence().unwrap();
        assert_eq!(groups.len(), 1);
        assert!(conflicts.is_empty());
    }

    #[test]
    fn duplicate_proxy_group_settings_keep_existing_members() {
        let primary = group_mapping(
            "proxy-groups:\n  - name: auto\n    type: select\n    url: https://a.example/test\n    proxies:\n      - a",
        );
        let supp = group_mapping(
            "proxy-groups:\n  - name: auto\n    type: url-test\n    url: https://b.example/test\n    proxies:\n      - b",
        );

        let (result, conflicts) = multi_profile_merge(&[primary, supp], &["primary", "supp"]);

        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].field, "proxy-groups");
        let groups = result.get("proxy-groups").unwrap().as_sequence().unwrap();
        let proxies = groups[0]
            .as_mapping()
            .unwrap()
            .get("proxies")
            .unwrap()
            .as_sequence()
            .unwrap();
        assert_eq!(proxies.len(), 1);
        assert_eq!(proxies[0].as_str().unwrap(), "a");
    }

    #[test]
    fn malformed_primary_proxies_are_replaced_during_merge() {
        let primary = mapping("proxies: invalid");
        let supp = mapping("proxies:\n  - name: px-supp\n    type: ss");

        let (result, conflicts) = multi_profile_merge(&[primary, supp], &["primary", "supp"]);

        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].field, "proxies");
        let proxies = result.get("proxies").unwrap().as_sequence().unwrap();
        assert_eq!(proxies.len(), 1);
        assert_eq!(
            proxies[0].as_mapping().unwrap().get("name").unwrap().as_str().unwrap(),
            "px-supp"
        );
    }

    #[test]
    fn malformed_primary_proxies_remain_unchanged_without_supplement() {
        let primary = mapping("proxies: invalid");
        let expected = primary.clone();

        let (result, conflicts) = multi_profile_merge(&[primary], &["primary"]);

        assert!(conflicts.is_empty());
        assert_eq!(result, expected);
    }

    #[test]
    fn empty_primary_proxy_group_members_can_be_filled_by_supplement() {
        let primary = group_mapping("proxy-groups:\n  - name: auto\n    type: select\n    proxies: []");
        let supp = group_mapping("proxy-groups:\n  - name: auto\n    type: select\n    proxies:\n      - a");

        let (result, conflicts) = multi_profile_merge(&[primary, supp], &["primary", "supp"]);

        assert!(conflicts.is_empty());
        let groups = result.get("proxy-groups").unwrap().as_sequence().unwrap();
        let proxies = groups[0]
            .as_mapping()
            .unwrap()
            .get("proxies")
            .unwrap()
            .as_sequence()
            .unwrap();
        assert_eq!(proxies.len(), 1);
        assert_eq!(proxies[0].as_str().unwrap(), "a");
    }

    #[test]
    fn empty_primary_proxy_group_use_members_can_be_filled_by_supplement() {
        let primary = group_mapping("proxy-groups:\n  - name: provider-group\n    type: select\n    use: []");
        let supp =
            group_mapping("proxy-groups:\n  - name: provider-group\n    type: select\n    use:\n      - provider-a");

        let (result, conflicts) = multi_profile_merge(&[primary, supp], &["primary", "supp"]);

        assert!(conflicts.is_empty());
        let groups = result.get("proxy-groups").unwrap().as_sequence().unwrap();
        let providers = groups[0]
            .as_mapping()
            .unwrap()
            .get("use")
            .unwrap()
            .as_sequence()
            .unwrap();
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].as_str().unwrap(), "provider-a");
    }

    #[test]
    fn malformed_primary_rules_are_replaced_during_merge() {
        let primary = mapping("rules: invalid");
        let supp = mapping("rules:\n  - MATCH,DIRECT");

        let (result, conflicts) = multi_profile_merge(&[primary, supp], &["primary", "supp"]);

        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].field, "rules");
        let rules = result.get("rules").unwrap().as_sequence().unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].as_str().unwrap(), "MATCH,DIRECT");
    }

    #[test]
    fn malformed_primary_rules_remain_unchanged_without_supplement() {
        let primary = mapping("rules: invalid");
        let expected = primary.clone();

        let (result, conflicts) = multi_profile_merge(&[primary], &["primary"]);

        assert!(conflicts.is_empty());
        assert_eq!(result, expected);
    }

    #[test]
    fn malformed_primary_rules_remain_unchanged_when_supplement_rules_are_empty() {
        let primary = mapping("rules: invalid");
        let expected = primary.clone();
        let supp = mapping("rules: []");

        let (result, conflicts) = multi_profile_merge(&[primary, supp], &["primary", "supp"]);

        assert!(conflicts.is_empty());
        assert_eq!(result, expected);
    }

    #[test]
    fn malformed_primary_proxy_groups_are_replaced_during_merge() {
        let primary = group_mapping("proxy-groups: invalid");
        let supp = group_mapping("proxy-groups:\n  - name: auto\n    type: select\n    proxies:\n      - a");

        let (result, conflicts) = multi_profile_merge(&[primary, supp], &["primary", "supp"]);

        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].field, "proxy-groups");
        let groups = result.get("proxy-groups").unwrap().as_sequence().unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(
            groups[0].as_mapping().unwrap().get("name").unwrap().as_str().unwrap(),
            "auto"
        );
    }

    #[test]
    fn malformed_primary_proxy_groups_remain_unchanged_without_supplement() {
        let primary = mapping("proxy-groups: invalid");
        let expected = primary.clone();

        let (result, conflicts) = multi_profile_merge(&[primary], &["primary"]);

        assert!(conflicts.is_empty());
        assert_eq!(result, expected);
    }

    #[test]
    fn malformed_primary_proxy_group_members_are_repaired_during_merge() {
        let primary = group_mapping("proxy-groups:\n  - name: auto\n    type: select\n    proxies: invalid");
        let supp = group_mapping("proxy-groups:\n  - name: auto\n    type: select\n    proxies:\n      - a");

        let (result, conflicts) = multi_profile_merge(&[primary, supp], &["primary", "supp"]);

        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].field, "proxy-groups");
        let groups = result.get("proxy-groups").unwrap().as_sequence().unwrap();
        let proxies = groups[0]
            .as_mapping()
            .unwrap()
            .get("proxies")
            .unwrap()
            .as_sequence()
            .unwrap();
        assert_eq!(proxies.len(), 1);
        assert_eq!(proxies[0].as_str().unwrap(), "a");
    }

    #[test]
    fn malformed_primary_proxy_group_use_members_are_repaired_during_merge() {
        let primary = group_mapping("proxy-groups:\n  - name: provider-group\n    type: select\n    use: invalid");
        let supp =
            group_mapping("proxy-groups:\n  - name: provider-group\n    type: select\n    use:\n      - provider-a");

        let (result, conflicts) = multi_profile_merge(&[primary, supp], &["primary", "supp"]);

        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].field, "proxy-groups");
        let groups = result.get("proxy-groups").unwrap().as_sequence().unwrap();
        let providers = groups[0]
            .as_mapping()
            .unwrap()
            .get("use")
            .unwrap()
            .as_sequence()
            .unwrap();
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].as_str().unwrap(), "provider-a");
    }

    #[test]
    fn malformed_primary_proxy_group_members_remain_unchanged_when_supplement_members_are_missing() {
        let primary = mapping("proxy-groups:\n  - name: auto\n    type: select\n    proxies: invalid");
        let expected = primary.clone();
        let supp = mapping("proxy-groups:\n  - name: auto\n    type: select");

        let (result, conflicts) = multi_profile_merge(&[primary, supp], &["primary", "supp"]);

        assert!(conflicts.is_empty());
        assert_eq!(result, expected);
    }

    #[test]
    fn malformed_primary_proxy_group_members_remain_unchanged_when_supplement_members_are_empty() {
        let primary = mapping("proxy-groups:\n  - name: auto\n    type: select\n    proxies: invalid");
        let expected = primary.clone();
        let supp = mapping("proxy-groups:\n  - name: auto\n    type: select\n    proxies: []");

        let (result, conflicts) = multi_profile_merge(&[primary, supp], &["primary", "supp"]);

        assert!(conflicts.is_empty());
        assert_eq!(result, expected);
    }

    #[test]
    fn malformed_primary_proxy_group_use_members_remain_unchanged_when_supplement_use_is_missing() {
        let primary = mapping("proxy-groups:\n  - name: provider-group\n    type: select\n    use: invalid");
        let expected = primary.clone();
        let supp = mapping("proxy-groups:\n  - name: provider-group\n    type: select");

        let (result, conflicts) = multi_profile_merge(&[primary, supp], &["primary", "supp"]);

        assert!(conflicts.is_empty());
        assert_eq!(result, expected);
    }

    #[test]
    fn malformed_primary_proxy_group_use_members_remain_unchanged_when_supplement_use_is_empty() {
        let primary = mapping("proxy-groups:\n  - name: provider-group\n    type: select\n    use: invalid");
        let expected = primary.clone();
        let supp = mapping("proxy-groups:\n  - name: provider-group\n    type: select\n    use: []");

        let (result, conflicts) = multi_profile_merge(&[primary, supp], &["primary", "supp"]);

        assert!(conflicts.is_empty());
        assert_eq!(result, expected);
    }

    #[test]
    fn missing_supplementary_proxy_group_members_do_not_create_conflict() {
        let primary = group_mapping("proxy-groups:\n  - name: auto\n    type: select\n    proxies:\n      - a");
        let supp = group_mapping("proxy-groups:\n  - name: auto\n    type: select");

        let (result, conflicts) = multi_profile_merge(&[primary, supp], &["primary", "supp"]);

        assert!(conflicts.is_empty());
        let groups = result.get("proxy-groups").unwrap().as_sequence().unwrap();
        let proxies = groups[0]
            .as_mapping()
            .unwrap()
            .get("proxies")
            .unwrap()
            .as_sequence()
            .unwrap();
        assert_eq!(proxies.len(), 1);
        assert_eq!(proxies[0].as_str().unwrap(), "a");
    }

    #[test]
    fn empty_supplementary_proxy_group_members_do_not_create_conflict() {
        let primary = group_mapping("proxy-groups:\n  - name: auto\n    type: select\n    proxies:\n      - a");
        let supp = group_mapping("proxy-groups:\n  - name: auto\n    type: select\n    proxies: []");

        let (result, conflicts) = multi_profile_merge(&[primary, supp], &["primary", "supp"]);

        assert!(conflicts.is_empty());
        let groups = result.get("proxy-groups").unwrap().as_sequence().unwrap();
        let proxies = groups[0]
            .as_mapping()
            .unwrap()
            .get("proxies")
            .unwrap()
            .as_sequence()
            .unwrap();
        assert_eq!(proxies.len(), 1);
        assert_eq!(proxies[0].as_str().unwrap(), "a");
    }

    #[test]
    fn empty_supplementary_proxy_group_use_members_do_not_create_conflict() {
        let primary =
            group_mapping("proxy-groups:\n  - name: provider-group\n    type: select\n    use:\n      - provider-a");
        let supp = group_mapping("proxy-groups:\n  - name: provider-group\n    type: select\n    use: []");

        let (result, conflicts) = multi_profile_merge(&[primary, supp], &["primary", "supp"]);

        assert!(conflicts.is_empty());
        let groups = result.get("proxy-groups").unwrap().as_sequence().unwrap();
        let providers = groups[0]
            .as_mapping()
            .unwrap()
            .get("use")
            .unwrap()
            .as_sequence()
            .unwrap();
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].as_str().unwrap(), "provider-a");
    }

    #[test]
    fn duplicate_proxy_group_merge_rejects_invalid_untouched_use_members() {
        let primary = group_mapping(
            "proxy-groups:\n  - name: mixed-group\n    type: select\n    proxies:\n      - a\n    use: invalid",
        );
        let expected = primary.clone();
        let supp = group_mapping("proxy-groups:\n  - name: mixed-group\n    type: select\n    proxies:\n      - b");

        let (result, conflicts) = multi_profile_merge(&[primary, supp], &["primary", "supp"]);

        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].field, "proxy-groups");
        assert_eq!(result, expected);
    }

    #[test]
    fn empty_proxy_group_members_do_not_force_incompatible_mode_conflict() {
        let primary =
            group_mapping("proxy-groups:\n  - name: provider-group\n    type: select\n    use:\n      - provider-a");
        let supp = group_mapping("proxy-groups:\n  - name: provider-group\n    type: select\n    proxies: []");

        let (result, conflicts) = multi_profile_merge(&[primary, supp], &["primary", "supp"]);

        assert!(conflicts.is_empty());
        let groups = result.get("proxy-groups").unwrap().as_sequence().unwrap();
        let providers = groups[0]
            .as_mapping()
            .unwrap()
            .get("use")
            .unwrap()
            .as_sequence()
            .unwrap();
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].as_str().unwrap(), "provider-a");
    }

    #[test]
    fn malformed_primary_rule_providers_are_replaced_during_merge() {
        let primary = mapping("rule-providers: invalid");
        let supp = mapping(
            "rule-providers:\n  Local-LAN:\n    type: inline\n    behavior: classical\n    payload:\n      - IP-CIDR,192.168.0.0/16,DIRECT",
        );

        let (result, conflicts) = multi_profile_merge(&[primary, supp], &["primary", "supp"]);

        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].field, "rule-providers");
        let providers = result.get("rule-providers").unwrap().as_mapping().unwrap();
        assert!(providers.get("Local-LAN").is_some());
    }

    #[test]
    fn malformed_primary_rule_providers_remain_unchanged_without_supplement() {
        let primary = mapping("rule-providers: invalid");
        let expected = primary.clone();

        let (result, conflicts) = multi_profile_merge(&[primary], &["primary"]);

        assert!(conflicts.is_empty());
        assert_eq!(result, expected);
    }

    #[test]
    fn malformed_primary_rule_providers_remain_unchanged_when_supplement_rule_providers_are_empty() {
        let primary = mapping("rule-providers: invalid");
        let expected = primary.clone();
        let supp = mapping("rule-providers: {}");

        let (result, conflicts) = multi_profile_merge(&[primary, supp], &["primary", "supp"]);

        assert!(conflicts.is_empty());
        assert_eq!(result, expected);
    }

    #[test]
    fn malformed_primary_proxy_providers_are_replaced_during_merge() {
        let primary = mapping("proxy-providers: invalid");
        let supp = mapping(
            "proxy-providers:\n  provider-b:\n    type: http\n    url: https://example.com/b.yaml\n    path: ./provider-b.yaml\n    interval: 3600",
        );

        let (result, conflicts) = multi_profile_merge(&[primary, supp], &["primary", "supp"]);

        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].field, "proxy-providers");
        let providers = result.get("proxy-providers").unwrap().as_mapping().unwrap();
        assert!(providers.get("provider-b").is_some());
    }

    #[test]
    fn malformed_primary_proxy_providers_remain_unchanged_without_supplement() {
        let primary = mapping("proxy-providers: invalid");
        let expected = primary.clone();

        let (result, conflicts) = multi_profile_merge(&[primary], &["primary"]);

        assert!(conflicts.is_empty());
        assert_eq!(result, expected);
    }

    #[test]
    fn duplicate_proxy_group_use_members_are_merged() {
        let primary =
            group_mapping("proxy-groups:\n  - name: provider-group\n    type: select\n    use:\n      - provider-a");
        let supp =
            group_mapping("proxy-groups:\n  - name: provider-group\n    type: select\n    use:\n      - provider-b");

        let (result, conflicts) = multi_profile_merge(&[primary, supp], &["primary", "supp"]);

        assert!(conflicts.is_empty());
        let groups = result.get("proxy-groups").unwrap().as_sequence().unwrap();
        let providers = groups[0]
            .as_mapping()
            .unwrap()
            .get("use")
            .unwrap()
            .as_sequence()
            .unwrap();
        assert_eq!(providers[0].as_str().unwrap(), "provider-b");
        assert_eq!(providers[1].as_str().unwrap(), "provider-a");
    }

    #[test]
    fn proxy_providers_are_merged_for_group_use_members() {
        let primary = mapping(
            "proxy-groups:\n  - name: provider-group\n    type: select\n    use:\n      - provider-a\nproxy-providers:\n  provider-a:\n    type: http\n    url: https://example.com/a.yaml\n    path: ./provider-a.yaml\n    interval: 3600",
        );
        let supp = mapping(
            "proxy-groups:\n  - name: provider-group\n    type: select\n    use:\n      - provider-b\nproxy-providers:\n  provider-b:\n    type: http\n    url: https://example.com/b.yaml\n    path: ./provider-b.yaml\n    interval: 3600",
        );

        let (result, conflicts) = multi_profile_merge(&[primary, supp], &["primary", "supp"]);

        assert!(conflicts.is_empty());
        let proxy_providers = result.get("proxy-providers").unwrap().as_mapping().unwrap();
        assert!(proxy_providers.get("provider-a").is_some());
        assert!(proxy_providers.get("provider-b").is_some());
        let groups = result.get("proxy-groups").unwrap().as_sequence().unwrap();
        let providers = groups[0]
            .as_mapping()
            .unwrap()
            .get("use")
            .unwrap()
            .as_sequence()
            .unwrap();
        assert_eq!(providers[0].as_str().unwrap(), "provider-b");
        assert_eq!(providers[1].as_str().unwrap(), "provider-a");
    }
}
