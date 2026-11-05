use std::collections::{HashMap, HashSet};

use serde_yaml_ng::{Mapping, Value};

use super::{MergeContext, ensure_sequence_field, renamed_target};

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

pub(super) fn merge_rules(ctx: &mut MergeContext, base: &mut Mapping, supp: &Mapping) {
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
