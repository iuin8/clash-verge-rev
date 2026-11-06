//! Isolated acceptance for the materialized Profile Bundle.
//!
//! Each test redirects the process to a temporary home (serialized by a process
//! wide lock) so it never reads or writes real application data.
//!
//! Run with `cargo test --test bundle_acceptance`.

#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use app_lib::{
    config::{IProfiles, PrfItem, bundle},
    utils::dirs,
};
use clash_verge_draft::Draft;
use serde_yaml_ng::Mapping;
use std::{
    env,
    ffi::OsString,
    fs,
    path::PathBuf,
    sync::{Mutex, MutexGuard},
    time::{SystemTime, UNIX_EPOCH},
};

static HOME_LOCK: Mutex<()> = Mutex::new(());

struct TempHome {
    root: PathBuf,
    previous_home: Option<OsString>,
    previous_data_home: Option<OsString>,
    _guard: MutexGuard<'static, ()>,
}

impl TempHome {
    fn new() -> Self {
        let guard = HOME_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let root = env::temp_dir().join(format!(
            "cvr-bundle-acceptance-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let previous_home = env::var_os("HOME");
        let previous_data_home = env::var_os("XDG_DATA_HOME");
        // SAFETY: the process-wide HOME_LOCK serializes every test in this binary.
        unsafe {
            env::set_var("HOME", &root);
            env::set_var("XDG_DATA_HOME", root.join("data"));
        }
        Self {
            root,
            previous_home,
            previous_data_home,
            _guard: guard,
        }
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        // SAFETY: same serialization as `new`.
        unsafe {
            match self.previous_home.take() {
                Some(value) => env::set_var("HOME", value),
                None => env::remove_var("HOME"),
            }
            match self.previous_data_home.take() {
                Some(value) => env::set_var("XDG_DATA_HOME", value),
                None => env::remove_var("XDG_DATA_HOME"),
            }
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn uids(list: &[&str]) -> Vec<smartstring::alias::String> {
    list.iter().map(|uid| (*uid).into()).collect()
}

fn item(uid: &str, itype: &str) -> PrfItem {
    PrfItem {
        uid: Some(uid.into()),
        itype: Some(itype.into()),
        file: Some(format!("{uid}.yaml").into()),
        ..PrfItem::default()
    }
}

fn profile(proxies: &[(&str, &str)], rules: &[&str]) -> String {
    let mut yaml = String::from("proxies:\n");
    for (name, server) in proxies {
        yaml.push_str(&format!(
            "  - name: {name}\n    type: ss\n    server: {server}\n    port: 443\n    cipher: aes-128-gcm\n    password: secret\n"
        ));
    }
    yaml.push_str("rules:\n");
    for rule in rules {
        yaml.push_str(&format!("  - {rule}\n"));
    }
    yaml
}

fn write_member(name: &str, content: &str) {
    let profiles_dir = dirs::app_profiles_dir().unwrap();
    fs::create_dir_all(&profiles_dir).unwrap();
    fs::write(profiles_dir.join(format!("{name}.yaml")), content).unwrap();
}

fn read_generated() -> String {
    fs::read_to_string(bundle::generated_path().unwrap()).unwrap()
}

fn profiles(items: Vec<PrfItem>, current: &str) -> IProfiles {
    IProfiles {
        current: Some(current.into()),
        items: Some(items),
    }
}

fn setup_two_members() -> IProfiles {
    write_member("alpha", &profile(&[("alpha", "a.example")], &["MATCH,DIRECT"]));
    write_member(
        "beta",
        &profile(&[("beta", "b.example")], &["DOMAIN,beta.example,DIRECT"]),
    );
    profiles(vec![item("alpha", "remote"), item("beta", "local")], "alpha")
}

#[tokio::test]
async fn generated_profile_is_plain_and_refreshes_when_a_member_changes() {
    let _home = TempHome::new();
    let mut profiles = setup_two_members();
    bundle::upsert_item(&mut profiles);

    let generated = bundle::generate(&profiles, &uids(&["alpha", "beta"]))
        .await
        .expect("members are valid");
    assert_eq!(generated.meta.members, uids(&["alpha", "beta"]));
    assert!(generated.yaml.contains("alpha"));
    assert!(generated.yaml.contains("beta"));

    bundle::write(&generated).await.unwrap();
    let parsed: Mapping = serde_yaml_ng::from_str(&read_generated()).unwrap();
    assert!(
        parsed.get("proxies").is_some(),
        "the generated file must be a plain Clash profile"
    );
    assert!(
        parsed.get("members").is_none(),
        "bundle metadata must not leak into the profile"
    );

    let meta = bundle::load_meta().await.unwrap().expect("metadata written");
    assert!(!bundle::is_stale(&profiles, &meta).await.unwrap());

    // The generated file is bundle-owned: a manual edit is detected and restored.
    fs::write(bundle::generated_path().unwrap(), "# tampered\n").unwrap();
    assert!(
        bundle::is_stale(&profiles, &meta).await.unwrap(),
        "a manually edited generated file must be detected as stale"
    );
    let restored = bundle::refresh(&profiles, false).await.unwrap().unwrap();
    assert!(restored.regenerated);
    assert!(
        read_generated().contains("alpha"),
        "refresh must restore the generated profile"
    );

    // A member change must mark the bundle stale; refreshing must regenerate it.
    write_member(
        "beta",
        &profile(&[("beta-renamed", "b.example")], &["DOMAIN,beta.example,DIRECT"]),
    );
    assert!(
        bundle::is_stale(&profiles, &meta).await.unwrap(),
        "a changed member source must be detected as stale"
    );
    let outcome = bundle::refresh(&profiles, false).await.unwrap().expect("bundle exists");
    assert!(outcome.regenerated);
    let regenerated = read_generated();
    assert!(regenerated.contains("beta-renamed"));
    assert!(!regenerated.contains("name: beta\n"));

    let refreshed_meta = bundle::load_meta().await.unwrap().unwrap();
    assert!(!bundle::is_stale(&profiles, &refreshed_meta).await.unwrap());
    let no_op = bundle::refresh(&profiles, false).await.unwrap().unwrap();
    assert!(!no_op.regenerated, "a fresh bundle must not be rewritten");
}

#[tokio::test]
async fn deleting_a_member_prunes_metadata_and_regenerates() {
    let _home = TempHome::new();
    write_member("alpha", &profile(&[("alpha", "a.example")], &["MATCH,DIRECT"]));
    write_member(
        "beta",
        &profile(&[("beta", "b.example")], &["DOMAIN,beta.example,DIRECT"]),
    );
    write_member("gamma", &profile(&[("gamma", "c.example")], &["MATCH,DIRECT"]));
    let mut profiles = profiles(
        vec![item("alpha", "remote"), item("beta", "local"), item("gamma", "remote")],
        "alpha",
    );
    bundle::upsert_item(&mut profiles);

    let generated = bundle::generate(&profiles, &uids(&["alpha", "beta", "gamma"]))
        .await
        .unwrap();
    bundle::write(&generated).await.unwrap();

    let meta = bundle::load_meta().await.unwrap().unwrap();
    let bundle::PrunePlan::Regenerate(remaining) = bundle::plan_prune(&meta, "beta") else {
        panic!("three members must regenerate, not tear down");
    };
    assert_eq!(remaining, uids(&["alpha", "gamma"]));

    bundle::regenerate(&profiles, &remaining).await.unwrap();
    let regenerated = read_generated();
    assert!(regenerated.contains("alpha"));
    assert!(
        !regenerated.contains("name: beta\n"),
        "deleted member must not survive regeneration"
    );
    let meta = bundle::load_meta().await.unwrap().unwrap();
    assert_eq!(meta.members, uids(&["alpha", "gamma"]));

    let bundle::PrunePlan::RemoveBundle = bundle::plan_prune(&meta, "alpha") else {
        panic!("dropping below two members must tear the bundle down");
    };
}

#[tokio::test]
async fn legacy_merged_json_migrates_once_and_is_archived() {
    let _home = TempHome::new();
    let profiles = setup_two_members();
    let app_home = dirs::app_home_dir().unwrap();
    fs::create_dir_all(&app_home).unwrap();
    let legacy = app_home.join("merged.json");
    fs::write(&legacy, r#"{"uids":["alpha","beta"]}"#).unwrap();

    let draft = Draft::new(profiles);
    let outcome = bundle::migrate_legacy(&draft).await.unwrap();
    assert!(outcome.migrated);
    assert!(!legacy.exists(), "the legacy store must be moved");
    assert!(legacy.with_file_name("merged.json.migrated").is_file());

    let snapshot = draft.latest_arc();
    let bundle_item = snapshot.get_item(bundle::BUNDLE_UID).expect("bundle item created");
    assert_eq!(bundle_item.itype.as_deref(), Some("local"));
    assert_eq!(bundle_item.file.as_deref(), Some(bundle::BUNDLE_FILE));
    assert_eq!(snapshot.current.as_deref(), Some(bundle::BUNDLE_UID));
    assert!(bundle::generated_path().unwrap().is_file());
    assert!(bundle::meta_path().unwrap().is_file());

    // Idempotent: a second run must not duplicate or re-create anything.
    let outcome = bundle::migrate_legacy(&draft).await.unwrap();
    assert!(!outcome.migrated);
    assert_eq!(
        draft
            .latest_arc()
            .items
            .as_ref()
            .unwrap()
            .iter()
            .filter(|item| item.uid.as_deref() == Some(bundle::BUNDLE_UID))
            .count(),
        1
    );
}

#[tokio::test]
async fn corrupt_legacy_state_is_archived_never_deleted() {
    let _home = TempHome::new();
    let profiles = setup_two_members();
    let app_home = dirs::app_home_dir().unwrap();
    fs::create_dir_all(&app_home).unwrap();
    let legacy = app_home.join("merged.json");
    fs::write(&legacy, "{ definitely not json").unwrap();

    let draft = Draft::new(profiles);
    let outcome = bundle::migrate_legacy(&draft).await.unwrap();
    assert!(!outcome.migrated);
    assert!(draft.latest_arc().get_item(bundle::BUNDLE_UID).is_err());

    let archived = outcome.archived.expect("corrupt state must be archived");
    assert!(archived.is_file());
    assert!(archived.file_name().unwrap().to_string_lossy().contains("corrupt-"));
    assert_eq!(fs::read_to_string(&archived).unwrap(), "{ definitely not json");
}
