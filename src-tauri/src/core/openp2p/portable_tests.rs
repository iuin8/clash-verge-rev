#![allow(clippy::needless_raw_string_hashes)] // fixtures intentionally use r#"..."#
use super::*;
use crate::cmd::openp2p::{OpenP2PSecretState, build_preview, read_import};
use crate::core::openp2p::profile_render;
use crate::core::openp2p::schema::fixtures::sample_host_key;
use crate::core::openp2p::state::OpenP2PPreviewAction;

const KEY: &str = "-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n-----END OPENSSH PRIVATE KEY-----";
const TOKEN: &str = "1234567890123";

struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "cvr-openp2p-portable-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("scratch");
        Self { root }
    }

    fn store(&self) -> ProfileStore {
        ProfileStore::at(self.root.clone())
    }

    fn vault(&self) -> SecretVault {
        SecretVault::at(self.root.join("openp2p"))
    }

    fn token_file(&self, reference: &str) -> PathBuf {
        self.root.join("openp2p/tokens").join(reference)
    }

    fn key_file(&self, reference: &str) -> PathBuf {
        self.root.join("openp2p/keys").join(reference)
    }

    fn read(&self, path: &std::path::Path) -> std::string::String {
        std::fs::read_to_string(path).expect("read")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[cfg(unix)]
fn mode(path: &std::path::Path) -> u32 {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::metadata(path).expect("metadata").permissions().mode() & 0o777
}

fn policy_document(extra_account: &str, extra_app: &str, extra_proxy: &str) -> std::string::String {
    let host_key = sample_host_key();
    format!(
        r#"version: 1
accounts:
  - id: home
    label: Home
    gateway: {{ host: api.openp2p.cn, port: 27183 }}
    {extra_account}
    apps:
      - name: ssh-home
        protocol: tcp
        peer_node: home-exit-01
        dst_host: 127.0.0.1
        dst_port: 22
        {extra_app}
proxies:
  - name: op2p-home
    account: home
    ssh_user: tunnel
    host_key: "{host_key}"
    {extra_proxy}
networks:
  - name: corp
    account: home
    domains: ["+.corp.example"]
    cidrs: ["10.0.0.0/8"]
    dns: ["10.0.0.53"]
"#
    )
}

fn self_contained_document(token: &str, key: &str) -> std::string::String {
    let host_key = sample_host_key();
    format!(
        r#"accounts:
  - id: home
    gateway: {{ host: api.openp2p.cn, port: 27183 }}
    token: {token}
    apps:
      - name: ssh-home
        protocol: tcp
        peer_node: home-exit-01
        dst_host: 127.0.0.1
        dst_port: 22
proxies:
  - name: op2p-home
    account: home
    ssh_user: tunnel
    private_key: |-
{key}
    host_key: "{host_key}"
networks:
  - account: home
    domains: ["+.corp.example"]
"#
    )
}

fn indent(block: &str) -> std::string::String {
    block
        .lines()
        .map(|line| format!("      {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn a_policy_only_document_generates_stable_device_fields() {
    let parsed = parse_import(&policy_document("", "", ""), None);
    assert!(parsed.issues.is_empty(), "{:?}", parsed.issues);
    let config = &parsed.config;
    let account = &config.accounts[0];
    assert!(valid_node_name(&account.node));
    assert!((PUBLIC_PORT_MIN..=PUBLIC_PORT_MAX).contains(&account.public_ip_port));
    assert_eq!(account.token_ref, "o2p/home/token");
    assert!((SRC_PORT_MIN..=SRC_PORT_MAX).contains(&account.apps[0].src_port));
    assert_eq!(config.proxies[0].private_key_ref, "o2p/home/op2p-home");
    assert!(!parsed.has_mihomo);

    let preview = build_preview(&parsed, None, |_| false, |_| false, OpenP2PImportSourceKind::Local);
    assert!(preview.can_apply);
    assert!(!preview.source.self_contained);
    assert_eq!(preview.source.kind, OpenP2PImportSourceKind::Local);
    assert_eq!(preview.accounts[0].nodes, vec!["op2p-home".to_owned()]);
    assert_eq!(preview.accounts[0].route_count, 4);
    assert_eq!(preview.accounts[0].token, OpenP2PSecretState::Missing);
    assert_eq!(preview.accounts[0].key, OpenP2PSecretState::Missing);
    assert!(preview.accounts[0].warnings.contains(&"token_missing".to_owned()));
}

#[test]
fn a_mihomo_profile_keeps_its_own_proxies_out_of_the_openp2p_store() {
    let scratch = Scratch::new("mihomo");
    let store = scratch.store();
    let vault = scratch.vault();
    let key = indent(KEY);
    let document = format!(
        r#"mixed-port: 7890
proxies:
  - {{ name: airport-node, type: ss, server: 192.0.2.10, port: 8388, cipher: aes-128-gcm, password: redacted }}
proxy-groups:
  - {{ name: PROXY, type: select, proxies: [airport-node] }}
rules:
  - MATCH,PROXY
x-openp2p:
  accounts:
    - id: home
      gateway: {{ host: api.openp2p.cn, port: 27183 }}
      token_ref: o2p/home/token
      apps:
        - {{ name: ssh-home, protocol: tcp, peer_node: home-exit-01, dst_host: 127.0.0.1, dst_port: 22 }}
  proxies:
    - name: op2p-home
      account: home
      ssh_user: tunnel
      private_key_ref: o2p/home/id_ed25519
      host_key: "{host_key}"
x-openp2p-secrets:
  tokens:
    o2p/home/token: "{TOKEN}"
  private_keys:
    o2p/home/id_ed25519: |-
{key}
"#,
        host_key = sample_host_key()
    );
    let parsed = parse_import(&document, None);
    assert!(parsed.issues.is_empty(), "{:?}", parsed.issues);
    assert!(parsed.has_mihomo);
    let mihomo = parsed.mihomo.as_deref().expect("mihomo part");
    assert!(mihomo.contains("mixed-port"));
    assert!(!mihomo.contains("x-openp2p"));
    assert!(!mihomo.contains(TOKEN));
    assert!(!mihomo.contains("PRIVATE KEY"));
    assert_eq!(parsed.config.accounts.len(), 1);
    assert_eq!(parsed.config.proxies.len(), 1);
    assert_eq!(parsed.config.proxies[0].name, "op2p-home");
    assert!(parsed.self_contained());

    let preview = build_preview(&parsed, None, |_| false, |_| false, OpenP2PImportSourceKind::Local);
    assert!(preview.source.contains_mihomo);
    assert!(preview.source.self_contained);
    assert_eq!(preview.accounts[0].token, OpenP2PSecretState::Inline);
    assert_eq!(preview.accounts[0].key, OpenP2PSecretState::Inline);

    ImportTransaction::begin(&store, &vault, &parsed)
        .expect("commit")
        .finish();
    let stored = scratch.read(&store.config_path());
    assert!(!stored.contains("airport-node"));
    assert!(!stored.contains("mixed-port"));
    assert!(!stored.contains(TOKEN));
    assert!(!stored.contains("PRIVATE KEY"));
    assert!(stored.contains("token_ref: o2p/home/token"));
}

#[test]
fn inline_credentials_are_written_0600_and_never_enter_the_source_or_profiles() {
    let scratch = Scratch::new("inline");
    let store = scratch.store();
    let vault = scratch.vault();
    let document = self_contained_document(TOKEN, &indent(KEY));
    let parsed = parse_import(&document, None);
    assert!(parsed.issues.is_empty(), "{:?}", parsed.issues);
    assert!(parsed.config.accounts[0].node.len() >= 8);

    ImportTransaction::begin(&store, &vault, &parsed)
        .expect("commit")
        .finish();
    let token_file = scratch.token_file("o2p/home/token");
    assert_eq!(scratch.read(&token_file), format!("{TOKEN}\n"));
    let key_file = scratch.key_file("o2p/home/op2p-home");
    assert_eq!(scratch.read(&key_file), KEY);
    #[cfg(unix)]
    {
        assert_eq!(mode(&token_file), 0o600);
        assert_eq!(mode(&key_file), 0o600);
        assert_eq!(mode(token_file.parent().expect("parent")), 0o700);
    }

    let source = scratch.read(&store.config_path());
    assert!(!source.contains(TOKEN));
    assert!(!source.contains("PRIVATE KEY"));
    assert!(source.contains("token_ref"));

    let profile = profile_render::render_account_profile_with(&parsed.config, "home", |_| true, |_| true)
        .expect("render")
        .expect("profile");
    assert!(!profile.contains(TOKEN));
    assert!(!profile.contains("PRIVATE KEY"));
    assert!(profile.contains("private-key: openp2p/keys/o2p/home/op2p-home"));
}

#[test]
fn secrets_block_conflicts_and_invalid_secrets_are_reported_without_writes() {
    let scratch = Scratch::new("bad-secret");
    let store = scratch.store();
    let vault = scratch.vault();
    let conflict = r#"accounts:
  - id: home
    gateway: { host: api.openp2p.cn, port: 27183 }
    token: "111111"
    token_ref: o2p/home/token
    apps: []
x-openp2p-secrets:
  tokens:
    o2p/home/token: "222222"
"#;
    let parsed = parse_import(conflict, None);
    assert!(!parsed.can_apply());
    assert!(parsed.issues.iter().any(|issue| issue.code == "secret_conflict"));
    assert!(ImportTransaction::begin(&store, &vault, &parsed).is_err());
    assert!(!scratch.token_file("o2p/home/token").exists());

    let bad_key = r#"accounts:
  - id: home
    gateway: { host: api.openp2p.cn, port: 27183 }
    apps: []
proxies:
  - name: op2p-home
    account: home
    ssh_user: tunnel
    private_key: "garbage"
    host_key: "ssh-ed25519 AAAA"
"#;
    let parsed = parse_import(bad_key, None);
    assert!(parsed.issues.iter().any(|issue| issue.code == "invalid_secret"));

    let bad_token = r#"accounts:
  - id: home
    gateway: { host: api.openp2p.cn, port: 27183 }
    token: "not-a-number"
    apps: []
"#;
    let parsed = parse_import(bad_token, None);
    assert!(parsed.issues.iter().any(|issue| issue.code == "invalid_secret"));
}

#[test]
fn explicit_device_values_are_respected_and_auto_values_are_unique() {
    let document = r#"accounts:
  - id: alpha
    gateway: { host: api.openp2p.cn, port: 27183 }
    node: auto
    public_ip_port: auto
    apps:
      - { name: ssh, protocol: tcp, peer_node: peer-node-01, src_port: auto }
  - id: beta
    gateway: { host: api.openp2p.cn, port: 27183 }
    node: explicit-node
    public_ip_port: 2050
    apps:
      - { name: ssh, protocol: tcp, peer_node: peer-node-02, src_port: 21001 }
"#;
    let parsed = parse_import(document, None);
    assert!(parsed.issues.is_empty(), "{:?}", parsed.issues);
    let alpha = &parsed.config.accounts[0];
    let beta = &parsed.config.accounts[1];
    assert!(valid_node_name(&alpha.node));
    assert_ne!(alpha.node, beta.node);
    assert!((PUBLIC_PORT_MIN..=PUBLIC_PORT_MAX).contains(&alpha.public_ip_port));
    assert_ne!(alpha.public_ip_port, beta.public_ip_port);
    assert_eq!(beta.node, "explicit-node");
    assert_eq!(beta.public_ip_port, 2050);
    assert!((SRC_PORT_MIN..=SRC_PORT_MAX).contains(&alpha.apps[0].src_port));
    assert_eq!(beta.apps[0].src_port, 21001);
    assert_ne!(alpha.apps[0].src_port, beta.apps[0].src_port);
    assert_ne!(alpha.apps[0].src_port, alpha.public_ip_port);

    let again = parse_import(document, Some(&parsed.config));
    assert_eq!(again.config, parsed.config);
}

#[test]
fn invalid_documents_and_unknown_fields_are_rejected() {
    let parsed = parse_import(":\n  - [", None);
    assert!(parsed.issues.iter().any(|issue| issue.code == "yaml_syntax"));

    let parsed = parse_import("foo: bar", None);
    assert!(parsed.issues.iter().any(|issue| issue.code == "openp2p_missing"));

    let unknown = r#"accounts:
  - id: home
    gateway: { host: api.openp2p.cn, port: 27183 }
    password: hunter2
    apps: []
"#;
    let parsed = parse_import(unknown, None);
    assert!(!parsed.can_apply());
    assert!(parsed.issues.iter().any(|issue| issue.code == "unknown_field"));
    let serialized = serde_yaml_ng::to_string(&parsed.config).expect("yaml");
    assert!(!serialized.contains("hunter2"));
}

#[tokio::test]
async fn urls_must_be_unambiguous_https_and_reachable() {
    assert_eq!(read_import(None, None).await.unwrap_err().code, "source_missing");
    assert_eq!(
        read_import(Some("a: 1".into()), Some("https://example.com/x".into()))
            .await
            .unwrap_err()
            .code,
        "source_ambiguous"
    );
    assert_eq!(
        read_import(None, Some("http://example.com/x".into()))
            .await
            .unwrap_err()
            .code,
        "url_insecure"
    );
    assert_eq!(
        read_import(None, Some("not a url".into())).await.unwrap_err().code,
        "url_invalid"
    );
    assert_eq!(
        read_import(None, Some("https://127.0.0.1:1/import.yaml".into()))
            .await
            .unwrap_err()
            .code,
        "fetch_failed"
    );
}

#[test]
fn repeated_apply_is_idempotent_and_policy_only_reimport_keeps_the_device_binding() {
    let scratch = Scratch::new("idempotent");
    let store = scratch.store();
    let vault = scratch.vault();
    let document = self_contained_document(TOKEN, &indent(KEY));
    let first = parse_import(&document, None);
    assert!(first.can_apply(), "{:?}", first.issues);
    ImportTransaction::begin(&store, &vault, &first)
        .expect("first apply")
        .finish();
    let stored = store.load_config().expect("load").expect("config");
    let source_after_first = scratch.read(&store.config_path());
    let token_after_first = scratch.read(&scratch.token_file("o2p/home/token"));

    let second = parse_import(&document, Some(&stored));
    assert_eq!(second.config, stored);
    ImportTransaction::begin(&store, &vault, &second)
        .expect("second apply")
        .finish();
    assert_eq!(store.load_config().expect("load").expect("config"), stored);
    assert_eq!(scratch.read(&store.config_path()), source_after_first);
    assert_eq!(scratch.read(&scratch.token_file("o2p/home/token")), token_after_first);

    let policy = policy_document("", "", "");
    let third = parse_import(&policy, Some(&stored));
    assert!(third.can_apply(), "{:?}", third.issues);
    assert_eq!(third.config.accounts[0].node, stored.accounts[0].node);
    assert_eq!(
        third.config.accounts[0].public_ip_port,
        stored.accounts[0].public_ip_port
    );
    assert_eq!(
        third.config.accounts[0].apps[0].src_port,
        stored.accounts[0].apps[0].src_port
    );
    assert_eq!(third.config.accounts[0].token_ref, stored.accounts[0].token_ref);
    assert_eq!(
        third.config.proxies[0].private_key_ref,
        stored.proxies[0].private_key_ref
    );
    assert_eq!(third.config.networks[0].domains, vec!["corp.example".to_owned()]);
    ImportTransaction::begin(&store, &vault, &third)
        .expect("policy apply")
        .finish();
    assert_eq!(scratch.read(&scratch.token_file("o2p/home/token")), token_after_first);
    assert!(!scratch.read(&store.config_path()).contains(TOKEN));
}

#[test]
fn a_failed_credential_write_rolls_back_earlier_files_and_the_source() {
    let scratch = Scratch::new("rollback");
    let store = scratch.store();
    let vault = scratch.vault();
    let document = self_contained_document(TOKEN, &indent(KEY));
    let parsed = parse_import(&document, None);
    std::fs::create_dir_all(scratch.key_file("o2p/home/op2p-home")).expect("blocking key path");
    let error = ImportTransaction::begin(&store, &vault, &parsed).expect_err("must fail");
    assert!(format!("{error:#}").contains("credential"), "{error:#}");
    assert!(!scratch.token_file("o2p/home/token").exists());
    assert!(store.load_config().expect("load").is_none());
}

#[test]
fn preview_actions_cover_add_update_and_remove() {
    let existing = parse_import(
        r#"accounts:
  - id: alpha
    gateway: { host: api.openp2p.cn, port: 27183 }
    apps:
      - { name: ssh, protocol: tcp, peer_node: peer-node-01 }
  - id: beta
    gateway: { host: api.openp2p.cn, port: 27183 }
    apps:
      - { name: ssh, protocol: tcp, peer_node: peer-node-02 }
"#,
        None,
    );
    assert!(existing.can_apply(), "{:?}", existing.issues);
    let incoming = parse_import(
        r#"accounts:
  - id: alpha
    gateway: { host: gw.example.net, port: 27183 }
    apps:
      - { name: ssh, protocol: tcp, peer_node: peer-node-01 }
  - id: gamma
    gateway: { host: api.openp2p.cn, port: 27183 }
    apps:
      - { name: ssh, protocol: tcp, peer_node: peer-node-03 }
"#,
        Some(&existing.config),
    );
    let preview = build_preview(
        &incoming,
        Some(&existing.config),
        |_| false,
        |_| false,
        OpenP2PImportSourceKind::Local,
    );
    let action = |id: &str| {
        preview
            .accounts
            .iter()
            .find(|account| account.id == id)
            .map(|account| account.action)
            .expect("account")
    };
    assert_eq!(action("alpha"), OpenP2PPreviewAction::Update);
    assert_eq!(action("gamma"), OpenP2PPreviewAction::Add);
    assert_eq!(action("beta"), OpenP2PPreviewAction::Remove);
}

#[test]
fn openp2p_detection_requires_a_schema_marker_not_just_proxies() {
    assert!(is_openp2p_document(
        "mixed-port: 7890\nproxies: []\nx-openp2p:\n  accounts: []\n"
    ));
    assert!(is_openp2p_document("version: 1\naccounts:\n  - id: home\n"));
    assert!(is_openp2p_document(
        "version: 1\nnetworks: []\nproxies:\n  - name: home\n    account: home\n    private_key_ref: o2p/home\n"
    ));
    assert!(!is_openp2p_document(
        "mixed-port: 7890\nproxies:\n  - {name: airport, type: ss, server: 192.0.2.1, port: 8388}\nrules:\n  - MATCH,airport\n"
    ));
    assert!(!is_openp2p_document("not: yaml"));
}
