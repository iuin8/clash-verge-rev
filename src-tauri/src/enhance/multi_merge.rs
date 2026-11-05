use std::borrow::Cow;
use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_yaml_ng::{Mapping, Value};

mod groups;
mod integrity;
mod providers;
mod proxies;
mod rules;
mod top_level;

use self::{
    groups::merge_proxy_groups,
    integrity::verify_reference_integrity,
    providers::{merge_proxy_providers, merge_rule_providers},
    proxies::merge_proxies,
    rules::merge_rules,
    top_level::log_top_level_key_conflicts,
};

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

/// 命中 `renames` 则返回重命名后的名字，否则原样返回。
fn renamed_target<'a>(renames: &'a HashMap<String, String>, name: &'a str) -> Cow<'a, str> {
    if let Some(new_name) = renames.get(name) {
        Cow::Borrowed(new_name.as_str())
    } else {
        Cow::Borrowed(name)
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
mod tests;
