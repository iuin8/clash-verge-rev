//! Subscription-scoped credential files and their manifest.
//!
//! Tokens and SSH keys never leave the subscription directory that staged them:
//! `subscriptions/<id>/tokens/<account-id>` and
//! `subscriptions/<id>/keys/<sha256>.key`, both 0600. Deletion is a directory
//! removal, so no global reference counting exists.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::core::openp2p::account;
use crate::core::openp2p::schema::valid_slug;

/// The account token. No Debug/Display output ever exposes the value.
#[derive(Clone, Copy)]
pub struct TokenValue(u64);

impl TokenValue {
    pub const fn as_u64(self) -> u64 {
        self.0
    }

    #[cfg(test)]
    pub const fn from_u64_for_tests(value: u64) -> Self {
        Self(value)
    }
}

impl std::fmt::Debug for TokenValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TokenValue(***)")
    }
}

pub fn parse_token(raw: &str) -> Result<TokenValue> {
    let token: u64 = raw
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("openp2p token must be a positive decimal integer"))?;
    if token == 0 {
        bail!("openp2p token must be a positive decimal integer");
    }
    Ok(TokenValue(token))
}

pub fn validate_private_key(private_key: &str) -> Result<()> {
    if private_key.is_empty() || private_key.len() > 64 * 1024 {
        bail!("private key is empty or too large");
    }
    if private_key.contains('\0') {
        bail!("private key contains a NUL byte");
    }
    if !private_key.contains("-----BEGIN") || !private_key.contains("PRIVATE KEY-----") {
        bail!("private key must be PEM encoded");
    }
    Ok(())
}

pub fn key_digest(private_key: &str) -> std::string::String {
    Sha256::digest(private_key.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|ch| ch.is_ascii_hexdigit())
}

pub fn read_token(path: &Path) -> Result<TokenValue> {
    account::ensure_private_file(path)?;
    let raw = std::fs::read_to_string(path).with_context(|| "openp2p token file is missing".to_string())?;
    parse_token(&raw)
}

/// Two 0600 files per staged credential kind plus a manifest of what this
/// subscription currently owns.
#[derive(Debug, Clone)]
pub struct SecretStore {
    dir: PathBuf,
}

impl SecretStore {
    pub const fn at(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn tokens_dir(&self) -> PathBuf {
        self.dir.join("tokens")
    }

    fn keys_dir(&self) -> PathBuf {
        self.dir.join("keys")
    }

    pub fn token_path(&self, account_id: &str) -> Result<PathBuf> {
        if !valid_slug(account_id, 32) {
            bail!("invalid openp2p account id");
        }
        Ok(self.tokens_dir().join(account_id))
    }

    pub fn key_path(&self, digest: &str) -> Result<PathBuf> {
        if !valid_digest(digest) {
            bail!("invalid openp2p private key digest");
        }
        Ok(self.keys_dir().join(format!("{digest}.key")))
    }

    pub fn token_present(&self, account_id: &str) -> bool {
        self.token_path(account_id)
            .is_ok_and(|path| path.is_file() && !std::fs::read(&path).unwrap_or_default().is_empty())
    }

    pub fn read_token(&self, account_id: &str) -> Result<TokenValue> {
        read_token(&self.token_path(account_id)?)
    }

    pub fn manifest_path(&self) -> PathBuf {
        self.dir.join(MANIFEST_FILE)
    }

    pub fn load_manifest(&self) -> SecretManifest {
        let path = self.manifest_path();
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_yaml_ng::from_str(&raw).ok())
            .unwrap_or_default()
    }

    pub fn remove(&self, path: &Path) -> Result<()> {
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error).with_context(|| format!("remove {}", path.display())),
        }
    }
}

pub const MANIFEST_FILE: &str = "secrets.yaml";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SecretManifest {
    pub tokens: BTreeSet<std::string::String>,
    pub keys: BTreeSet<std::string::String>,
}
