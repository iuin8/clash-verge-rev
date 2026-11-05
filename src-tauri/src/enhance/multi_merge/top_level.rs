use serde_yaml_ng::{Mapping, Value};

use clash_verge_logging::{Type, logging};

use super::{MergeContext, push_conflict};

pub(super) fn log_top_level_key_conflicts(ctx: &mut MergeContext, base: &mut Mapping, supp: &Mapping) {
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
