use super::fixtures::{sample_config, sample_host_key};
use super::*;

#[test]
fn sample_config_validates_cleanly() {
    let mut config = sample_config();
    let issues = normalize_and_validate(&mut config);
    assert!(!has_errors(&issues), "unexpected issues: {issues:?}");
}

#[test]
fn a_profile_top_level_seed_is_extracted() {
    let yaml = format!(
        "proxies: []\nx-openp2p:\n  version: 1\n  accounts:\n    - id: home\n      gateway: {{host: api.openp2p.cn, port: 27183}}\n      node: mbp-fa-home\n      token_ref: o2p/home\n      public_ip_port: 2065\n      apps:\n        - {{name: ssh-home, protocol: tcp, src_port: 21001, peer_node: home-exit-01, dst_host: 127.0.0.1, dst_port: 22, enabled: 1}}\n  proxies:\n    - name: op2p-home\n      account: home\n      app: ssh-home\n      ssh_user: tunnel\n      private_key_ref: o2p/home/id_ed25519\n      host_key: \"{}\"\n  networks:\n    - domains: [corp.example]\n",
        sample_host_key()
    );
    let parsed = parse_seed(&yaml);
    assert!(!has_errors(&parsed.issues), "{:?}", parsed.issues);
    assert_eq!(parsed.config.accounts.len(), 1);
    assert_eq!(parsed.config.accounts[0].apps[0].src_port, 21001);
}

#[test]
fn a_token_value_is_rejected_but_token_ref_is_accepted() {
    let yaml = "version: 1\naccounts:\n  - id: home\n    token: 12345\n    token_ref: o2p/home\n";
    let parsed = parse_seed(yaml);
    assert!(parsed.issues.iter().any(|issue| issue.code == "token_in_config"));
}

#[test]
fn duplicate_ports_across_accounts_are_rejected() {
    let mut config = sample_config();
    let mut second = config.accounts[0].clone();
    second.id = "corp".into();
    second.token_ref = "o2p/corp".into();
    second.public_ip_port = 2066;
    second.apps[0].src_port = 21001;
    config.accounts.push(second);
    let issues = normalize_and_validate(&mut config);
    assert!(issues.iter().any(|issue| issue.code == "duplicate_port"));
}

#[test]
fn traversal_references_are_rejected() {
    let mut config = sample_config();
    config.accounts[0].token_ref = "../etc/passwd".into();
    config.proxies[0].private_key_ref = "o2p/../../key".into();
    let issues = normalize_and_validate(&mut config);
    assert!(issues.iter().any(|issue| issue.code == "token_ref_invalid"));
    assert!(issues.iter().any(|issue| issue.code == "key_ref_invalid"));
}

#[test]
fn a_garbage_host_key_is_rejected() {
    let mut config = sample_config();
    config.proxies[0].host_key = Some("ssh-ed25519 not-base64".into());
    let issues = normalize_and_validate(&mut config);
    assert!(issues.iter().any(|issue| issue.code == "host_key_invalid"));
}

#[test]
fn a_multi_app_account_requires_an_explicit_app() {
    let mut config = sample_config();
    let mut second_app = config.accounts[0].apps[0].clone();
    second_app.name = "other".into();
    second_app.src_port = 21002;
    config.accounts[0].apps.push(second_app);
    config.proxies[0].app = None;
    let issues = normalize_and_validate(&mut config);
    assert!(issues.iter().any(|issue| issue.code == "proxy_app_ambiguous"));
}
#[test]
fn an_empty_config_is_valid_after_the_last_account_is_deleted() {
    let mut config = OpenP2PConfig::default();
    let issues = normalize_and_validate(&mut config);
    assert!(!has_errors(&issues), "{issues:?}");
}
