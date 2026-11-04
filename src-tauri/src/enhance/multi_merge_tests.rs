use super::*;

const LOCAL_LAN_PRIMARY_YAML: &str = include_str!("../../../docs/repro/local-lan-rule-set/10-merge-primary-valid.yaml");
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
    let primary = mapping("dns:\n  enable: true\n  nameserver:\n    - 1.1.1.1\nproxies:\n  - name: p\n    type: ss");
    let supp = mapping("dns:\n  enable: false\n  fallback:\n    - 8.8.8.8\nproxies:\n  - name: p2\n    type: vmess");
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
fn builtin_policy_members_survive_reference_integrity() {
    let primary = mapping(
        "proxies:\n  - name: p1\n    type: ss\nproxy-groups:\n  - name: g\n    type: select\n    proxies:\n      - p1\n      - REJECT\n      - REJECT-DROP\n      - COMPATIBLE\n      - PASS\n      - PASS-RULE",
    );
    let supp = mapping("rules:\n  - MATCH,DIRECT");
    let (result, conflicts) = multi_profile_merge(&[primary, supp], &["primary", "supp"]);

    assert!(conflicts.is_empty(), "{conflicts:?}");
    let groups = result.get("proxy-groups").unwrap().as_sequence().unwrap();
    let members = groups[0]
        .as_mapping()
        .unwrap()
        .get("proxies")
        .unwrap()
        .as_sequence()
        .unwrap();
    for name in ["REJECT", "REJECT-DROP", "COMPATIBLE", "PASS", "PASS-RULE"] {
        assert!(
            members.iter().any(|m| m.as_str() == Some(name)),
            "missing {name}: {members:?}"
        );
    }
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
    let supp = group_mapping("proxy-groups:\n  - name: provider-group\n    type: select\n    use:\n      - provider-a");

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
    let supp = group_mapping("proxy-groups:\n  - name: provider-group\n    type: select\n    use:\n      - provider-a");

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
    let supp = group_mapping("proxy-groups:\n  - name: provider-group\n    type: select\n    use:\n      - provider-b");

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
