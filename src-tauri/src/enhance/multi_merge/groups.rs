use std::collections::{HashMap, HashSet};

use serde_yaml_ng::{Mapping, Value};

use super::{ConflictEntry, MergeContext, ensure_sequence_field, push_conflict, renamed_target};

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

pub(super) fn merge_proxy_groups(ctx: &mut MergeContext, base: &mut Mapping, supp: &Mapping) {
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
