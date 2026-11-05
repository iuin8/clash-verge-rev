use serde_yaml_ng::{Mapping, Value};

use super::{MergeContext, ensure_mapping_field};

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

pub(super) fn merge_rule_providers(ctx: &mut MergeContext, base: &mut Mapping, supp: &Mapping) {
    merge_named_mapping(ctx, base, supp, "rule-providers");
}

pub(super) fn merge_proxy_providers(ctx: &mut MergeContext, base: &mut Mapping, supp: &Mapping) {
    merge_named_mapping(ctx, base, supp, "proxy-providers");
}
