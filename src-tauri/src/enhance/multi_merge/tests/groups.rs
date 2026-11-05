use super::super::multi_profile_merge;
use super::{group_mapping, mapping};

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
