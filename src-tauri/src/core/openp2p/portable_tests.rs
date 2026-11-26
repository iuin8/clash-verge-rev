#![allow(clippy::needless_raw_string_hashes)] // fixtures intentionally use r#"..."#
use super::*;
use crate::cmd::openp2p::read_import;
use crate::core::openp2p::schema::fixtures::sample_host_key;

const KEY: &str = "-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n-----END OPENSSH PRIVATE KEY-----";
const OTHER_KEY: &str = "-----BEGIN OPENSSH PRIVATE KEY-----\nxyz\n-----END OPENSSH PRIVATE KEY-----";
const TOKEN: &str = "1234567890123";
const SUBSCRIPTION_ID: &str = "0123456789abcdef";

fn indent_by(block: &str, width: usize) -> std::string::String {
    block
        .lines()
        .map(|line| format!("{}{line}", " ".repeat(width)))
        .collect::<Vec<_>>()
        .join("\n")
}

fn indent(block: &str) -> std::string::String {
    indent_by(block, 6)
}

fn document() -> std::string::String {
    let host_key = sample_host_key();
    format!(
        r#"mixed-port: 7890
proxies:
  - name: op2p-home
    type: ssh
    server: 127.0.0.1
    port: 21001
    username: tunnel
    private-key: |-
{key}
    host-key: "{host_key}"
  - {{ name: airport-node, type: ss, server: 192.0.2.10, port: 8388, cipher: aes-128-gcm, password: redacted }}
proxy-groups:
  - {{ name: PROXY, type: select, proxies: [airport-node] }}
rules:
  - MATCH,PROXY
dns:
  nameserver: [119.29.29.29]
x-openp2p:
  version: 1
  name: Home
  accounts:
    - id: home
      label: Home
      token: {TOKEN}
      gateway: {{ host: api.openp2p.cn, port: 27183 }}
      apps:
        - name: ssh-home
          protocol: tcp
          src_port: 21001
          peer_node: home-exit-01
          dst_host: 127.0.0.1
          dst_port: 22
"#,
        key = indent(KEY),
    )
}

#[test]
fn canonical_detection_requires_top_level_x_openp2p() {
    assert!(is_openp2p_document(&document()));
    assert!(is_openp2p_document("x-openp2p:\n  version: 1\n"));
    assert!(!is_openp2p_document(
        "accounts:\n  - id: home\nproxies:\n  - {name: home, account: home, ssh_user: tunnel}\n"
    ));
    assert!(!is_openp2p_document(
        "mixed-port: 7890\nproxies:\n  - {name: airport, type: ss, server: 192.0.2.1, port: 8388}\nrules:\n  - MATCH,airport\n"
    ));
    assert!(!is_openp2p_document("not: yaml"));
}

#[test]
fn an_import_stages_credentials_and_projects_the_mihomo_document() {
    let parsed = parse_import(&document(), None, SUBSCRIPTION_ID);
    assert!(parsed.can_apply(), "{:?}", parsed.issues);
    assert!(parsed.has_mihomo);
    assert!(parsed.tokens().any(|(id, _)| id == "home"));
    assert_eq!(parsed.config.accounts[0].token, None);
    assert_eq!(parsed.config.accounts[0].apps[0].src_port, 21001);
    assert_eq!(parsed.config.name.as_deref(), Some("Home"));

    let keys: Vec<(&str, &str)> = parsed.keys().collect();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].1, KEY);

    let profile = parsed.profile_yaml().expect("projection");
    assert!(profile.contains("mixed-port: 7890"));
    assert!(profile.contains("airport-node"));
    assert!(profile.contains(&format!("private-key: openp2p/subscriptions/{SUBSCRIPTION_ID}/keys/")));
    assert!(!profile.contains("x-openp2p"));
    assert!(!profile.contains(TOKEN));
    assert!(!profile.contains("PRIVATE KEY"));
    assert!(!profile.contains("OP2P-home"));
    assert!(!profile.contains("DOMAIN-SUFFIX"));
    assert_eq!(profile.matches("MATCH,PROXY").count(), 1);
    let projected: Value = serde_yaml_ng::from_str(profile).expect("projected profile is yaml");
    assert!(
        projected["proxies"][0]["host-key"].is_sequence(),
        "the managed host-key is normalized to mihomo's list form"
    );
}

#[test]
fn legacy_document_shapes_fail_explicitly() {
    let parsed = parse_import("accounts:\n  - id: home\n", None, SUBSCRIPTION_ID);
    assert!(parsed.issues.iter().any(|issue| issue.code == "x_openp2p_missing"));

    let parsed = parse_import(
        "x-openp2p:\n  version: 1\n  accounts:\n    - id: home\n      token_ref: o2p/home\n      gateway: {host: api.openp2p.cn, port: 27183}\n      node: mbp-fa-home\n      public_ip_port: 2065\n      apps: []\n",
        None,
        SUBSCRIPTION_ID,
    );
    assert!(parsed.issues.iter().any(|issue| issue.code == "legacy_reference"));

    let parsed = parse_import(
        "x-openp2p:\n  version: 1\nx-openp2p-secrets:\n  tokens: {}\n",
        None,
        SUBSCRIPTION_ID,
    );
    assert!(parsed.issues.iter().any(|issue| issue.code == "secrets_block_removed"));

    let parsed = parse_import("x-openp2p:\n  version: 1\n  proxies: []\n", None, SUBSCRIPTION_ID);
    assert!(
        parsed
            .issues
            .iter()
            .any(|issue| issue.code == "mihomo_section_at_top_level")
    );

    let parsed = parse_import(
        "proxies:\n  - {name: op2p-home, account: home, ssh_user: tunnel, private_key: x}\nx-openp2p:\n  version: 1\n",
        None,
        SUBSCRIPTION_ID,
    );
    assert!(parsed.issues.iter().any(|issue| issue.code == "legacy_reference"));
}

#[test]
fn removed_root_fields_are_rejected() {
    let enabled = document().replace("  version: 1\n", "  version: 1\n  enabled: false\n");
    let parsed = parse_import(&enabled, None, SUBSCRIPTION_ID);
    assert!(!parsed.can_apply());
    assert!(
        parsed
            .issues
            .iter()
            .any(|issue| issue.code == "unknown_field" || issue.code == "schema_invalid")
    );
}

#[test]
fn x_openp2p_networks_are_rejected() {
    let with_networks = document().replace(
        "  name: Home\n",
        "  name: Home\n  networks:\n    - name: corp\n      account: home\n      domains: [\"+.corp.example\"]\n      cidrs: [\"10.0.0.0/8\"]\n",
    );
    let parsed = parse_import(&with_networks, None, SUBSCRIPTION_ID);
    assert!(!parsed.can_apply());
    assert!(
        parsed.issues.iter().any(|issue| issue.code == "unknown_field"),
        "{:?}",
        parsed.issues
    );
}

#[test]
fn a_managed_proxy_requires_username_host_key_and_inline_key() {
    let missing_username = document().replace("    username: tunnel\n", "");
    let parsed = parse_import(&missing_username, None, SUBSCRIPTION_ID);
    assert!(parsed.issues.iter().any(|issue| issue.code == "invalid_ssh_user"));

    let missing_host_key = document().replace(&format!("    host-key: \"{}\"\n", sample_host_key()), "");
    let parsed = parse_import(&missing_host_key, None, SUBSCRIPTION_ID);
    assert!(parsed.issues.iter().any(|issue| issue.code == "host_key_missing"));

    let bad_host_key = document().replace(
        &format!("    host-key: \"{}\"", sample_host_key()),
        "    host-key: \"ssh-ed25519 not-base64\"",
    );
    let parsed = parse_import(&bad_host_key, None, SUBSCRIPTION_ID);
    assert!(parsed.issues.iter().any(|issue| issue.code == "host_key_invalid"));

    let with_password = document().replace(
        "    username: tunnel\n",
        "    username: tunnel\n    password: hunter2\n",
    );
    let parsed = parse_import(&with_password, None, SUBSCRIPTION_ID);
    assert!(
        parsed
            .issues
            .iter()
            .any(|issue| issue.code == "proxy_password_not_allowed")
    );

    let without_key = document().replace(&format!("    private-key: |-\n{}\n", indent(KEY)), "");
    let parsed = parse_import(&without_key, None, SUBSCRIPTION_ID);
    assert!(parsed.issues.iter().any(|issue| issue.code == "private_key_missing"));
}

#[test]
fn an_unmatched_loopback_ssh_proxy_is_only_a_warning() {
    let unmatched = document().replace("    port: 21001\n", "    port: 21999\n");
    let parsed = parse_import(&unmatched, None, SUBSCRIPTION_ID);
    assert!(parsed.can_apply(), "{:?}", parsed.issues);
    assert!(
        parsed
            .issues
            .iter()
            .any(|issue| issue.code == "proxy_app_unmatched" && !issue.is_error())
    );
}

#[test]
fn device_fields_are_generated_and_stable_across_reimports() {
    let generated = document().replace(
        "      label: Home\n",
        "      label: Home\n      node: auto\n      public_ip_port: auto\n",
    );
    let first = parse_import(&generated, None, SUBSCRIPTION_ID);
    assert!(first.can_apply(), "{:?}", first.issues);
    assert!(valid_node_name(&first.config.accounts[0].node));
    assert!((PUBLIC_PORT_MIN..=PUBLIC_PORT_MAX).contains(&first.config.accounts[0].public_ip_port));

    let again = parse_import(&generated, Some(&first.config), SUBSCRIPTION_ID);
    assert_eq!(again.config.accounts[0].node, first.config.accounts[0].node);
    assert_eq!(
        again.config.accounts[0].public_ip_port,
        first.config.accounts[0].public_ip_port
    );
}

#[test]
fn src_port_is_required_and_never_allocated() {
    let without_port = document().replace("          src_port: 21001\n", "");
    let parsed = parse_import(&without_port, None, SUBSCRIPTION_ID);
    assert!(!parsed.can_apply());
    assert!(parsed.issues.iter().any(|issue| issue.code == "src_port_required"));
}

#[test]
fn inline_keys_in_proxy_provider_payloads_are_staged_too() {
    let host_key = sample_host_key();
    let provider = format!(
        r#"proxy-providers:
  inline:
    type: inline
    payload:
      - name: provider-ssh
        type: ssh
        server: 192.0.2.6
        port: 22
        username: root
        private-key: |-
{key}
        host-key: "{host_key}"
"#,
        key = indent_by(OTHER_KEY, 10)
    );
    let document = format!("{}{}", provider, document().replace("mixed-port: 7890\n", ""));
    let parsed = parse_import(&document, None, SUBSCRIPTION_ID);
    assert!(parsed.can_apply(), "{:?}", parsed.issues);
    assert_eq!(
        parsed.keys().count(),
        2,
        "the proxy and the provider payload stage different keys"
    );
    let profile = parsed.profile_yaml().expect("projection");
    assert!(!profile.contains("PRIVATE KEY"));
    assert_eq!(
        profile
            .matches(&format!("openp2p/subscriptions/{SUBSCRIPTION_ID}/keys/"))
            .count(),
        2
    );
}

#[test]
fn a_private_key_body_that_cannot_be_staged_is_caught_by_the_positive_scan() {
    let block = format!("    private-key: |-\n{}\n", indent(KEY));
    let replaced = document().replace(&block, "    private-key: [\"-----BEGIN PRIVATE KEY-----\"]\n");
    let parsed = parse_import(&replaced, None, SUBSCRIPTION_ID);
    assert!(!parsed.can_apply());
    assert!(parsed.issues.iter().any(|issue| issue.code == "invalid_secret"));
}

#[test]
fn a_key_body_that_is_not_pem_is_rejected() {
    let broken = document().replace(&indent(KEY), "      PRIVATE KEY without a BEGIN marker");
    let parsed = parse_import(&broken, None, SUBSCRIPTION_ID);
    assert!(!parsed.can_apply());
    assert!(parsed.issues.iter().any(|issue| issue.code == "invalid_secret"));
}

#[tokio::test]
async fn urls_must_be_unambiguous_https_and_reachable() {
    assert_eq!(read_import(None, None, None).await.unwrap_err().code, "source_missing");
    assert_eq!(
        read_import(Some("a: 1".into()), Some("https://example.com/x".into()), None)
            .await
            .unwrap_err()
            .code,
        "source_ambiguous"
    );
    assert_eq!(
        read_import(None, Some("http://example.com/x".into()), None)
            .await
            .unwrap_err()
            .code,
        "url_insecure"
    );
    assert_eq!(
        read_import(None, Some("not a url".into()), None)
            .await
            .unwrap_err()
            .code,
        "url_invalid"
    );
    assert_eq!(
        read_import(None, Some("https://127.0.0.1:1/import.yaml".into()), None)
            .await
            .unwrap_err()
            .code,
        "fetch_failed"
    );
}
