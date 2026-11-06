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
