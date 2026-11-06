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
        generated_sha256: String::new(),
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
