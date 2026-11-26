use super::*;

fn item(uid: &str, itype: &str, file: Option<&str>) -> PrfItem {
    PrfItem {
        uid: Some(uid.into()),
        itype: Some(itype.into()),
        file: file.map(Into::into),
        ..PrfItem::default()
    }
}

fn uids(list: &[&str]) -> Vec<String> {
    list.iter().map(|uid| (*uid).into()).collect()
}

fn meta(members: &[&str]) -> BundleMeta {
    BundleMeta {
        version: META_VERSION,
        members: uids(members),
        sources: BTreeMap::new(),
        members_sha256: String::new(),
        generated_sha256: String::new(),
        validated: false,
        conflicts: Vec::new(),
        generated_at: 0,
    }
}

#[test]
fn metadata_is_strict_and_versioned() {
    assert!(parse_meta("version: 1\nmembers: [a, b]\n").is_ok());
    assert!(
        parse_meta("version: 1\nmembers: [a, b]\nunknown: true\n").is_err(),
        "unknown keys must be rejected so a typo cannot change the member list"
    );
    assert!(parse_meta("version: 2\nmembers: [a, b]\n").is_err());
    assert!(parse_meta("version: 1\nmembers: [a]\n").is_err());
}

#[test]
fn member_lists_need_two_unique_non_bundle_uids() {
    assert!(validate_member_list(&uids(&["a", "b"])).is_ok());
    assert!(validate_member_list(&uids(&["a"])).is_err());
    assert!(validate_member_list(&uids(&["a", "a"])).is_err());
    assert!(validate_member_list(&uids(&["a", BUNDLE_UID])).is_err());
    assert!(validate_member_list(&uids(&["", "b"])).is_err());
}

#[test]
fn validation_rejects_non_subscriptions_and_missing_items() {
    let profiles = IProfiles {
        current: None,
        items: Some(vec![
            item("local", "local", Some("local.yaml")),
            item("remote", "remote", Some("remote.yaml")),
            item("chain", "merge", Some("chain.yaml")),
            item("no-file", "remote", None),
        ]),
    };

    assert!(validate_members(&profiles, &uids(&["local", "remote"])).is_ok());
    for rejected in [
        uids(&["local", "missing"]),
        uids(&["local", "chain"]),
        uids(&["no-file", "remote"]),
    ] {
        assert!(validate_members(&profiles, &rejected).is_err(), "{rejected:?}");
    }
}

fn subscription_item(uid: &str) -> PrfItem {
    PrfItem {
        uid: Some(uid.into()),
        itype: Some("local".into()),
        file: Some(format!("{uid}.yaml").into()),
        ..PrfItem::default()
    }
}

#[test]
fn a_bundle_accepts_one_openp2p_subscription_member() {
    let profiles = IProfiles {
        current: None,
        items: Some(vec![
            item("local", "local", Some("local.yaml")),
            subscription_item("op2p-sub-0123456789abcdef"),
        ]),
    };
    validate_members(&profiles, &uids(&["local", "op2p-sub-0123456789abcdef"]))
        .expect("one OpenP2P subscription member is allowed");
}

#[test]
fn a_bundle_rejects_two_openp2p_subscription_members() {
    let profiles = IProfiles {
        current: None,
        items: Some(vec![
            item("local", "local", Some("local.yaml")),
            subscription_item("op2p-sub-0123456789abcdef"),
            subscription_item("op2p-sub-fedcba9876543210"),
        ]),
    };
    let error = validate_members(
        &profiles,
        &uids(&["local", "op2p-sub-0123456789abcdef", "op2p-sub-fedcba9876543210"]),
    )
    .expect_err("two OpenP2P subscriptions must be rejected");
    assert!(
        format!("{error:#}").contains("at most one OpenP2P subscription"),
        "{error:#}"
    );
}

#[test]
fn a_bundle_merges_a_subscription_member_without_its_secrets() {
    use crate::core::openp2p::portable::parse_import;
    use serde_yaml_ng::{Mapping, Value};

    const TOKEN: &str = "1234567890123";
    const KEY: &str = "-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n-----END OPENSSH PRIVATE KEY-----";
    let key = KEY
        .lines()
        .map(|line| format!("      {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    let document = format!(
        r#"proxies:
  - name: op2p-home
    type: ssh
    server: 127.0.0.1
    port: 21001
    username: tunnel
    private-key: |-
{key}
    host-key: "{host_key}"
x-openp2p:
  version: 1
  accounts:
    - id: home
      label: Home
      gateway: {{ host: api.openp2p.cn, port: 27183 }}
      token: {TOKEN}
      apps:
        - {{ name: ssh-home, protocol: tcp, src_port: 21001, peer_node: home-exit-01, dst_host: 127.0.0.1, dst_port: 22 }}
"#,
        host_key = crate::core::openp2p::schema::fixtures::sample_host_key()
    );
    let parsed = parse_import(&document, None, "0123456789abcdef");
    assert!(parsed.can_apply(), "{:?}", parsed.issues);
    let member = parsed.profile_yaml().expect("projected profile").to_owned();
    assert!(!member.contains(TOKEN));
    assert!(!member.contains("PRIVATE KEY"));

    let base: Mapping = serde_yaml_ng::from_str("mixed-port: 7890\nrules:\n  - MATCH,DIRECT\n").expect("base");
    let member: Mapping = serde_yaml_ng::from_str(&member).expect("member");
    let (merged, _) = crate::enhance::multi_merge::multi_profile_merge(&[base, member], &["base", "openp2p"]);
    let merged = serde_yaml_ng::to_string(&Value::Mapping(merged)).expect("yaml");
    assert!(!merged.contains(TOKEN), "token must not reach the bundle profile");
    assert!(
        !merged.contains("PRIVATE KEY"),
        "key body must not reach the bundle profile"
    );
    assert!(merged.contains("op2p-home"));
}

#[test]
fn upsert_keeps_a_single_bundle_item() {
    let mut profiles = IProfiles {
        current: None,
        items: Some(vec![]),
    };

    assert!(upsert_item(&mut profiles));
    let items = profiles.items.as_ref().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].uid.as_deref(), Some(BUNDLE_UID));
    assert_eq!(items[0].itype.as_deref(), Some("local"));
    assert_eq!(items[0].file.as_deref(), Some(BUNDLE_FILE));

    profiles.items.as_mut().unwrap()[0].name = Some("My Bundle".into());
    assert!(!upsert_item(&mut profiles));
    let items = profiles.items.as_ref().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].name.as_deref(), Some("My Bundle"));

    profiles
        .items
        .as_mut()
        .unwrap()
        .push(item(BUNDLE_UID, "local", Some("other.yaml")));
    assert!(!upsert_item(&mut profiles));
    assert_eq!(profiles.items.as_ref().unwrap().len(), 1);
}

#[test]
fn prune_keeps_order_and_tears_down_below_two_members() {
    assert_eq!(
        plan_prune(&meta(&["a", "b", "c"]), "b"),
        PrunePlan::Regenerate(uids(&["a", "c"]))
    );
    assert_eq!(plan_prune(&meta(&["a", "b"]), "a"), PrunePlan::RemoveBundle);
    assert_eq!(plan_prune(&meta(&["a", "b"]), "outsider"), PrunePlan::Unchanged);
}

#[test]
fn migration_filters_unavailable_and_duplicate_members() {
    let items = vec![
        item("a", "remote", Some("a.yaml")),
        item("b", "local", Some("b.yaml")),
        item("chain", "merge", Some("chain.yaml")),
    ];

    let plan = plan_migration(&items, &uids(&["a", BUNDLE_UID, "a", "chain", "missing", "b"]))
        .expect("two usable members remain");
    assert_eq!(plan.members, uids(&["a", "b"]));
    assert_eq!(plan.dropped, uids(&[BUNDLE_UID, "a", "chain", "missing"]));

    assert!(
        plan_migration(&items, &uids(&["a", "missing"])).is_none(),
        "fewer than two usable members must not create a bundle"
    );
}

#[test]
fn legacy_store_parsing_is_strict_but_tolerant_of_missing_uids() {
    assert_eq!(
        super::migration::parse_legacy(r#"{"uids":["a","b"]}"#).unwrap(),
        uids(&["a", "b"])
    );
    assert_eq!(super::migration::parse_legacy("{}").unwrap(), Vec::<String>::new());
    assert!(super::migration::parse_legacy("not json").is_err());
    assert!(super::migration::parse_legacy(r#"{"uids":"a"}"#).is_err());
}

#[test]
fn bundle_generated_paths_are_fixed_and_fork_owned() {
    assert_eq!(BUNDLE_FILE, "bundle-main.yaml");
    assert_eq!(BUNDLE_META_FILE, "bundle-main.yaml");
    assert!(is_bundle_uid(BUNDLE_UID));
    assert!(!is_bundle_uid("bundle-other"));
}

#[test]
fn invalid_generated_output_is_never_written() {
    assert_eq!(commit_decision(BundleValidation::Valid, false), CommitDecision::Write);
    assert_eq!(
        commit_decision(BundleValidation::Invalid, false),
        CommitDecision::Reject
    );
    assert_eq!(commit_decision(BundleValidation::Invalid, true), CommitDecision::Reject);
    assert_eq!(
        commit_decision(BundleValidation::Unavailable, true),
        CommitDecision::Write,
        "paths that immediately apply the runtime may proceed and roll back on failure"
    );
    assert_eq!(
        commit_decision(BundleValidation::Unavailable, false),
        CommitDecision::KeepStale,
        "background refreshes must not write unvalidated YAML"
    );
}

#[test]
fn teardown_clears_only_the_bundle_dns_override() {
    use crate::config::dns::ProfileDnsSettings;

    let mut verge = crate::config::IVerge::default();
    verge
        .profile_dns_settings
        .insert(BUNDLE_UID.into(), ProfileDnsSettings::default());
    verge
        .profile_dns_settings
        .insert("other".into(), ProfileDnsSettings::default());

    assert!(remove_dns_settings(&mut verge));
    assert!(!verge.profile_dns_settings.contains_key(BUNDLE_UID));
    assert!(verge.profile_dns_settings.contains_key("other"));
    assert!(!remove_dns_settings(&mut verge), "clearing is idempotent");
}

#[test]
fn pruning_an_inactive_bundle_markers_it_stale() {
    let mut current = meta(&["a", "b"]);
    current.sources.insert("a".into(), String::from("hash-a"));
    current.generated_sha256 = String::from("generated-hash");

    let stale = stale_meta_after_prune(&current, &uids(&["b"]));
    assert_eq!(stale.members, uids(&["b"]));
    assert!(
        stale.generated_sha256.is_empty(),
        "old generated file must be treated as stale"
    );
    assert!(!stale.sources.contains_key("a"));
    assert!(!stale.sources.contains_key("b"));
    assert!(stale.conflicts.is_empty());
}

#[test]
fn metadata_fingerprint_detects_member_or_order_tampering() {
    let mut current = meta(&["a", "b"]);
    current.members_sha256 = members_fingerprint(current.version, &current.members);
    assert!(metadata_fingerprint_matches(&current));

    current.members.swap(0, 1);
    assert!(
        !metadata_fingerprint_matches(&current),
        "reordering members must invalidate the fingerprint"
    );
    assert!(!metadata_trusted(&current));

    current.members.swap(0, 1);
    current.members[0] = String::from("c");
    assert!(!metadata_fingerprint_matches(&current));

    current.members_sha256 = String::new();
    assert!(!metadata_fingerprint_matches(&current), "legacy metadata is stale");
    assert!(
        metadata_trusted(&current),
        "empty fingerprints are legacy, not tampered"
    );
}

#[test]
fn orphan_cleanup_always_clears_dns_for_a_missing_item() {
    assert_eq!(plan_orphan_cleanup(false, false), (false, true));
    assert_eq!(plan_orphan_cleanup(false, true), (true, true));
    assert_eq!(plan_orphan_cleanup(true, false), (false, false));
    assert_eq!(plan_orphan_cleanup(true, true), (false, false));
}
