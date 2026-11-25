#![allow(clippy::needless_raw_string_hashes)] // fixtures intentionally use r#"..."#

use std::path::PathBuf;

use crate::config::IProfiles;
use crate::core::openp2p::portable::OpenP2PImportSourceKind;
use crate::core::openp2p::schema::fixtures::sample_host_key;
use crate::core::openp2p::subscriptions::{
    PROFILE_DESC_PREFIX, SubscriptionStore, id_from_profile_uid, is_subscription_profile_uid,
    single_subscription_member,
};

const TOKEN: &str = "1234567890123";
const TOKEN_TWO: &str = "9876543210987";
const KEY: &str = "-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n-----END OPENSSH PRIVATE KEY-----";

struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "cvr-openp2p-subscriptions-{name}-{}-{}",
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

    fn store(&self) -> SubscriptionStore {
        SubscriptionStore::at(self.root.clone())
    }

    fn vault(&self) -> crate::core::openp2p::portable::SecretVault {
        crate::core::openp2p::portable::SecretVault::at(self.root.join("openp2p"))
    }

    fn read(&self, path: &std::path::Path) -> std::string::String {
        std::fs::read_to_string(path).expect("read")
    }

    fn token_path(&self) -> PathBuf {
        self.root.join("openp2p/tokens/o2p/home/token")
    }

    fn key_path(&self) -> PathBuf {
        self.root.join("openp2p/keys/o2p/home/op2p-home")
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

fn indent(block: &str) -> std::string::String {
    block
        .lines()
        .map(|line| format!("      {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn document(label: &str, token: &str, key: &str) -> std::string::String {
    format!(
        r#"accounts:
  - id: home
    label: {label}
    gateway: {{ host: api.openp2p.cn, port: 27183 }}
    token: {token}
    apps:
      - {{ name: ssh-home, protocol: tcp, peer_node: home-exit-01, dst_host: 127.0.0.1, dst_port: 22 }}
proxies:
  - name: op2p-home
    account: home
    ssh_user: tunnel
    private_key: |-
{key}
    host_key: "{host_key}"
networks:
  - name: corp
    account: home
    domains: ["+.corp.example"]
    cidrs: ["10.0.0.0/8"]
    dns: ["10.0.0.53"]
"#,
        host_key = sample_host_key()
    )
}

fn mixed_document() -> std::string::String {
    let host_key = sample_host_key();
    format!(
        r#"mixed-port: 7890
proxies:
  - {{ name: airport-node, type: ss, server: 192.0.2.10, port: 8388, cipher: aes-128-gcm, password: redacted }}
proxy-groups:
  - {{ name: PROXY, type: select, proxies: [airport-node] }}
rules:
  - MATCH,PROXY
dns:
  nameserver: [119.29.29.29]
x-openp2p:
  accounts:
    - id: alpha
      label: Alpha
      gateway: {{ host: api.openp2p.cn, port: 27183 }}
      apps:
        - {{ name: ssh-alpha, protocol: tcp, peer_node: peer-alpha-01, dst_host: 127.0.0.1, dst_port: 22 }}
    - id: beta
      label: Beta
      gateway: {{ host: api.openp2p.cn, port: 27183 }}
      apps:
        - {{ name: ssh-beta, protocol: tcp, peer_node: peer-beta-01, dst_host: 127.0.0.1, dst_port: 22 }}
  proxies:
    - name: op2p-alpha
      account: alpha
      ssh_user: tunnel
      private_key_ref: o2p/alpha/key
      host_key: "{host_key}"
    - name: op2p-beta
      account: beta
      ssh_user: tunnel
      private_key_ref: o2p/beta/key
      host_key: "{host_key}"
  networks:
    - account: alpha
      domains: ["+.alpha.example"]
      dns: ["10.0.0.53"]
    - account: beta
      domains: ["+.beta.example"]
      dns: ["10.0.0.54"]
x-openp2p-secrets:
  tokens:
    o2p/alpha/token: "111111"
    o2p/beta/token: "222222"
  private_keys:
    o2p/alpha/key: |-
{alpha_key}
    o2p/beta/key: |-
{beta_key}
"#,
        alpha_key = indent(KEY),
        beta_key = indent(KEY),
    )
}

#[test]
#[allow(clippy::cognitive_complexity, reason = "one end-to-end import assertion block")]
fn an_import_writes_private_source_meta_config_and_profile_without_activating() {
    let scratch = Scratch::new("import");
    let store = scratch.store();
    let vault = scratch.vault();
    let mut profiles = IProfiles::default();
    let text = document("Home", TOKEN, &indent(KEY));

    let import = store
        .prepare_import(&vault, OpenP2PImportSourceKind::Local, &text, None, &mut profiles)
        .expect("prepare")
        .commit();
    assert!(import.created);
    assert!(import.items_changed);
    assert!(import.profile_changed);
    assert_eq!(import.meta.account_count, 1);
    assert_eq!(import.meta.name, "OpenP2P Home");
    assert_eq!(import.meta.source, OpenP2PImportSourceKind::Local);
    assert!(profiles.current.is_none());

    // The raw source keeps the imported document, credentials included, and is 0600.
    let source_path = store.source_path(&import.meta.id);
    assert_eq!(scratch.read(&source_path), text);
    #[cfg(unix)]
    assert_eq!(mode(&source_path), 0o600);

    // The normalized config keeps references only.
    let config = store.load_config(&import.meta.id).expect("config");
    assert_eq!(config.accounts[0].token_ref, "o2p/home/token");
    let config_text = scratch.read(&store.config_path(&import.meta.id));
    assert!(!config_text.contains(TOKEN));
    assert!(!config_text.contains("PRIVATE KEY"));

    // Inline credentials reach the vault, never the generated profile.
    assert_eq!(scratch.read(&scratch.token_path()), format!("{TOKEN}\n"));
    assert_eq!(scratch.read(&scratch.key_path()), KEY);
    #[cfg(unix)]
    {
        assert_eq!(mode(&scratch.token_path()), 0o600);
        assert_eq!(mode(&scratch.key_path()), 0o600);
    }
    let profile = scratch.read(&store.profile_path(&import.profile_uid));
    assert!(profile.contains("OP2P-home"));
    assert!(profile.contains("private-key: openp2p/keys/o2p/home/op2p-home"));
    assert!(!profile.contains(TOKEN));
    assert!(!profile.contains("PRIVATE KEY"));
    assert!(!profile.contains("MATCH"));
    assert!(!profile.contains("x-openp2p"));

    let items = profiles.items.as_ref().expect("items");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].uid.as_deref(), Some(import.profile_uid.as_str()));
    assert_eq!(items[0].itype.as_deref(), Some("local"));
    assert_eq!(
        items[0].file.as_deref(),
        Some(format!("{}.yaml", import.profile_uid).as_str())
    );
    assert_eq!(
        items[0].desc.as_deref(),
        Some(format!("{PROFILE_DESC_PREFIX}{}", import.meta.id).as_str())
    );
    assert!(items[0].option.is_none());
    assert_eq!(id_from_profile_uid(&import.profile_uid), Some(import.meta.id.as_str()));
    assert!(is_subscription_profile_uid(&import.profile_uid));
}

#[test]
fn reimporting_keeps_the_subscription_identity_and_generated_device_fields() {
    let scratch = Scratch::new("reimport");
    let store = scratch.store();
    let vault = scratch.vault();
    let mut profiles = IProfiles::default();
    let text = document("Home", TOKEN, &indent(KEY));

    let first = store
        .prepare_import(&vault, OpenP2PImportSourceKind::Local, &text, None, &mut profiles)
        .expect("first")
        .commit();
    let config_before = scratch.read(&store.config_path(&first.meta.id));
    let profile_before = scratch.read(&store.profile_path(&first.profile_uid));

    let again = store
        .prepare_import(&vault, OpenP2PImportSourceKind::Local, &text, None, &mut profiles)
        .expect("second");
    assert!(!again.outcome().created);
    assert!(!again.outcome().items_changed);
    assert!(!again.outcome().profile_changed);
    let second = again.commit();
    assert_eq!(second.meta.id, first.meta.id);
    assert_eq!(scratch.read(&store.config_path(&second.meta.id)), config_before);
    assert_eq!(scratch.read(&store.profile_path(&second.profile_uid)), profile_before);
}

#[test]
fn a_remote_id_is_stable_across_url_spelling() {
    let scratch = Scratch::new("remote");
    let store = scratch.store();
    let vault = scratch.vault();
    let mut profiles = IProfiles::default();
    let text = document("Home", TOKEN, &indent(KEY));

    let first = store
        .prepare_import(
            &vault,
            OpenP2PImportSourceKind::Url,
            &text,
            Some("https://example.com/profiles/sub.yaml"),
            &mut profiles,
        )
        .expect("first")
        .commit();
    assert_eq!(first.meta.name, "example.com");
    assert_eq!(
        first.meta.source_url.as_deref(),
        Some("https://example.com/profiles/sub.yaml")
    );

    let second = store
        .prepare_import(
            &vault,
            OpenP2PImportSourceKind::Url,
            &text,
            Some("  https://EXAMPLE.com/profiles/sub.yaml  "),
            &mut profiles,
        )
        .expect("second");
    assert!(!second.outcome().created);
    assert_eq!(second.commit().meta.id, first.meta.id);
}

#[test]
fn the_rendered_profile_merges_the_mihomo_base_with_every_account() {
    let scratch = Scratch::new("render");
    let store = scratch.store();
    let vault = scratch.vault();
    let mut profiles = IProfiles::default();
    let import = store
        .prepare_import(
            &vault,
            OpenP2PImportSourceKind::Local,
            &mixed_document(),
            None,
            &mut profiles,
        )
        .expect("import")
        .commit();
    assert_eq!(import.meta.account_count, 2);
    let profile = scratch.read(&store.profile_path(&import.profile_uid));

    assert!(profile.contains("mixed-port: 7890"));
    assert!(profile.contains("airport-node"));
    assert!(profile.contains("name: PROXY"));
    assert!(profile.contains("OP2P-alpha"));
    assert!(profile.contains("OP2P-beta"));
    assert!(profile.contains("op2p-alpha"));
    assert!(profile.contains("op2p-beta"));
    assert!(profile.contains("DOMAIN-SUFFIX,alpha.example,OP2P-alpha"));
    assert!(profile.contains("DOMAIN-SUFFIX,beta.example,OP2P-beta"));
    assert!(profile.contains("+.alpha.example"));
    assert!(profile.contains("tcp://10.0.0.53:53#OP2P-alpha"));
    assert!(profile.contains("119.29.29.29"));
    assert_eq!(profile.matches("MATCH,PROXY").count(), 1);
    assert!(
        profile
            .find("DOMAIN-SUFFIX,alpha.example,OP2P-alpha")
            .expect("generated rule")
            < profile.find("MATCH,PROXY").expect("base match")
    );
    assert!(!profile.contains("111111"));
    assert!(!profile.contains("222222"));
    assert!(!profile.contains("PRIVATE KEY"));
    assert!(!profile.contains("x-openp2p"));
}

#[test]
fn the_current_profile_selects_exactly_one_subscription() {
    let scratch = Scratch::new("current");
    let store = scratch.store();
    let vault = scratch.vault();
    let mut profiles = IProfiles::default();
    let first = store
        .prepare_import(
            &vault,
            OpenP2PImportSourceKind::Local,
            &document("One", TOKEN, &indent(KEY)),
            None,
            &mut profiles,
        )
        .expect("first")
        .commit();
    let second = store
        .prepare_import(
            &vault,
            OpenP2PImportSourceKind::Local,
            &credential_document("beta", "Beta", "o2p/beta/token", TOKEN_TWO),
            None,
            &mut profiles,
        )
        .expect("second")
        .commit();
    assert_ne!(first.meta.id, second.meta.id);
    assert_eq!(store.list().expect("list").len(), 2);

    let runtime = store
        .runtime_for_uid(&first.profile_uid)
        .expect("runtime")
        .expect("subscription");
    assert_eq!(runtime.meta.id, first.meta.id);
    assert_eq!(runtime.profile_uid, first.profile_uid);
    assert_eq!(runtime.config, store.load_config(&first.meta.id).expect("config"));
    assert_eq!(runtime.source.name.as_deref(), Some("OpenP2P One"));
    assert_eq!(runtime.source.kind, "local");
    assert!(store.runtime_for_uid("plain-local-profile").expect("runtime").is_none());
    assert!(store.runtime_for_uid(&second.profile_uid).expect("runtime").is_some());
    assert_eq!(
        store
            .find_by_profile_uid(&second.profile_uid)
            .expect("lookup")
            .expect("subscription")
            .id,
        second.meta.id
    );
}

fn credential_document(account: &str, label: &str, token_ref: &str, token: &str) -> std::string::String {
    format!(
        r#"accounts:
  - id: {account}
    label: {label}
    gateway: {{ host: api.openp2p.cn, port: 27183 }}
    token_ref: {token_ref}
    token: {token}
    apps:
      - {{ name: ssh-{account}, protocol: tcp, peer_node: peer-{account}-01, dst_host: 127.0.0.1, dst_port: 22 }}
"#
    )
}

#[test]
fn subscriptions_may_share_an_account_id_when_references_differ() {
    let scratch = Scratch::new("account-ids-shared");
    let store = scratch.store();
    let vault = scratch.vault();
    let mut profiles = IProfiles::default();
    store
        .prepare_import(
            &vault,
            OpenP2PImportSourceKind::Local,
            &credential_document("home", "One", "o2p/one/token", TOKEN),
            None,
            &mut profiles,
        )
        .expect("first")
        .commit();
    store
        .prepare_import(
            &vault,
            OpenP2PImportSourceKind::Local,
            &credential_document("home", "Two", "o2p/two/token", TOKEN_TWO),
            None,
            &mut profiles,
        )
        .expect("shared account id is allowed when refs differ")
        .commit();
    assert_eq!(store.list().expect("list").len(), 2);
}

#[test]
fn subscriptions_may_share_credential_references() {
    let scratch = Scratch::new("refs-shared");
    let store = scratch.store();
    let vault = scratch.vault();
    let mut profiles = IProfiles::default();
    store
        .prepare_import(
            &vault,
            OpenP2PImportSourceKind::Local,
            &credential_document("alpha", "Alpha", "o2p/shared", TOKEN),
            None,
            &mut profiles,
        )
        .expect("first")
        .commit();
    store
        .prepare_import(
            &vault,
            OpenP2PImportSourceKind::Local,
            &credential_document("beta", "Beta", "o2p/shared", TOKEN_TWO),
            None,
            &mut profiles,
        )
        .expect("shared references are allowed")
        .commit();
    assert_eq!(store.list().expect("list").len(), 2);
}

#[test]
fn a_bundle_resolves_at_most_one_subscription_member() {
    let plain = "local".to_owned();
    let one = "op2p-sub-0123456789abcdef".to_owned();
    let two = "op2p-sub-fedcba9876543210".to_owned();

    assert_eq!(
        single_subscription_member(&[plain.clone(), one.clone()]).expect("one member"),
        Some(one.clone())
    );
    assert_eq!(
        single_subscription_member(std::slice::from_ref(&plain)).expect("no member"),
        None
    );
    let error = single_subscription_member(&[one, two]).expect_err("two members must be rejected");
    assert!(
        format!("{error:#}").contains("at most one OpenP2P subscription"),
        "{error:#}"
    );
}

#[test]
fn removing_a_subscription_keeps_an_account_workdir_shared_with_another_subscription() {
    let scratch = Scratch::new("shared-workdir");
    let store = scratch.store();
    let vault = scratch.vault();
    let mut profiles = IProfiles::default();
    let alpha = store
        .prepare_import(
            &vault,
            OpenP2PImportSourceKind::Local,
            &credential_document("shared", "Alpha", "o2p/alpha/token", TOKEN),
            None,
            &mut profiles,
        )
        .expect("alpha")
        .commit();
    let beta = store
        .prepare_import(
            &vault,
            OpenP2PImportSourceKind::Local,
            &credential_document("shared", "Beta", "o2p/beta/token", TOKEN_TWO),
            None,
            &mut profiles,
        )
        .expect("beta")
        .commit();
    let account_dir = scratch.root.join("openp2p/accounts/shared");
    std::fs::create_dir_all(&account_dir).expect("account dir");

    store.remove(&alpha.meta.id).expect("remove alpha");
    assert!(account_dir.exists(), "the shared workdir is used by beta");
    store.remove(&beta.meta.id).expect("remove beta");
    assert!(!account_dir.exists(), "the last owner removes the workdir");
}

#[test]
fn removing_a_subscription_removes_its_store_account_dirs_and_credentials() {
    let scratch = Scratch::new("remove");
    let store = scratch.store();
    let vault = scratch.vault();
    let mut profiles = IProfiles::default();
    let import = store
        .prepare_import(
            &vault,
            OpenP2PImportSourceKind::Local,
            &document("Home", TOKEN, &indent(KEY)),
            None,
            &mut profiles,
        )
        .expect("import")
        .commit();
    let account_dir = scratch.root.join("openp2p/accounts/home");
    std::fs::create_dir_all(&account_dir).expect("account dir");
    std::fs::write(account_dir.join("config.json"), "{}").expect("config.json");

    store.remove(&import.meta.id).expect("remove");
    assert!(!store.source_path(&import.meta.id).exists());
    assert!(!store.config_path(&import.meta.id).exists());
    assert!(!account_dir.exists());
    assert!(!scratch.token_path().exists());
    assert!(!scratch.key_path().exists());
    assert!(store.list().expect("list").is_empty());
    assert!(
        store
            .find_by_profile_uid(&import.profile_uid)
            .expect("lookup")
            .is_none()
    );
}

#[test]
fn removing_one_subscription_leaves_another_intact() {
    let scratch = Scratch::new("remove-isolated");
    let store = scratch.store();
    let vault = scratch.vault();
    let mut profiles = IProfiles::default();
    let alpha = store
        .prepare_import(
            &vault,
            OpenP2PImportSourceKind::Local,
            &credential_document("alpha", "Alpha", "o2p/alpha/token", TOKEN),
            None,
            &mut profiles,
        )
        .expect("alpha")
        .commit();
    let beta = store
        .prepare_import(
            &vault,
            OpenP2PImportSourceKind::Local,
            &credential_document("beta", "Beta", "o2p/beta/token", TOKEN_TWO),
            None,
            &mut profiles,
        )
        .expect("beta")
        .commit();
    let alpha_token = scratch.root.join("openp2p/tokens/o2p/alpha/token");
    let beta_token = scratch.root.join("openp2p/tokens/o2p/beta/token");
    assert!(alpha_token.exists() && beta_token.exists());

    store.remove(&alpha.meta.id).expect("remove alpha");
    assert!(!alpha_token.exists());
    assert!(beta_token.exists());
    assert!(store.source_path(&beta.meta.id).exists());
    assert!(store.config_path(&beta.meta.id).exists());
    assert!(store.profile_path(&beta.profile_uid).exists());
    assert_eq!(store.list().expect("list").len(), 1);
}

#[test]
fn a_rolled_back_import_restores_the_previous_subscription_files() {
    let scratch = Scratch::new("rollback");
    let store = scratch.store();
    let vault = scratch.vault();
    let mut profiles = IProfiles::default();
    let first = store
        .prepare_import(
            &vault,
            OpenP2PImportSourceKind::Local,
            &document("Home", TOKEN, &indent(KEY)),
            None,
            &mut profiles,
        )
        .expect("first")
        .commit();
    let source_before = scratch.read(&store.source_path(&first.meta.id));
    let config_before = scratch.read(&store.config_path(&first.meta.id));
    let profile_before = scratch.read(&store.profile_path(&first.profile_uid));
    let token_before = scratch.read(&scratch.token_path());

    let updated = format!(
        r#"accounts:
  - id: home
    label: Home
    gateway: {{ host: api.openp2p.cn, port: 27183 }}
    token: {TOKEN_TWO}
    apps:
      - {{ name: ssh-home, protocol: tcp, peer_node: home-exit-01, dst_host: 127.0.0.1, dst_port: 22 }}
proxies:
  - name: op2p-home
    account: home
    ssh_user: tunnel
    private_key: |-
{key}
    host_key: "{host_key}"
networks:
  - name: corp
    account: home
    domains: ["+.other.example"]
"#,
        key = indent(KEY),
        host_key = sample_host_key()
    );
    let prepared = store
        .prepare_update(&vault, &updated, &first.meta.id, &mut profiles)
        .expect("prepare update");
    assert!(prepared.outcome().profile_changed);
    assert_eq!(scratch.read(&scratch.token_path()), format!("{TOKEN_TWO}\n"));
    prepared.rollback().expect("rollback");

    assert_eq!(scratch.read(&store.source_path(&first.meta.id)), source_before);
    assert_eq!(scratch.read(&store.config_path(&first.meta.id)), config_before);
    assert_eq!(scratch.read(&store.profile_path(&first.profile_uid)), profile_before);
    assert_eq!(scratch.read(&scratch.token_path()), token_before);
}

#[test]
fn a_rolled_back_new_import_removes_the_subscription_and_credentials() {
    let scratch = Scratch::new("rollback-new");
    let store = scratch.store();
    let vault = scratch.vault();
    let mut profiles = IProfiles::default();
    let prepared = store
        .prepare_import(
            &vault,
            OpenP2PImportSourceKind::Local,
            &document("Home", TOKEN, &indent(KEY)),
            None,
            &mut profiles,
        )
        .expect("prepare");
    let id = prepared.outcome().meta.id.clone();
    prepared.rollback().expect("rollback");
    assert!(!store.source_path(&id).exists());
    assert!(!store.meta_path(&id).exists());
    assert!(!scratch.token_path().exists());
    assert!(!scratch.key_path().exists());
    assert!(store.list().expect("list").is_empty());
}

#[test]
fn a_failed_credential_write_leaves_no_subscription_files() {
    let scratch = Scratch::new("bad-secret-write");
    let store = scratch.store();
    let vault = scratch.vault();
    let mut profiles = IProfiles::default();
    std::fs::create_dir_all(scratch.key_path()).expect("blocking key path");
    store
        .prepare_import(
            &vault,
            OpenP2PImportSourceKind::Local,
            &document("Home", TOKEN, &indent(KEY)),
            None,
            &mut profiles,
        )
        .expect_err("credential write must fail");
    assert!(!scratch.token_path().exists());
    assert!(store.list().expect("list").is_empty());
}
