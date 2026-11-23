//! App-level OpenP2P source and per-account profile projection (M2, not wired yet).
//!
//! The store is the single source of truth; profile files are generated views
//! that CVR can list, activate and bundle. Tokens and private keys never enter
//! either file.
#![allow(dead_code)] // wired by profile sync in M2/M3

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::core::openp2p::account::{self, token_present};
use crate::core::openp2p::artifacts;
use crate::core::openp2p::schema::{self, OpenP2PConfig};
use crate::utils::dirs;

const MANIFEST_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ManagedEntry {
    account_id: std::string::String,
    uid: std::string::String,
    file: std::string::String,
    sha256: std::string::String,
    generated_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Manifest {
    version: u32,
    profiles: BTreeMap<std::string::String, ManagedEntry>,
}

impl Default for Manifest {
    fn default() -> Self {
        Self {
            version: MANIFEST_VERSION,
            profiles: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncReport {
    pub written: Vec<std::string::String>,
    pub removed: Vec<std::string::String>,
    pub edited: Vec<std::string::String>,
}

#[derive(Debug, Clone)]
pub struct ProfileStore {
    root: PathBuf,
}

impl ProfileStore {
    pub fn app_default() -> Result<Self> {
        Ok(Self {
            root: dirs::app_home_dir()?,
        })
    }

    #[cfg(test)]
    pub const fn at(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn openp2p_dir(&self) -> PathBuf {
        self.root.join("openp2p")
    }

    pub fn config_path(&self) -> PathBuf {
        self.openp2p_dir().join("config.yaml")
    }

    pub fn manifest_path(&self) -> PathBuf {
        self.openp2p_dir().join("profiles.yaml")
    }

    pub fn profile_dir(&self) -> PathBuf {
        self.root.join("profiles")
    }

    pub fn profile_path(&self, uid: &str) -> PathBuf {
        self.profile_dir().join(format!("{uid}.yaml"))
    }

    pub fn load_config(&self) -> Result<Option<OpenP2PConfig>> {
        let path = self.config_path();
        let raw = match std::fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
        };
        let parsed = schema::parse_seed(&raw);
        if schema::has_errors(&parsed.issues) {
            bail!("openp2p config is invalid: {:?}", parsed.issues);
        }
        Ok(Some(parsed.config))
    }

    pub fn save_config(&self, config: &OpenP2PConfig) -> Result<()> {
        let yaml = serde_yaml_ng::to_string(config)?;
        account::create_private_dir(&self.openp2p_dir())?;
        write_private(&self.config_path(), yaml.as_bytes())
    }

    pub fn sync(&self, config: &OpenP2PConfig) -> Result<SyncReport> {
        self.sync_with(config, token_present, |reference| {
            account::key_path(reference).ok().is_some_and(|path| path.is_file())
        })
    }

    pub fn sync_with<T, K>(&self, config: &OpenP2PConfig, token_ok: T, key_ok: K) -> Result<SyncReport>
    where
        T: Fn(&str) -> bool,
        K: Fn(&str) -> bool,
    {
        std::fs::create_dir_all(self.profile_dir())
            .with_context(|| format!("create {}", self.profile_dir().display()))?;
        account::create_private_dir(&self.openp2p_dir())?;
        let mut manifest = self.load_manifest().unwrap_or_default();
        let mut report = SyncReport::default();
        let mut expected_accounts = BTreeMap::new();

        for account in &config.accounts {
            let uid = format!("op2p-{}", account.id);
            let file = format!("{uid}.yaml");
            let path = self.profile_path(&uid);
            let Some(yaml) = artifacts::render_account_profile_with(config, &account.id, &token_ok, &key_ok)? else {
                continue;
            };
            let expected_sha = sha256(yaml.as_bytes());
            if let Some(previous) = manifest.profiles.get(&uid)
                && path.exists()
            {
                let current = sha256(&std::fs::read(&path)?);
                if current != previous.sha256 && current != expected_sha {
                    report.edited.push(uid.clone());
                    expected_accounts.insert(uid, previous.clone());
                    continue;
                }
            }
            let previous_sha = manifest.profiles.get(&uid).map(|entry| entry.sha256.as_str());
            if previous_sha != Some(expected_sha.as_str()) || !path.is_file() {
                write_private(&path, yaml.as_bytes())?;
                report.written.push(uid.clone());
            }
            expected_accounts.insert(
                uid.clone(),
                ManagedEntry {
                    account_id: account.id.clone(),
                    uid,
                    file,
                    sha256: expected_sha,
                    generated_at: now_secs(),
                },
            );
        }

        for (uid, entry) in &manifest.profiles {
            if expected_accounts.contains_key(uid) {
                continue;
            }
            let path = self.profile_path(uid);
            if path.exists() {
                let current = sha256(&std::fs::read(&path)?);
                if current != entry.sha256 {
                    report.edited.push(uid.clone());
                    expected_accounts.insert(uid.clone(), entry.clone());
                    continue;
                }
                std::fs::remove_file(&path).with_context(|| format!("remove {}", path.display()))?;
            }
            report.removed.push(uid.clone());
        }
        manifest.profiles = expected_accounts;
        self.save_manifest(&manifest)?;
        Ok(report)
    }

    fn load_manifest(&self) -> Result<Manifest> {
        let path = self.manifest_path();
        let raw = match std::fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Manifest::default()),
            Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
        };
        serde_yaml_ng::from_str(&raw).with_context(|| format!("parse {}", path.display()))
    }

    fn save_manifest(&self, manifest: &Manifest) -> Result<()> {
        let yaml = serde_yaml_ng::to_string(manifest)?;
        write_private(&self.manifest_path(), yaml.as_bytes())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectReport {
    pub files: SyncReport,
    pub items_changed: bool,
}

impl ProfileStore {
    pub fn managed_uids(&self) -> Result<Vec<std::string::String>> {
        Ok(self.load_manifest()?.profiles.keys().cloned().collect())
    }

    pub fn managed_profiles(&self, config: &OpenP2PConfig) -> Vec<crate::core::openp2p::profile_items::ManagedProfile> {
        let manifest = self.load_manifest().unwrap_or_default();
        let mut managed = Vec::new();
        for account in &config.accounts {
            let uid = format!("op2p-{}", account.id);
            let generated_at = manifest
                .profiles
                .get(&uid)
                .map(|entry| entry.generated_at)
                .unwrap_or_else(now_secs);
            managed.push(crate::core::openp2p::profile_items::ManagedProfile {
                account_id: account.id.clone(),
                uid,
                name: format!("OpenP2P · {}", account.label()),
                file: format!("op2p-{}.yaml", account.id),
                updated: generated_at,
            });
        }
        managed
    }

    /// Saves the source, projects profile files and updates the managed
    /// profiles.yaml items. The caller owns the profiles.yaml transaction.
    pub fn project(&self, config: &OpenP2PConfig, profiles: &mut crate::config::IProfiles) -> Result<ProjectReport> {
        let previous = self.managed_uids()?;
        self.save_config(config)?;
        let files = self.sync(config)?;
        let managed = self.managed_profiles(config);
        let items_changed = crate::core::openp2p::profile_items::sync_managed_items(profiles, &managed, &previous)?;
        Ok(ProjectReport { files, items_changed })
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

fn sha256(bytes: &[u8]) -> std::string::String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().iter().map(|byte| format!("{byte:02x}")).collect()
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("refusing to write a path without a parent"))?;
    std::fs::create_dir_all(parent)?;
    let tmp = parent.join(format!(
        ".{}.tmp-{}",
        path.file_name().and_then(|name| name.to_str()).unwrap_or("write"),
        std::process::id()
    ));
    std::fs::write(&tmp, bytes).with_context(|| format!("write {}", tmp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&tmp, path).with_context(|| format!("rename {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::ProfileStore;
    use crate::core::openp2p::schema::fixtures::sample_config;

    fn root() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "cvr-openp2p-store-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn projects_source_files_and_managed_items_idempotently() {
        let root = root();
        let store = ProfileStore::at(root.clone());
        let config = sample_config();
        let mut profiles = crate::config::IProfiles::default();
        let first = store.project(&config, &mut profiles).expect("project");
        assert_eq!(first.files.written, vec!["op2p-home"]);
        assert!(first.items_changed);
        assert!(profiles.current.is_none());
        let items = profiles.items.as_ref().expect("items");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].uid.as_deref(), Some("op2p-home"));
        assert_eq!(items[0].itype.as_deref(), Some("local"));
        assert!(items[0].option.is_none());
        assert!(store.profile_path("op2p-home").is_file());
        let second = store.project(&config, &mut profiles).expect("project again");
        assert_eq!(second.files.written, Vec::<String>::new());
        assert!(!second.items_changed);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn saves_and_loads_the_non_secret_source() {
        let root = root();
        let store = ProfileStore::at(root.clone());
        let config = sample_config();
        store.save_config(&config).expect("save");
        let loaded = store.load_config().expect("load").expect("config");
        assert_eq!(loaded, config);
        let text = std::fs::read_to_string(store.config_path()).expect("read");
        assert!(!text.contains("PRIVATE KEY"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                std::fs::metadata(store.config_path()).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn syncs_stable_profiles_and_honors_user_edits() {
        let root = root();
        let store = ProfileStore::at(root.clone());
        let config = sample_config();
        let first = store.sync_with(&config, |_| true, |_| true).expect("sync");
        assert_eq!(first.written, vec!["op2p-home"]);
        let path = store.profile_path("op2p-home");
        let text = std::fs::read_to_string(&path).expect("profile");
        assert!(text.contains("OP2P-home"));
        assert!(!text.contains("PRIVATE KEY"));
        assert!(!text.contains("MATCH"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let again = store.sync_with(&config, |_| true, |_| true).expect("sync again");
        assert!(again.written.is_empty());
        std::fs::write(&path, "user edit").expect("edit");
        let edited = store.sync_with(&config, |_| true, |_| true).expect("sync edited");
        assert_eq!(edited.edited, vec!["op2p-home"]);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "user edit");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn disabled_or_missing_secret_profiles_have_no_nodes_and_removed_accounts_are_pruned() {
        let root = root();
        let store = ProfileStore::at(root.clone());
        let config = sample_config();
        store.sync_with(&config, |_| false, |_| false).expect("sync");
        let text = std::fs::read_to_string(store.profile_path("op2p-home")).expect("profile");
        assert!(!text.contains("op2p-home"));
        assert!(text.contains("proxies: []"));
        let mut empty = config;
        empty.accounts.clear();
        empty.proxies.clear();
        empty.networks.clear();
        let report = store.sync_with(&empty, |_| true, |_| true).expect("prune");
        assert!(report.removed.contains(&"op2p-home".to_string()));
        assert!(!store.profile_path("op2p-home").exists());
        let _ = std::fs::remove_dir_all(root);
    }
}
