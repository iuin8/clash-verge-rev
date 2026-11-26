use super::fixtures::sample_config;
use super::*;

#[test]
fn sample_config_validates_cleanly() {
    let mut config = sample_config();
    let issues = crate::core::openp2p::schema_validate::normalize_and_validate(&mut config);
    assert!(!has_errors(&issues), "unexpected issues: {issues:?}");
}

#[test]
fn the_store_document_round_trips_through_the_canonical_parser() {
    let config = sample_config();
    let yaml = store_document(&config).expect("serialize");
    assert!(yaml.starts_with("x-openp2p:"), "{yaml}");
    assert!(!yaml.contains("token"));
    let parsed = parse_store(&yaml);
    assert!(!has_errors(&parsed.issues), "{:?}", parsed.issues);
    assert_eq!(parsed.config, config);
}

#[test]
fn a_document_without_x_openp2p_is_not_a_store_document() {
    let parsed = parse_store("version: 1\naccounts: []\n");
    assert!(parsed.issues.iter().any(|issue| issue.code == "x_openp2p_missing"));
}

#[test]
fn legacy_references_and_secret_blocks_are_rejected() {
    let parsed = parse_store(
        "x-openp2p:\n  version: 1\n  accounts:\n    - id: home\n      token_ref: o2p/home\n      gateway: {host: api.openp2p.cn, port: 27183}\n      node: mbp-fa-home\n      public_ip_port: 2065\n      apps: []\n",
    );
    assert!(parsed.issues.iter().any(|issue| issue.code == "legacy_reference"));

    let parsed = parse_store("x-openp2p:\n  version: 1\nx-openp2p-secrets:\n  tokens: {}\n");
    assert!(parsed.issues.iter().any(|issue| issue.code == "secrets_block_removed"));

    let parsed = parse_store("x-openp2p:\n  version: 1\n  proxies: []\n");
    assert!(
        parsed
            .issues
            .iter()
            .any(|issue| issue.code == "mihomo_section_at_top_level")
    );
}

#[test]
fn a_stored_token_is_rejected() {
    let yaml = "x-openp2p:\n  version: 1\n  accounts:\n    - id: home\n      token: \"123\"\n      gateway: {host: api.openp2p.cn, port: 27183}\n      node: mbp-fa-home\n      public_ip_port: 2065\n      apps: []\n";
    let parsed = parse_store(yaml);
    assert!(parsed.issues.iter().any(|issue| issue.code == "invalid_secret"));
}

#[test]
fn duplicate_ports_across_accounts_are_rejected() {
    let mut config = sample_config();
    let mut second = config.accounts[0].clone();
    second.id = "corp".into();
    second.public_ip_port = 2066;
    second.apps[0].src_port = 21001;
    config.accounts.push(second);
    let issues = crate::core::openp2p::schema_validate::normalize_and_validate(&mut config);
    assert!(issues.iter().any(|issue| issue.code == "duplicate_port"));
}

#[test]
fn a_missing_src_port_is_an_error() {
    let mut config = sample_config();
    config.accounts[0].apps[0].src_port = 0;
    let issues = crate::core::openp2p::schema_validate::normalize_and_validate(&mut config);
    assert!(issues.iter().any(|issue| issue.code == "src_port_required"));
}

#[test]
fn x_openp2p_networks_are_rejected() {
    let parsed = parse_store("x-openp2p:\n  version: 1\n  networks: []\n");
    assert!(
        parsed.issues.iter().any(|issue| issue.code == "unknown_field"),
        "{:?}",
        parsed.issues
    );
}

#[test]
fn an_empty_config_is_valid_after_the_last_account_is_deleted() {
    let mut config = OpenP2PConfig::default();
    let issues = crate::core::openp2p::schema_validate::normalize_and_validate(&mut config);
    assert!(!has_errors(&issues), "{issues:?}");
}
