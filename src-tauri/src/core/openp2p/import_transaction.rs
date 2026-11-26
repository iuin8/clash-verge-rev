//! Rollback units for one subscription import.
//!
//! `SecretTransaction` owns the subscription-scoped token/key files and their
//! manifest; `ImportUndo` owns the subscription documents and the generated
//! profile. Both restore their previous bytes when a later step fails.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};

use crate::core::openp2p::account;
use crate::core::openp2p::portable::ParsedImport;
use crate::core::openp2p::secrets::{SecretManifest, SecretStore};
use crate::core::openp2p::subscriptions::{SubscriptionImport, SubscriptionStore};
use clash_verge_logging::{Type, logging};

pub(crate) struct PreparedSubscriptionImport {
    import: SubscriptionImport,
    transaction: Option<SecretTransaction>,
    undo: Option<ImportUndo>,
}

impl PreparedSubscriptionImport {
    pub(crate) const fn outcome(&self) -> &SubscriptionImport {
        &self.import
    }

    /// Keeps the new files and credentials after the caller persisted the profile
    /// item list.
    pub fn commit(mut self) -> SubscriptionImport {
        self.undo.take();
        if let Some(transaction) = self.transaction.take() {
            transaction.finish();
        }
        self.import.clone()
    }

    /// Restores the previous subscription files and credentials after the profile
    /// item list could not be persisted.
    pub fn rollback(mut self) -> Result<()> {
        let transaction_error = match self.transaction.take() {
            Some(transaction) => transaction.rollback().err(),
            None => None,
        };
        let undo_error = match self.undo.take() {
            Some(undo) => undo.restore().err(),
            None => None,
        };
        match transaction_error.or(undo_error) {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

impl Drop for PreparedSubscriptionImport {
    fn drop(&mut self) {
        let transaction_error = match self.transaction.take() {
            Some(transaction) => transaction.rollback().err(),
            None => None,
        };
        if let Some(error) = transaction_error {
            logging!(
                warn,
                Type::Network,
                "failed to roll back unfinished openp2p credentials: {error:#}"
            );
        }
        let Some(undo) = self.undo.take() else {
            return;
        };
        if let Err(error) = undo.restore() {
            logging!(
                warn,
                Type::Network,
                "failed to roll back an unfinished openp2p subscription import: {error:#}"
            );
        }
    }
}

#[derive(Debug)]
pub(crate) struct ImportUndo {
    created: bool,
    sub_dir: PathBuf,
    source: (PathBuf, Option<Vec<u8>>),
    meta: (PathBuf, Option<Vec<u8>>),
    config: (PathBuf, Option<Vec<u8>>),
    profile: (PathBuf, Option<Vec<u8>>),
}

impl ImportUndo {
    pub(crate) fn snapshot(store: &SubscriptionStore, id: &str, created: bool, profile_path: &Path) -> Result<Self> {
        let source_path = store.source_path(id);
        let meta_path = store.meta_path(id);
        let config_path = store.config_path(id);
        Ok(Self {
            created,
            sub_dir: store.dir(id),
            source: (source_path.clone(), read_optional(&source_path)?),
            meta: (meta_path.clone(), read_optional(&meta_path)?),
            config: (config_path.clone(), read_optional(&config_path)?),
            profile: (profile_path.to_path_buf(), read_optional(profile_path)?),
        })
    }

    pub(crate) fn profile_changed(&self, yaml: &str) -> bool {
        self.profile.1.as_deref() != Some(yaml.as_bytes())
    }

    pub(crate) fn restore(self) -> Result<()> {
        let mut first_error = None;
        for (path, previous) in [&self.source, &self.meta, &self.config, &self.profile] {
            let result = match previous {
                Some(bytes) => account::atomic_write(path, bytes, 0o600),
                None => match std::fs::remove_file(path) {
                    Ok(()) => Ok(()),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Err(error) => Err(error.into()),
                },
            };
            if let Err(error) = result
                && first_error.is_none()
            {
                first_error = Some(error.context(format!("restore {}", path.display())));
            }
        }
        if self.created
            && let Err(error) = std::fs::remove_dir_all(&self.sub_dir)
            && error.kind() != std::io::ErrorKind::NotFound
            && first_error.is_none()
        {
            first_error = Some(anyhow::Error::new(error).context(format!("remove {}", self.sub_dir.display())));
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

/// Writes the staged credentials and their manifest, and removes files that the
/// previous manifest owned and this import no longer references.
pub(crate) struct SecretTransaction {
    secrets: SecretStore,
    backups: Vec<(PathBuf, Option<Vec<u8>>)>,
    finished: bool,
}

impl SecretTransaction {
    pub(crate) fn begin(secrets: &SecretStore, parsed: &ParsedImport, previous: &SecretManifest) -> Result<Self> {
        let mut transaction = Self {
            secrets: secrets.clone(),
            backups: Vec::new(),
            finished: false,
        };
        let result = transaction.stage(parsed, previous);
        if let Err(error) = result {
            return Err(match transaction.rollback() {
                Ok(()) => error,
                Err(rollback_error) => error.context(format!("rollback failed: {rollback_error:#}")),
            });
        }
        Ok(transaction)
    }

    fn stage(&mut self, parsed: &ParsedImport, previous: &SecretManifest) -> Result<()> {
        let account_ids: BTreeSet<&str> = parsed
            .config
            .accounts
            .iter()
            .map(|account| account.id.as_str())
            .collect();
        let mut tokens: BTreeSet<std::string::String> = previous
            .tokens
            .iter()
            .filter(|id| account_ids.contains(id.as_str()))
            .cloned()
            .collect();
        for (account_id, token) in parsed.tokens() {
            let path = self.secrets.token_path(account_id)?;
            self.write(&path, format!("{token}\n").as_bytes())?;
            tokens.insert(account_id.to_owned());
        }
        let mut keys = BTreeSet::new();
        for (digest, private_key) in parsed.keys() {
            let path = self.secrets.key_path(digest)?;
            self.write(&path, private_key.as_bytes())?;
            keys.insert(digest.to_owned());
        }
        let manifest = SecretManifest {
            tokens: tokens.clone(),
            keys: keys.clone(),
        };
        self.write(
            &self.secrets.manifest_path(),
            serde_yaml_ng::to_string(&manifest)?.as_bytes(),
        )?;
        for account_id in previous.tokens.difference(&tokens) {
            let path = self.secrets.token_path(account_id)?;
            self.remove(&path)?;
        }
        for digest in previous.keys.difference(&keys) {
            let path = self.secrets.key_path(digest)?;
            self.remove(&path)?;
        }
        Ok(())
    }

    fn write(&mut self, path: &Path, bytes: &[u8]) -> Result<()> {
        self.backup(path)?;
        account::atomic_write(path, bytes, 0o600)
    }

    fn remove(&mut self, path: &Path) -> Result<()> {
        self.backup(path)?;
        self.secrets.remove(path)
    }

    fn backup(&mut self, path: &Path) -> Result<()> {
        if self.backups.iter().any(|(existing, _)| existing == path) {
            return Ok(());
        }
        let previous = read_optional(path)?;
        self.backups.push((path.to_path_buf(), previous));
        Ok(())
    }

    pub(crate) fn finish(mut self) {
        self.finished = true;
    }

    pub(crate) fn rollback(mut self) -> Result<()> {
        self.finished = true;
        self.rollback_inner()
    }

    fn rollback_inner(&self) -> Result<()> {
        let mut first_error = None;
        for (path, previous) in self.backups.iter().rev() {
            let result = match previous {
                Some(bytes) => account::atomic_write(path, bytes, 0o600),
                None => match std::fs::remove_file(path) {
                    Ok(()) => Ok(()),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Err(error) => Err(error.into()),
                },
            };
            if let Err(error) = result
                && first_error.is_none()
            {
                first_error = Some(error.context(format!("restore {}", path.display())));
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

impl Drop for SecretTransaction {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        if let Err(error) = self.rollback_inner() {
            logging!(
                warn,
                clash_verge_logging::Type::Config,
                "openp2p credential rollback failed: {error:#}"
            );
        }
    }
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("read {}", path.display())),
    }
}

impl PreparedSubscriptionImport {
    pub(crate) const fn new(import: SubscriptionImport, transaction: SecretTransaction, undo: ImportUndo) -> Self {
        Self {
            import,
            transaction: Some(transaction),
            undo: Some(undo),
        }
    }
}
