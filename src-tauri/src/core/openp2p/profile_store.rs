//! Normalized OpenP2P source document store.
//!
//! A `ProfileStore` reads and writes exactly one normalized, secret-free
//! `OpenP2PConfig` document. The subscription store owns one file per
//! subscription (`openp2p/subscriptions/<id>/config.yaml`); the default layout
//! (`<app home>/openp2p/config.yaml`) is what the credential transaction tests
//! exercise.

use std::path::PathBuf;

use anyhow::{Context as _, Result, bail};

use crate::core::openp2p::account;
use crate::core::openp2p::schema::{self, OpenP2PConfig};
use crate::utils::dirs;

#[derive(Debug, Clone)]
pub struct ProfileStore {
    root: PathBuf,
    config: Option<PathBuf>,
}

impl ProfileStore {
    pub fn app_default() -> Result<Self> {
        Ok(Self {
            root: dirs::app_home_dir()?,
            config: None,
        })
    }

    #[cfg(test)]
    pub const fn at(root: PathBuf) -> Self {
        Self { root, config: None }
    }

    /// Points the store at a normalized config outside the default layout.
    pub const fn with_config(root: PathBuf, config: PathBuf) -> Self {
        Self {
            root,
            config: Some(config),
        }
    }

    pub fn openp2p_dir(&self) -> PathBuf {
        self.root.join("openp2p")
    }

    pub fn config_path(&self) -> PathBuf {
        self.config
            .clone()
            .unwrap_or_else(|| self.openp2p_dir().join("config.yaml"))
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
        account::atomic_write(&self.config_path(), yaml.as_bytes(), 0o600)
    }

    /// Profile items generated from a subscription stay read-only in the editor.
    pub fn is_managed_uid(&self, uid: &str) -> bool {
        crate::core::openp2p::subscriptions::is_subscription_profile_uid(uid)
    }
}
