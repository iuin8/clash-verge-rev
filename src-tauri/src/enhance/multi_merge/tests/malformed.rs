use super::super::multi_profile_merge;
use super::{group_mapping, mapping};

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
