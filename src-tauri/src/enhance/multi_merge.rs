use std::collections::HashSet;

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

fn normalize_existing_sequence_field(
    base: &mut Mapping,
    field: &str,
    primary_name: &str,
    conflicts: &mut Vec<ConflictEntry>,
) {
    let Some(base_value) = base.get_mut(field) else {
        return;
    };
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
}

fn normalize_existing_mapping_field(
    base: &mut Mapping,
    field: &str,
    primary_name: &str,
    conflicts: &mut Vec<ConflictEntry>,
) {
    let Some(base_value) = base.get_mut(field) else {
        return;
    };
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
}

fn normalize_group_member_field(
    group: &mut Mapping,
    field: &str,
    group_name: &str,
    primary_name: &str,
    conflicts: &mut Vec<ConflictEntry>,
) {
    let Some(member_value) = group.get_mut(field) else {
        return;
    };
    if !matches!(member_value, Value::Sequence(_)) {
        push_conflict(
            conflicts,
            "proxy-groups",
            group_name,
            primary_name,
            format!("existing group has invalid `{field}` members; replaced during merge"),
        );
        *member_value = Value::Sequence(vec![]);
    }
}

fn merge_proxies(
    base: &mut Mapping,
    supp: &Mapping,
    supp_name: &str,
    primary_name: &str,
    conflicts: &mut Vec<ConflictEntry>,
) {
    if let Some(Value::Sequence(supp_proxies)) = supp.get("proxies") {
        let base_seq = ensure_sequence_field(base, "proxies", primary_name, conflicts);
        let mut existing_names: HashSet<String> = base_seq
            .iter()
            .filter_map(|p| {
                p.as_mapping()
                    .and_then(|m| m.get("name"))
                    .and_then(|v| v.as_str())
                    .map(String::from)
            })
            .collect();

        let mut to_prepend: Vec<Value> = vec![];
        for proxy in supp_proxies {
            let proxy_name = proxy
                .as_mapping()
                .and_then(|m| m.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if !existing_names.insert(proxy_name.to_string()) {
                push_conflict(
                    conflicts,
                    "proxies",
                    proxy_name,
                    supp_name,
                    format!("already exists in {primary_name}"),
                );
                continue;
            }
            to_prepend.push(proxy.clone());
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

fn merge_group_member_list(existing_map: &mut Mapping, incoming_map: &Mapping, field: &str) {
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
    if let Value::Sequence(existing_seq) = existing_members {
        let mut seen_members: HashSet<String> = existing_seq
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();
        let mut members_to_prepend: Vec<Value> = vec![];
        for member in &supp_members {
            let member_name = member.as_str().unwrap_or("");
            if seen_members.insert(member_name.to_string()) {
                members_to_prepend.push(member.clone());
            }
        }
        for item in members_to_prepend.into_iter().rev() {
            existing_seq.insert(0, item);
        }
    }
}

fn merge_group_members(existing_group: &mut Value, incoming_group: &Value) {
    let Some(existing_map) = existing_group.as_mapping_mut() else {
        return;
    };
    let Some(incoming_map) = incoming_group.as_mapping() else {
        return;
    };

    merge_group_member_list(existing_map, incoming_map, "proxies");
    merge_group_member_list(existing_map, incoming_map, "use");
}

fn group_member_mode(mapping: &Mapping) -> &'static str {
    let has_proxies = mapping.get("proxies").and_then(Value::as_sequence).is_some();
    let has_use = mapping.get("use").and_then(Value::as_sequence).is_some();

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

    let existing_mode = group_member_mode(existing_map);
    let incoming_mode = group_member_mode(incoming_map);

    existing_mode == incoming_mode || existing_mode == "none" || incoming_mode == "none"
}

fn normalize_primary_merge_fields(base: &mut Mapping, primary_name: &str, conflicts: &mut Vec<ConflictEntry>) {
    normalize_existing_sequence_field(base, "proxies", primary_name, conflicts);
    normalize_existing_mapping_field(base, "proxy-providers", primary_name, conflicts);
    normalize_existing_sequence_field(base, "proxy-groups", primary_name, conflicts);
    normalize_existing_sequence_field(base, "rules", primary_name, conflicts);
    normalize_existing_mapping_field(base, "rule-providers", primary_name, conflicts);
}

fn merge_proxy_groups(
    base: &mut Mapping,
    supp: &Mapping,
    supp_name: &str,
    primary_name: &str,
    conflicts: &mut Vec<ConflictEntry>,
) {
    if let Some(Value::Sequence(supp_groups)) = supp.get("proxy-groups") {
        let base_seq = ensure_sequence_field(base, "proxy-groups", primary_name, conflicts);
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
                groups_to_prepend.push(group.clone());
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

            if let Some(target_map) = target_group.as_mapping_mut() {
                normalize_group_member_field(target_map, "proxies", group_name, primary_name, conflicts);
                normalize_group_member_field(target_map, "use", group_name, primary_name, conflicts);
            }

            let existing_settings = target_group
                .as_mapping()
                .map(filtered_group_settings)
                .unwrap_or_default();
            let incoming_settings = group.as_mapping().map(filtered_group_settings).unwrap_or_default();
            if existing_settings != incoming_settings || !can_merge_group_members(target_group, group) {
                push_conflict(
                    conflicts,
                    "proxy-groups",
                    group_name,
                    supp_name,
                    format!("already exists in {primary_name} with incompatible settings; kept existing group"),
                );
                continue;
            }

            merge_group_members(target_group, group);
        }

        for item in groups_to_prepend.into_iter().rev() {
            base_seq.insert(0, item);
        }
    }
}

fn merge_rules(base: &mut Mapping, supp: &Mapping, primary_name: &str, conflicts: &mut Vec<ConflictEntry>) {
    if let Some(Value::Sequence(supp_rules)) = supp.get("rules") {
        let base_seq = ensure_sequence_field(base, "rules", primary_name, conflicts);
        let mut existing_rules: HashSet<String> = base_seq
            .iter()
            .filter_map(|rule| rule.as_str().map(String::from))
            .collect();
        let mut rules_to_prepend: Vec<Value> = vec![];
        for rule in supp_rules {
            let rule_str = rule.as_str().unwrap_or("");
            if existing_rules.insert(rule_str.to_string()) {
                rules_to_prepend.push(rule.clone());
            }
        }
        for item in rules_to_prepend.into_iter().rev() {
            base_seq.insert(0, item);
        }
    }
}

fn merge_named_mapping(
    base: &mut Mapping,
    supp: &Mapping,
    field: &str,
    supp_name: &str,
    primary_name: &str,
    conflicts: &mut Vec<ConflictEntry>,
) {
    if let Some(Value::Mapping(supp_map)) = supp.get(field) {
        let base_map = ensure_mapping_field(base, field, primary_name, conflicts);
        for (name, value) in supp_map {
            let name_str = name.as_str().map(str::to_owned).unwrap_or_else(|| format!("{name:?}"));
            match base_map.get(name) {
                None => {
                    base_map.insert(name.clone(), value.clone());
                }
                Some(existing) if existing == value => {}
                Some(_) => {
                    push_conflict(
                        conflicts,
                        field,
                        name_str,
                        supp_name,
                        format!("already exists in {primary_name} with different definition"),
                    );
                }
            }
        }
    }
}

fn merge_rule_providers(
    base: &mut Mapping,
    supp: &Mapping,
    supp_name: &str,
    primary_name: &str,
    conflicts: &mut Vec<ConflictEntry>,
) {
    merge_named_mapping(base, supp, "rule-providers", supp_name, primary_name, conflicts);
}

fn merge_proxy_providers(
    base: &mut Mapping,
    supp: &Mapping,
    supp_name: &str,
    primary_name: &str,
    conflicts: &mut Vec<ConflictEntry>,
) {
    merge_named_mapping(base, supp, "proxy-providers", supp_name, primary_name, conflicts);
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
    let mut conflicts: Vec<ConflictEntry> = vec![];

    normalize_primary_merge_fields(&mut base, primary_name, &mut conflicts);

    for (i, supp) in configs[1..].iter().enumerate() {
        let supp_name = names.get(i + 1).copied().unwrap_or("unknown");
        merge_proxies(&mut base, supp, supp_name, primary_name, &mut conflicts);
        merge_proxy_providers(&mut base, supp, supp_name, primary_name, &mut conflicts);
        merge_proxy_groups(&mut base, supp, supp_name, primary_name, &mut conflicts);
        merge_rules(&mut base, supp, primary_name, &mut conflicts);
        merge_rule_providers(&mut base, supp, supp_name, primary_name, &mut conflicts);
    }

    (base, conflicts)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const LOCAL_LAN_PRIMARY_YAML: &str = r#"
mixed-port: 7890
mode: rule
log-level: info

proxies: []
proxy-groups: []

rules:
  - MATCH,DIRECT
"#;

    const LOCAL_LAN_SUPPLEMENT_YAML: &str = r#"
mixed-port: 7891
mode: rule
log-level: info

proxies: []
proxy-groups: []

rule-providers:
  Local-LAN:
    type: inline
    behavior: classical
    payload:
      - IP-CIDR,192.168.0.0/16,DIRECT
      - DOMAIN-SUFFIX,lan,DIRECT

rules:
  - IP-CIDR,10.0.0.0/8,DIRECT
  - RULE-SET,Local-LAN,DIRECT
  - MATCH,DIRECT
"#;

    const LOCAL_LAN_BROKEN_BASELINE_YAML: &str = r#"
mixed-port: 7890
mode: rule
log-level: info

proxies: []
proxy-groups: []

rules:
  - IP-CIDR,10.0.0.0/8,DIRECT
  - RULE-SET,Local-LAN,DIRECT
  - MATCH,DIRECT
"#;

    fn mapping(yaml: &str) -> Mapping {
        serde_yaml_ng::from_str(yaml).expect("test YAML is valid")
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
        let proxies = result.get("proxies").unwrap().as_sequence().unwrap();
        assert_eq!(proxies.len(), 1);
        assert_eq!(
            proxies[0].as_mapping().unwrap().get("name").unwrap().as_str().unwrap(),
            "px1"
        );
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
    fn duplicate_proxy_name_logged_as_conflict() {
        let primary = mapping("proxies:\n  - name: dup\n    type: ss");
        let supp = mapping("proxies:\n  - name: dup\n    type: vmess");
        let (result, conflicts) = multi_profile_merge(&[primary, supp], &["primary", "supp"]);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].name, "dup");
        assert_eq!(conflicts[0].field, "proxies");
        let proxies = result.get("proxies").unwrap().as_sequence().unwrap();
        assert_eq!(proxies.len(), 1);
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
    fn top_level_keys_from_primary_are_preserved() {
        let primary = mapping("dns:\n  enable: true\nproxies:\n  - name: p1\n    type: ss");
        let supp = mapping("dns:\n  enable: false\nproxies:\n  - name: p2\n    type: vmess");
        let (result, _) = multi_profile_merge(&[primary, supp], &["primary", "supp"]);
        let dns = result.get("dns").unwrap().as_mapping().unwrap();
        assert!(dns.get("enable").unwrap().as_bool().unwrap());
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

        assert!(conflicts.is_empty());
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

        assert!(conflicts.is_empty());
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
    fn conflicting_rule_provider_definitions_are_logged_and_primary_wins() {
        let primary = mapping(
            "rule-providers:\n  Local-LAN:\n    type: inline\n    behavior: classical\n    payload:\n      - IP-CIDR,192.168.0.0/16,DIRECT",
        );
        let supp = mapping(
            "rule-providers:\n  Local-LAN:\n    type: inline\n    behavior: classical\n    payload:\n      - IP-CIDR,10.0.0.0/8,DIRECT",
        );

        let (result, conflicts) = multi_profile_merge(&[primary, supp], &["primary", "supp"]);

        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].field, "rule-providers");
        assert_eq!(conflicts[0].name, "Local-LAN");
        let rule_providers = result.get("rule-providers").unwrap().as_mapping().unwrap();
        let provider = rule_providers.get("Local-LAN").unwrap().as_mapping().unwrap();
        let payload = provider.get("payload").unwrap().as_sequence().unwrap();
        assert_eq!(payload[0].as_str().unwrap(), "IP-CIDR,192.168.0.0/16,DIRECT");
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
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].field, "proxies");
        assert_eq!(conflicts[0].name, "dup");
    }

    #[test]
    fn duplicate_proxy_groups_inside_same_supplement_are_deduped() {
        let primary = mapping("proxy-groups: []");
        let supp = mapping(
            "proxy-groups:\n  - name: auto\n    type: select\n    proxies:\n      - a\n  - name: auto\n    type: select\n    proxies:\n      - a",
        );

        let (result, conflicts) = multi_profile_merge(&[primary, supp], &["primary", "supp"]);

        let groups = result.get("proxy-groups").unwrap().as_sequence().unwrap();
        assert_eq!(groups.len(), 1);
        assert!(conflicts.is_empty());
    }

    #[test]
    fn duplicate_proxy_group_settings_keep_existing_members() {
        let primary = mapping(
            "proxy-groups:\n  - name: auto\n    type: select\n    url: https://a.example/test\n    proxies:\n      - a",
        );
        let supp = mapping(
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
    fn malformed_primary_proxies_are_normalized_without_supplement() {
        let primary = mapping("proxies: invalid");

        let (result, conflicts) = multi_profile_merge(&[primary], &["primary"]);

        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].field, "proxies");
        let proxies = result.get("proxies").unwrap().as_sequence().unwrap();
        assert!(proxies.is_empty());
    }

    #[test]
    fn empty_primary_proxy_group_members_can_be_filled_by_supplement() {
        let primary = mapping("proxy-groups:\n  - name: auto\n    type: select\n    proxies: []");
        let supp = mapping("proxy-groups:\n  - name: auto\n    type: select\n    proxies:\n      - a");

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
        let primary = mapping("proxy-groups:\n  - name: provider-group\n    type: select\n    use: []");
        let supp = mapping("proxy-groups:\n  - name: provider-group\n    type: select\n    use:\n      - provider-a");

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
    fn malformed_primary_rules_are_normalized_without_supplement() {
        let primary = mapping("rules: invalid");

        let (result, conflicts) = multi_profile_merge(&[primary], &["primary"]);

        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].field, "rules");
        let rules = result.get("rules").unwrap().as_sequence().unwrap();
        assert!(rules.is_empty());
    }

    #[test]
    fn malformed_primary_proxy_groups_are_replaced_during_merge() {
        let primary = mapping("proxy-groups: invalid");
        let supp = mapping("proxy-groups:\n  - name: auto\n    type: select\n    proxies:\n      - a");

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
    fn malformed_primary_proxy_group_members_are_repaired_during_merge() {
        let primary = mapping("proxy-groups:\n  - name: auto\n    type: select\n    proxies: invalid");
        let supp = mapping("proxy-groups:\n  - name: auto\n    type: select\n    proxies:\n      - a");

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
    fn missing_supplementary_proxy_group_members_do_not_create_conflict() {
        let primary = mapping("proxy-groups:\n  - name: auto\n    type: select\n    proxies:\n      - a");
        let supp = mapping("proxy-groups:\n  - name: auto\n    type: select");

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
    fn malformed_primary_rule_providers_are_normalized_without_supplement() {
        let primary = mapping("rule-providers: invalid");

        let (result, conflicts) = multi_profile_merge(&[primary], &["primary"]);

        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].field, "rule-providers");
        let providers = result.get("rule-providers").unwrap().as_mapping().unwrap();
        assert!(providers.is_empty());
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
    fn malformed_primary_proxy_providers_are_normalized_without_supplement() {
        let primary = mapping("proxy-providers: invalid");

        let (result, conflicts) = multi_profile_merge(&[primary], &["primary"]);

        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].field, "proxy-providers");
        let providers = result.get("proxy-providers").unwrap().as_mapping().unwrap();
        assert!(providers.is_empty());
    }

    #[test]
    fn duplicate_proxy_group_use_members_are_merged() {
        let primary =
            mapping("proxy-groups:\n  - name: provider-group\n    type: select\n    use:\n      - provider-a");
        let supp = mapping("proxy-groups:\n  - name: provider-group\n    type: select\n    use:\n      - provider-b");

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
