#![allow(clippy::needless_raw_string_hashes)] // fixtures intentionally use r#"..."#

use std::path::PathBuf;

use crate::config::IProfiles;
use crate::core::openp2p::portable::OpenP2PImportSourceKind;
use crate::core::openp2p::schema::fixtures::sample_host_key;
use crate::core::openp2p::secrets::key_digest;
use crate::core::openp2p::subscriptions::{
    PROFILE_DESC_PREFIX, SubscriptionStore, id_from_profile_uid, is_subscription_profile_uid,
    single_subscription_member,
};

const TOKEN: &str = "1234567890123";
const TOKEN_TWO: &str = "9876543210987";
const KEY: &str = "-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n-----END OPENSSH PRIVATE KEY-----";
const OTHER_KEY: &str = "-----BEGIN OPENSSH PRIVATE KEY-----\nxyz\n-----END OPENSSH PRIVATE KEY-----";

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

fn indent_by(block: &str, width: usize) -> std::string::String {
    block
        .lines()
        .map(|line| format!("{}{line}", " ".repeat(width)))
        .collect::<Vec<_>>()
        .join("\n")
}

fn document(label: &str, token: Option<&str>, ordinary_proxy: bool) -> std::string::String {
    let token = token.map_or_else(std::string::String::new, |token| format!("      token: {token}\n"));
    let ordinary = if ordinary_proxy {
        format!(
            r#"  - name: ordinary-ssh
    type: ssh
    server: 192.0.2.9
    port: 22
    username: root
    private-key: |-
{key}
"#,
            key = indent_by(OTHER_KEY, 6)
        )
    } else {
        std::string::String::new()
    };
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
{ordinary}rules:
  - MATCH,DIRECT
x-openp2p:
  version: 1
  name: {label}
  accounts:
    - id: home
      label: {label}
{token}      gateway: {{ host: api.openp2p.cn, port: 27183 }}
      apps:
        - name: ssh-home
          protocol: tcp
          src_port: 21001
          peer_node: home-exit-01
          dst_host: 127.0.0.1
          dst_port: 22
  networks:
    - name: corp
      account: home
      domains: ["+.corp.example"]
      cidrs: ["10.0.0.0/8"]
"#,
        key = indent_by(KEY, 6),
        host_key = sample_host_key(),
    )
}

#[test]
#[allow(clippy::cognitive_complexity, reason = "one end-to-end import assertion block")]
fn an_import_writes_private_source_meta_config_and_profile_without_activating() {
    let scratch = Scratch::new("import");
    let store = scratch.store();
    let mut profiles = IProfiles::default();
    let text = document("Home", Some(TOKEN), true);

    let import = store
        .prepare_import(OpenP2PImportSourceKind::Local, &text, None, &mut profiles)
        .expect("prepare")
        .commit();
    assert!(import.created);
    assert!(import.items_changed);
    assert!(import.profile_changed);
    assert_eq!(import.meta.account_count, 1);
    assert_eq!(import.meta.name, "Home");
    assert_eq!(import.meta.source, OpenP2PImportSourceKind::Local);
    assert!(profiles.current.is_none());

    // The raw source keeps the imported document, credentials included, and is 0600.
    let source_path = store.source_path(&import.meta.id);
    assert_eq!(scratch.read(&source_path), text);
    #[cfg(unix)]
    assert_eq!(mode(&source_path), 0o600);

    // The normalized config is the re-parseable x-openp2p wrapper, token-free.
    let config = store.load_config(&import.meta.id).expect("config");
    assert_eq!(config.accounts[0].token, None);
    let config_text = scratch.read(&store.config_path(&import.meta.id));
    assert!(config_text.starts_with("x-openp2p:"));
    assert!(!config_text.contains(TOKEN));
    assert!(!config_text.contains("PRIVATE KEY"));
    assert!(!config_text.contains("token_ref"));

    // Inline credentials are staged below the subscription directory.
    let token_path = store.token_path(&import.meta.id, "home").expect("token path");
    let key_path = store
        .secrets(&import.meta.id)
        .key_path(&key_digest(KEY))
        .expect("key path");
    assert_eq!(scratch.read(&token_path), format!("{TOKEN}\n"));
    assert_eq!(scratch.read(&key_path), KEY);
    #[cfg(unix)]
    {
        assert_eq!(mode(&token_path), 0o600);
        assert_eq!(mode(&key_path), 0o600);
        assert_eq!(mode(token_path.parent().expect("parent")), 0o700);
    }
    let manifest = scratch.read(&store.secrets(&import.meta.id).manifest_path());
    assert!(manifest.contains("home"));
    assert!(manifest.contains(&key_digest(KEY)));
    assert!(manifest.contains(&key_digest(OTHER_KEY)));

    let profile = scratch.read(&store.profile_path(&import.profile_uid));
    assert!(profile.contains("mixed-port: 7890"));
    assert!(profile.contains("ordinary-ssh"));
    assert!(profile.contains(&format!(
        "private-key: openp2p/subscriptions/{}/keys/{}.key",
        import.meta.id,
        key_digest(KEY)
    )));
    assert!(!profile.contains(TOKEN));
    assert!(!profile.contains("PRIVATE KEY"));
    assert!(!profile.contains("OP2P-home"));
    assert!(!profile.contains("DOMAIN-SUFFIX"));
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
    let mut profiles = IProfiles::default();
    let text = document("Home", Some(TOKEN), true);

    let first = store
        .prepare_import(OpenP2PImportSourceKind::Local, &text, None, &mut profiles)
        .expect("first")
        .commit();
    let config_before = scratch.read(&store.config_path(&first.meta.id));
    let profile_before = scratch.read(&store.profile_path(&first.profile_uid));

    let again = store
        .prepare_import(OpenP2PImportSourceKind::Local, &text, None, &mut profiles)
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
fn an_update_that_omits_the_token_keeps_the_staged_token_and_cleans_stale_keys() {
    let scratch = Scratch::new("retain-token");
    let store = scratch.store();
    let mut profiles = IProfiles::default();
    let first = store
        .prepare_import(
            OpenP2PImportSourceKind::Local,
            &document("Home", Some(TOKEN), true),
            None,
            &mut profiles,
        )
        .expect("first")
        .commit();
    let token_path = store.token_path(&first.meta.id, "home").expect("token path");
    let stale_key = store
        .secrets(&first.meta.id)
        .key_path(&key_digest(OTHER_KEY))
        .expect("stale key");
    assert!(stale_key.is_file());

    let updated = document("Home", None, false);
    let second = store
        .prepare_update(&updated, &first.meta.id, &mut profiles)
        .expect("update")
        .commit();
    assert!(!second.created);
    assert!(second.profile_changed);
    assert_eq!(scratch.read(&token_path), format!("{TOKEN}\n"));
    assert!(token_path.is_file(), "an omitted token keeps the staged token");
    assert!(!stale_key.exists(), "the removed ordinary proxy drops its key");
    assert!(
        store
            .secrets(&first.meta.id)
            .key_path(&key_digest(KEY))
            .expect("key")
            .is_file()
    );
    let runtime = store
        .runtime_for_uid(&first.profile_uid)
        .expect("runtime")
        .expect("subscription");
    assert!(
        runtime.source.self_contained,
        "an omitted token still counts through the staged token file"
    );
}

#[test]
fn a_remote_id_is_stable_across_url_spelling() {
    let scratch = Scratch::new("remote");
    let store = scratch.store();
    let mut profiles = IProfiles::default();
    let text = document("Home", Some(TOKEN), true);

    let first = store
        .prepare_import(
            OpenP2PImportSourceKind::Url,
            &text,
            Some("https://example.com/profiles/sub.yaml"),
            &mut profiles,
        )
        .expect("first")
        .commit();
    assert_eq!(first.meta.name, "Home");
    assert_eq!(
        first.meta.source_url.as_deref(),
        Some("https://example.com/profiles/sub.yaml")
    );

    let second = store
        .prepare_import(
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
fn the_current_profile_selects_exactly_one_subscription() {
    let scratch = Scratch::new("current");
    let store = scratch.store();
    let mut profiles = IProfiles::default();
    let first = store
        .prepare_import(
            OpenP2PImportSourceKind::Local,
            &document("One", Some(TOKEN), true),
            None,
            &mut profiles,
        )
        .expect("first")
        .commit();
    let second = store
        .prepare_import(
            OpenP2PImportSourceKind::Local,
            &document("Two", Some(TOKEN_TWO), true),
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
    assert_eq!(runtime.source.name.as_deref(), Some("One"));
    assert_eq!(runtime.source.kind, "local");
    assert_eq!(runtime.source.version, Some(1));
    assert!(runtime.source.contains_mihomo);
    assert!(runtime.source.self_contained);
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

#[test]
fn subscriptions_may_share_an_account_id_and_keep_independent_tokens() {
    let scratch = Scratch::new("account-ids-shared");
    let store = scratch.store();
    let mut profiles = IProfiles::default();
    let alpha = store
        .prepare_import(
            OpenP2PImportSourceKind::Local,
            &document("One", Some(TOKEN), true),
            None,
            &mut profiles,
        )
        .expect("first")
        .commit();
    let beta = store
        .prepare_import(
            OpenP2PImportSourceKind::Local,
            &document("Two", Some(TOKEN_TWO), true),
            None,
            &mut profiles,
        )
        .expect("second")
        .commit();
    assert_eq!(store.list().expect("list").len(), 2);
    assert_eq!(
        scratch.read(&store.token_path(&alpha.meta.id, "home").expect("alpha token")),
        format!("{TOKEN}\n")
    );
    assert_eq!(
        scratch.read(&store.token_path(&beta.meta.id, "home").expect("beta token")),
        format!("{TOKEN_TWO}\n")
    );
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
    let mut profiles = IProfiles::default();
    let alpha = store
        .prepare_import(
            OpenP2PImportSourceKind::Local,
            &document("One", Some(TOKEN), true),
            None,
            &mut profiles,
        )
        .expect("alpha")
        .commit();
    let beta = store
        .prepare_import(
            OpenP2PImportSourceKind::Local,
            &document("Two", Some(TOKEN_TWO), true),
            None,
            &mut profiles,
        )
        .expect("beta")
        .commit();
    let account_dir = scratch.root.join("openp2p/accounts/home");
    std::fs::create_dir_all(&account_dir).expect("account dir");

    store.remove(&alpha.meta.id).expect("remove alpha");
    assert!(account_dir.exists(), "the shared workdir is used by beta");
    store.remove(&beta.meta.id).expect("remove beta");
    assert!(!account_dir.exists(), "the last owner removes the workdir");
}

#[test]
fn removing_a_subscription_removes_its_directory_and_scoped_credentials() {
    let scratch = Scratch::new("remove");
    let store = scratch.store();
    let mut profiles = IProfiles::default();
    let import = store
        .prepare_import(
            OpenP2PImportSourceKind::Local,
            &document("Home", Some(TOKEN), true),
            None,
            &mut profiles,
        )
        .expect("import")
        .commit();
    let account_dir = scratch.root.join("openp2p/accounts/home");
    std::fs::create_dir_all(&account_dir).expect("account dir");
    std::fs::write(account_dir.join("config.json"), "{}").expect("config.json");
    let token_path = store.token_path(&import.meta.id, "home").expect("token");
    let key_path = store.secrets(&import.meta.id).key_path(&key_digest(KEY)).expect("key");

    store.remove(&import.meta.id).expect("remove");
    assert!(!store.dir(&import.meta.id).exists());
    assert!(!token_path.exists());
    assert!(!key_path.exists());
    assert!(!account_dir.exists());
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
    let mut profiles = IProfiles::default();
    let alpha = store
        .prepare_import(
            OpenP2PImportSourceKind::Local,
            &document("One", Some(TOKEN), true),
            None,
            &mut profiles,
        )
        .expect("alpha")
        .commit();
    let beta = store
        .prepare_import(
            OpenP2PImportSourceKind::Local,
            &document("Two", Some(TOKEN_TWO), true),
            None,
            &mut profiles,
        )
        .expect("beta")
        .commit();
    let alpha_token = store.token_path(&alpha.meta.id, "home").expect("alpha token");
    let beta_token = store.token_path(&beta.meta.id, "home").expect("beta token");

    store.remove(&alpha.meta.id).expect("remove alpha");
    assert!(!alpha_token.exists());
    assert!(beta_token.exists());
    assert!(store.source_path(&beta.meta.id).exists());
    assert!(store.config_path(&beta.meta.id).exists());
    assert!(store.profile_path(&beta.profile_uid).exists());
    assert_eq!(store.list().expect("list").len(), 1);
}

#[test]
fn a_rolled_back_update_restores_files_and_credentials() {
    let scratch = Scratch::new("rollback");
    let store = scratch.store();
    let mut profiles = IProfiles::default();
    let first = store
        .prepare_import(
            OpenP2PImportSourceKind::Local,
            &document("Home", Some(TOKEN), true),
            None,
            &mut profiles,
        )
        .expect("first")
        .commit();
    let source_before = scratch.read(&store.source_path(&first.meta.id));
    let config_before = scratch.read(&store.config_path(&first.meta.id));
    let profile_before = scratch.read(&store.profile_path(&first.profile_uid));
    let manifest_before = scratch.read(&store.secrets(&first.meta.id).manifest_path());
    let token_path = store.token_path(&first.meta.id, "home").expect("token");
    let token_before = scratch.read(&token_path);
    let stale_key = store
        .secrets(&first.meta.id)
        .key_path(&key_digest(OTHER_KEY))
        .expect("stale key");

    let prepared = store
        .prepare_update(&document("Home", Some(TOKEN_TWO), false), &first.meta.id, &mut profiles)
        .expect("prepare update");
    assert!(prepared.outcome().profile_changed);
    assert_eq!(scratch.read(&token_path), format!("{TOKEN_TWO}\n"));
    assert!(!stale_key.exists());
    prepared.rollback().expect("rollback");

    assert_eq!(scratch.read(&store.source_path(&first.meta.id)), source_before);
    assert_eq!(scratch.read(&store.config_path(&first.meta.id)), config_before);
    assert_eq!(scratch.read(&store.profile_path(&first.profile_uid)), profile_before);
    assert_eq!(
        scratch.read(&store.secrets(&first.meta.id).manifest_path()),
        manifest_before
    );
    assert_eq!(scratch.read(&token_path), token_before);
    assert!(stale_key.is_file(), "rollback restores the removed key");
}

#[test]
fn a_rolled_back_new_import_removes_the_subscription_and_credentials() {
    let scratch = Scratch::new("rollback-new");
    let store = scratch.store();
    let mut profiles = IProfiles::default();
    let prepared = store
        .prepare_import(
            OpenP2PImportSourceKind::Local,
            &document("Home", Some(TOKEN), true),
            None,
            &mut profiles,
        )
        .expect("prepare");
    let id = prepared.outcome().meta.id.clone();
    let token_path = store.token_path(&id, "home").expect("token");
    prepared.rollback().expect("rollback");
    assert!(!store.source_path(&id).exists());
    assert!(!store.meta_path(&id).exists());
    assert!(!token_path.exists());
    assert!(store.list().expect("list").is_empty());
}

#[test]
fn a_failed_credential_write_leaves_no_subscription_files() {
    let scratch = Scratch::new("bad-secret-write");
    let store = scratch.store();
    let mut profiles = IProfiles::default();
    let text = document("Home", Some(TOKEN), true);
    let preview = store
        .preview_import(OpenP2PImportSourceKind::Local, &text, None)
        .expect("preview");
    let id = preview.meta.id.clone();
    // A directory where the token file must go makes the staging transaction fail.
    std::fs::create_dir_all(store.dir(&id).join("tokens/home")).expect("blocking token path");
    assert!(
        store.commit_preview(preview, &mut profiles).is_err(),
        "credential write must fail"
    );
    assert!(!store.source_path(&id).exists());
    assert!(!store.config_path(&id).exists());
    assert!(!store.meta_path(&id).exists());
    assert!(store.list().expect("list").is_empty());
}
