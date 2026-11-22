use super::build::build_artifacts_with;
use super::files::{
    get_key, read_mapping_sync, read_seq, reject_group_conflict, reject_proxy_conflicts, save_seq_sync,
};
use super::*;
use crate::core::openp2p::schema::{self, fixtures::sample_config};
use crate::enhance::seq::SeqMap;
use serde_yaml_ng::Sequence;

fn fixture() -> ArtifactSet {
    build_artifacts_with(&sample_config(), true, |_| true, |_| true)
}

#[test]
fn a_profile_seed_is_read_once_and_missing_blocks_are_ignored() {
    let path = std::env::temp_dir().join(format!(
        "cvr-openp2p-seed-{}-{}.yaml",
        std::process::id(),
        chrono::Local::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    std::fs::write(&path, "proxies: []\nx-openp2p:\n  version: 1\n").expect("write seed");
    let seed = read_profile_seed(&path).expect("seed");
    assert!(seed.contains("version: 1"));

    std::fs::write(&path, "proxies: []\n").expect("write no seed");
    assert!(read_profile_seed(&path).is_none());

    std::fs::write(&path, "x-openp2p: [unterminated").expect("write invalid yaml");
    assert!(read_profile_seed(&path).is_none());
    let _ = std::fs::remove_file(path);
}

#[test]
fn ssh_nodes_group_rules_and_dns_are_materialized() {
    let set = fixture();
    assert_eq!(set.proxy_names, vec!["op2p-home".to_string()]);
    let node = set.proxies[0].as_mapping().expect("node mapping");
    assert_eq!(node.get("type").and_then(Value::as_str), Some("ssh"));
    assert_eq!(node.get("server").and_then(Value::as_str), Some("127.0.0.1"));
    assert_eq!(node.get("port").and_then(Value::as_u64), Some(21001));
    assert_eq!(
        node.get("private-key").and_then(Value::as_str),
        Some("openp2p/keys/o2p/home/id_ed25519")
    );
    assert!(
        node.get("host-key")
            .and_then(Value::as_sequence)
            .is_some_and(|keys| keys.len() == 1)
    );

    let group = set.group.as_ref().and_then(Value::as_mapping).expect("group");
    assert_eq!(group.get("type").and_then(Value::as_str), Some("fallback"));
    assert_eq!(group.get("empty-fallback").and_then(Value::as_str), Some("REJECT"));
    assert_eq!(group.get("lazy").and_then(Value::as_bool), Some(false));
    assert_eq!(group.get("interval").and_then(Value::as_u64), Some(60));
    assert_eq!(group.get("max-failed-times").and_then(Value::as_u64), Some(1));
    assert_eq!(
        group.get("url").and_then(Value::as_str),
        Some("http://health.corp.example/")
    );

    assert_eq!(
        set.rule_texts,
        vec![
            "AND,((NETWORK,UDP),(DOMAIN-SUFFIX,corp.example)),REJECT".to_string(),
            "AND,((NETWORK,UDP),(IP-CIDR,10.0.0.0/8,no-resolve)),REJECT".to_string(),
            "DOMAIN-SUFFIX,corp.example,OP2P".to_string(),
            "IP-CIDR,10.0.0.0/8,OP2P,no-resolve".to_string(),
        ]
    );
    assert_eq!(
        set.dns
            .get("+.corp.example")
            .and_then(Value::as_sequence)
            .and_then(|values| values[0].as_str()),
        Some("tcp://10.0.0.53:53#OP2P")
    );
}

#[test]
fn a_missing_secret_never_produces_a_half_configured_artifact_set() {
    let set = build_artifacts_with(&sample_config(), true, |_| true, |_| false);
    assert!(set.proxies.is_empty());
    assert!(set.group.is_none());
    assert!(set.rules.is_empty());
    assert!(set.warnings.iter().any(|issue| issue.code == "key_missing"));

    let token_missing = build_artifacts_with(&sample_config(), true, |_| false, |_| true);
    assert!(token_missing.proxies.is_empty());
    assert!(token_missing.warnings.iter().any(|issue| issue.code == "token_missing"));
}

#[test]
fn a_preview_shows_the_planned_nodes_without_secrets() {
    let set = build_artifacts_with(&sample_config(), false, |_| false, |_| false);
    assert_eq!(set.proxy_names.len(), 1);
    assert!(set.warnings.iter().any(|issue| issue.code == "token_missing"));
}

#[test]
fn managed_sequence_entries_are_replaced_without_touching_user_entries() {
    let path = std::env::temp_dir().join(format!(
        "cvr-openp2p-seq-{}-{}.yaml",
        std::process::id(),
        chrono::Local::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    let mut seq = SeqMap::default();
    seq.prepend
        .push(serde_yaml_ng::from_str("name: user-proxy\ntype: ss").expect("user"));
    seq.prepend
        .push(serde_yaml_ng::from_str("name: old-managed\ntype: ssh").expect("old"));
    seq.delete.push("old-managed".into());
    let old = ProfileArtifacts {
        profile_uid: "L1".into(),
        merge_uid: "m1".into(),
        proxies_uid: "p1".into(),
        groups_uid: "g1".into(),
        rules_uid: "r1".into(),
        proxy_names: vec!["old-managed".into()],
        group_name: Some("OP2P".into()),
        rules: Vec::new(),
        dns_domains: Vec::new(),
        dns_previous: BTreeMap::new(),
        applied_at: 0,
    };
    cleanup_seq(&mut seq, Some(&old), &BTreeSet::new(), &BTreeSet::new());
    save_seq_sync(&path, &seq).expect("save");
    let text = std::fs::read_to_string(&path).expect("read");
    assert!(text.contains("user-proxy"));
    assert!(!text.contains("old-managed"));
    let _ = std::fs::remove_file(path);
}

#[test]
fn a_snapshot_restores_content_and_removes_created_files() {
    let root = std::env::temp_dir().join(format!(
        "cvr-openp2p-snapshot-{}-{}",
        std::process::id(),
        chrono::Local::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    std::fs::create_dir_all(&root).expect("scratch");
    let existing = root.join("existing.yaml");
    let created = root.join("created.yaml");
    std::fs::write(&existing, b"before").expect("write");
    let snapshot = FileSnapshot::capture(&[existing.clone(), created.clone()]);
    std::fs::write(&existing, b"after").expect("write");
    std::fs::write(&created, b"new").expect("write");
    snapshot.restore().expect("restore");
    assert_eq!(std::fs::read(&existing).expect("read"), b"before");
    assert!(!created.exists());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn simple_mapping_string_lookup() {
    for raw in [
        "x-openp2p: 1\ndns: 2",
        "x-openp2p:\n  version: 1\nrules: []",
        "rules: []\nx-openp2p:\n  version: 1",
    ] {
        let mut value: Value = serde_yaml_ng::from_str(raw).expect("value");
        value.apply_merge().expect("merge");
        let mapping = value.as_mapping().expect("mapping");
        assert!(get_key(mapping, "x-openp2p").is_some(), "{raw}");
    }
}

#[test]
fn merge_writes_restore_previous_dns_policy_on_cleanup() {
    let root = std::env::temp_dir().join(format!(
        "cvr-openp2p-merge-{}-{}",
        std::process::id(),
        chrono::Local::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    std::fs::create_dir_all(&root).expect("scratch");
    let path = root.join("m-test.yaml");
    std::fs::write(
            &path,
            b"rules:\n  - MATCH,DIRECT\ndns:\n  nameserver: [9.9.9.9]\n  nameserver-policy:\n    existing.example: [tcp://1.1.1.1:53]\n",
        )
        .expect("merge");
    let config = sample_config();
    let dns = fixture().dns;
    let previous = write_merge(&path, &config, None, &dns).expect("write merge");
    let mapping = read_mapping_sync(&path).expect("read merge");
    assert!(get_key(&mapping, "x-openp2p").is_some());
    assert_eq!(
        get_key(&mapping, "rules")
            .and_then(Value::as_sequence)
            .map(Sequence::len),
        Some(1)
    );
    let policy = get_key(&mapping, "dns")
        .and_then(|dns| get_key(dns.as_mapping().expect("dns mapping"), "nameserver-policy"))
        .and_then(Value::as_mapping)
        .expect("policy");
    assert!(get_key(policy, "existing.example").is_some());
    assert!(get_key(policy, "+.corp.example").is_some());
    assert_eq!(
        get_key(&mapping, "dns")
            .and_then(|dns| get_key(dns.as_mapping()?, "nameserver"))
            .and_then(Value::as_sequence)
            .map(Sequence::len),
        Some(1)
    );

    let entry = ProfileArtifacts {
        profile_uid: "L1".into(),
        merge_uid: "m1".into(),
        proxies_uid: "p1".into(),
        groups_uid: "g1".into(),
        rules_uid: "r1".into(),
        proxy_names: Vec::new(),
        group_name: None,
        rules: Vec::new(),
        dns_domains: dns.keys().cloned().collect(),
        dns_previous: previous,
        applied_at: 0,
    };
    cleanup_merge(&path, &entry).expect("cleanup");
    let mapping = read_mapping_sync(&path).expect("read merge");
    assert!(get_key(&mapping, "x-openp2p").is_none());
    let policy = mapping
        .get("dns")
        .and_then(|dns| dns.get("nameserver-policy"))
        .and_then(Value::as_mapping)
        .expect("policy");
    assert!(get_key(policy, "existing.example").is_some());
    assert!(get_key(policy, "+.corp.example").is_none());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn empty_comment_only_chain_files_are_valid_templates() {
    let root = std::env::temp_dir().join(format!(
        "cvr-openp2p-empty-{}-{}",
        std::process::id(),
        chrono::Local::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    std::fs::create_dir_all(&root).expect("scratch");
    let merge = root.join("m.yaml");
    let seq = root.join("p.yaml");
    std::fs::write(&merge, b"# Profile Enhancement Merge Template for Clash Verge\n").expect("merge");
    std::fs::write(&seq, b"").expect("seq");
    assert!(read_mapping_sync(&merge).expect("merge mapping").is_empty());
    assert!(read_seq(&seq).expect("seq").prepend.is_empty());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn user_chain_names_are_never_silently_replaced() {
    let mut seq = SeqMap::default();
    seq.prepend
        .push(serde_yaml_ng::from_str("name: op2p-home\ntype: ss").expect("user proxy"));
    let names = BTreeSet::from(["op2p-home".to_string()]);
    assert!(reject_proxy_conflicts(&seq, None, &names).is_err());

    let mut groups = SeqMap::default();
    groups
        .prepend
        .push(serde_yaml_ng::from_str("name: OP2P\ntype: select").expect("user group"));
    assert!(reject_group_conflict(&groups, None).is_err());
    assert!(reject_group_conflict(&groups, None).is_err());
}

#[test]
fn schema_rules_are_used_by_the_artifact_builder() {
    let mut config = sample_config();
    let issues = schema::normalize_and_validate(&mut config);
    assert!(!schema::has_errors(&issues), "{issues:?}");
}
