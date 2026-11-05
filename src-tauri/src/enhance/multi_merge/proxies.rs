use std::collections::HashMap;

use serde_yaml_ng::{Mapping, Value};

use super::{MergeContext, ensure_sequence_field};

/// 返回去掉 `name` 键的浅拷贝，用于「重名但定义是否相同」的比较。
fn strip_name(mapping: &Mapping) -> Mapping {
    mapping
        .iter()
        .filter(|(k, _)| k.as_str() != Some("name"))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

pub(super) fn merge_proxies(ctx: &mut MergeContext, base: &mut Mapping, supp: &Mapping) {
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
