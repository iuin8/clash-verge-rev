//! Isolated acceptance for the on-disk `merged.json` store.
//!
//! The test redirects the process to a temporary home, so it never reads or
//! writes the real application data directory.
//!
//! Run with `cargo test --test merge_store_acceptance`.

#![cfg(unix)]
#![allow(clippy::unwrap_used)]

use app_lib::{config::merged, utils::dirs};
use smartstring::alias::String;
use std::{
    env,
    ffi::OsString,
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

fn uids(list: &[&str]) -> Vec<String> {
    list.iter().map(|uid| (*uid).into()).collect()
}

struct TempHome {
    root: PathBuf,
    previous_home: Option<OsString>,
    previous_data_home: Option<OsString>,
}

impl TempHome {
    fn new() -> Self {
        let root = env::temp_dir().join(format!(
            "cvr-merge-acceptance-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let previous_home = env::var_os("HOME");
        let previous_data_home = env::var_os("XDG_DATA_HOME");
        // SAFETY: this integration-test binary runs a single test and redirects
        // the whole process to the temporary home before touching app data.
        unsafe {
            env::set_var("HOME", &root);
            env::set_var("XDG_DATA_HOME", root.join("data"));
        }
        Self {
            root,
            previous_home,
            previous_data_home,
        }
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        // SAFETY: same single-test process as `new`.
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

#[tokio::test]
async fn merged_store_is_isolated_and_recovers_from_corruption() {
    let home = TempHome::new();
    let app_home = dirs::app_home_dir().unwrap();
    assert!(
        app_home.starts_with(&home.root),
        "app data must stay under the temporary home, got {}",
        app_home.display()
    );
    fs::create_dir_all(&app_home).unwrap();
    let store = app_home.join("merged.json");

    // Apply persists the ordered selection.
    merged::save(&uids(&["a", "b"])).await.unwrap();
    assert_eq!(merged::load().await.unwrap(), uids(&["a", "b"]));
    assert!(store.is_file());

    // Deleting a member prunes it; deleting an outsider is idempotent.
    merged::remove("a").await.unwrap();
    assert_eq!(merged::load().await.unwrap(), uids(&["b"]));
    merged::remove("missing").await.unwrap();
    assert_eq!(merged::load().await.unwrap(), uids(&["b"]));

    // Clear drops the selection.
    merged::clear().await.unwrap();
    assert!(merged::load().await.unwrap().is_empty());

    // A corrupt store fails closed; only the explicit clear recovers it.
    fs::write(&store, "{ invalid json").unwrap();
    assert!(merged::load().await.is_err());
    assert!(merged::remove("b").await.is_err());
    merged::clear().await.unwrap();
    assert!(merged::load().await.unwrap().is_empty());

    let temporary_files: Vec<_> = fs::read_dir(&app_home)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(".tmp-"))
        .collect();
    assert!(
        temporary_files.is_empty(),
        "atomic saves left temporary files: {temporary_files:?}"
    );
}
