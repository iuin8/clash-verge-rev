use std::collections::HashSet;

use serde_yaml_ng::{Mapping, Value};

use super::{ConflictEntry, push_conflict};

/// 合并完成后校验所有 group 的 proxies/use 成员引用是否存在于最终 config，
/// 不存在的记 conflict 并从成员中移除（降级，不阻断）。
pub(super) fn verify_reference_integrity(base: &mut Mapping, conflicts: &mut Vec<ConflictEntry>) {
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
