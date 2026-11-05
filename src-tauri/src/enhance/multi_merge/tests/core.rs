use super::super::multi_profile_merge;
use super::mapping;

const LOCAL_LAN_PRIMARY_YAML: &str =
    include_str!("../../../../../docs/repro/local-lan-rule-set/10-merge-primary-valid.yaml");
const LOCAL_LAN_SUPPLEMENT_YAML: &str =
    include_str!("../../../../../docs/repro/local-lan-rule-set/11-merge-supplement-valid.yaml");
const LOCAL_LAN_BROKEN_BASELINE_YAML: &str =
    include_str!("../../../../../docs/repro/local-lan-rule-set/12-expected-broken-merged-output.yaml");

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
