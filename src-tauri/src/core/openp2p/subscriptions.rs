//! OpenP2P subscription store: one imported YAML is one subscription/profile.
//!
//! Layout below openp2p/:
//!
//! - subscriptions/<id>/source.yaml — the imported document exactly as supplied
//!   (0600). It may carry inline credentials; only the content command reads it.
//! - subscriptions/<id>/config.yaml — the normalized, secret-free
//!   OpenP2PConfig consumed by the sidecar manager and the renderer. Keeping it
//!   also keeps generated node names and ports stable across restarts.
//! - subscriptions/<id>/meta.yaml — id, name, source kind, source URL, import
//!   time and account count.
//!
//! profiles.yaml:current is the single active authority: the sidecar runs a
//! subscription only while the current profile is its op2p-sub-<id> card. There
//! is no separate active pointer to keep in sync.
//!
//! Stable ids: a remote subscription is identified by its canonical URL
//! (sha256 over "url\n<url>"), a local one by its derived name plus its content
//! (sha256 over "local\n<name>\n<content>"); the first 16 hex digits are used.
//! Re-importing the same remote URL therefore updates one subscription, while an
//! edited local document becomes a new subscription. Re-import keeps the
//! previous device fields because the normalized config doubles as the merge
//! source for parse_import.
//!
//! Account ids and credential references may repeat across subscriptions; only
//! the active source runs its sidecars, so a shared account workdir is reused
//! serially and shared vault refs are reference-counted during removal.
//!
//! The generated profile is profiles/op2p-sub-<id>.yaml and its PrfItem carries
//! desc: openp2p-subscription:<id>. Import never changes profiles.yaml:current.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::config::{IProfiles, PrfItem};
use crate::core::openp2p::account;
use crate::core::openp2p::portable::{self, ImportTransaction, OpenP2PImportSourceKind, ParsedImport, SecretVault};
use crate::core::openp2p::profile_render::{self, RenderedProfile};
use crate::core::openp2p::profile_store::ProfileStore;
use crate::core::openp2p::schema::{self, OpenP2PConfig, OpenP2PIssue};
use crate::core::openp2p::state::OpenP2PSource;
use crate::utils::dirs;
use clash_verge_logging::{Type, logging};

pub const PROFILE_UID_PREFIX: &str = "op2p-sub-";
pub const PROFILE_DESC_PREFIX: &str = "openp2p-subscription:";

const SOURCE_FILE: &str = "source.yaml";
const CONFIG_FILE: &str = "config.yaml";
const META_FILE: &str = "meta.yaml";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubscriptionMeta {
    pub id: std::string::String,
    pub name: std::string::String,
    pub source: OpenP2PImportSourceKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<std::string::String>,
    pub imported_at: u64,
    #[serde(default)]
    pub account_count: usize,
}

impl SubscriptionMeta {
    pub fn profile_uid(&self) -> std::string::String {
        profile_uid(&self.id)
    }
}

/// Everything the manager needs to run the subscription named by one profile.
#[derive(Debug, Clone)]
pub struct SubscriptionRuntime {
    pub meta: SubscriptionMeta,
    pub profile_uid: std::string::String,
    pub config: OpenP2PConfig,
    pub source: OpenP2PSource,
}

#[derive(Debug, Clone)]
pub struct SubscriptionImport {
    pub meta: SubscriptionMeta,
    pub profile_uid: std::string::String,
    pub created: bool,
    pub profile_changed: bool,
    pub items_changed: bool,
    pub warnings: Vec<OpenP2PIssue>,
}

#[derive(Debug, Clone)]
pub struct SubscriptionStore {
    root: PathBuf,
}

impl SubscriptionStore {
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

    pub fn subscriptions_dir(&self) -> PathBuf {
        self.openp2p_dir().join("subscriptions")
    }

    pub fn profiles_dir(&self) -> PathBuf {
        self.root.join("profiles")
    }

    fn dir(&self, id: &str) -> PathBuf {
        self.subscriptions_dir().join(id)
    }

    pub fn dir_exists(&self, id: &str) -> bool {
        schema::valid_slug(id, 32) && self.dir(id).is_dir()
    }

    pub fn source_path(&self, id: &str) -> PathBuf {
        self.dir(id).join(SOURCE_FILE)
    }

    pub fn config_path(&self, id: &str) -> PathBuf {
        self.dir(id).join(CONFIG_FILE)
    }

    pub fn meta_path(&self, id: &str) -> PathBuf {
        self.dir(id).join(META_FILE)
    }

    pub fn profile_path(&self, profile_uid: &str) -> PathBuf {
        self.profiles_dir().join(format!("{profile_uid}.yaml"))
    }

    pub fn list(&self) -> Result<Vec<SubscriptionMeta>> {
        let dir = self.subscriptions_dir();
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error).with_context(|| format!("read {}", dir.display())),
        };
        let mut metas = Vec::new();
        for entry in entries.flatten() {
            if !entry.path().is_dir() {
                continue;
            }
            let id = entry.file_name().to_string_lossy().into_owned();
            if !valid_id(&id) {
                continue;
            }
            if let Ok(meta) = self.load_meta(&id) {
                metas.push(meta);
            }
        }
        metas.sort_by(|left, right| {
            left.imported_at
                .cmp(&right.imported_at)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(metas)
    }

    pub fn load_meta(&self, id: &str) -> Result<SubscriptionMeta> {
        validate_id(id)?;
        let path = self.meta_path(id);
        let raw = match std::fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                bail!("openp2p subscription {id} is missing")
            }
            Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
        };
        let meta: SubscriptionMeta =
            serde_yaml_ng::from_str(&raw).with_context(|| format!("parse {}", path.display()))?;
        if meta.id != id {
            bail!("openp2p subscription {id} has mismatched metadata");
        }
        Ok(meta)
    }

    pub fn load_config(&self, id: &str) -> Result<OpenP2PConfig> {
        validate_id(id)?;
        ProfileStore::with_config(self.root.clone(), self.config_path(id))
            .load_config()?
            .ok_or_else(|| anyhow::anyhow!("openp2p subscription {id} has no normalized config"))
    }

    pub fn read_source(&self, id: &str) -> Result<std::string::String> {
        validate_id(id)?;
        let path = self.source_path(id);
        std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))
    }

    pub fn find_by_profile_uid(&self, profile_uid: &str) -> Result<Option<SubscriptionMeta>> {
        let Some(id) = id_from_profile_uid(profile_uid) else {
            return Ok(None);
        };
        if !self.dir(id).is_dir() {
            return Ok(None);
        }
        Ok(Some(self.load_meta(id)?))
    }

    /// The subscription behind one profile uid, or None when the current profile
    /// is not a live OpenP2P subscription (plain profile, bundle, restored
    /// orphan).
    pub fn runtime_for_uid(&self, profile_uid: &str) -> Result<Option<SubscriptionRuntime>> {
        let Some(meta) = self.find_by_profile_uid(profile_uid)? else {
            return Ok(None);
        };
        let config = self.load_config(&meta.id)?;
        let source = self.source_view(&meta);
        Ok(Some(SubscriptionRuntime {
            profile_uid: meta.profile_uid(),
            meta,
            config,
            source,
        }))
    }

    /// Source metadata for the OpenP2P view. Parses the raw source once; the
    /// result is cached by the manager instead of being rebuilt per event.
    pub fn source_view(&self, meta: &SubscriptionMeta) -> OpenP2PSource {
        let (contains_mihomo, self_contained) = self
            .read_source(&meta.id)
            .map(|raw| {
                let parsed = portable::parse_import(&raw, None);
                (parsed.has_mihomo, parsed.self_contained())
            })
            .unwrap_or((false, false));
        OpenP2PSource {
            kind: match meta.source {
                OpenP2PImportSourceKind::Url => "url".into(),
                OpenP2PImportSourceKind::Local => "local".into(),
            },
            name: Some(meta.name.clone()),
            version: None,
            applied_at: Some(meta.imported_at),
            status: Some("applied".into()),
            contains_mihomo,
            self_contained,
        }
    }

    pub fn render(&self, meta: &SubscriptionMeta) -> Result<RenderedProfile> {
        let text = self.read_source(&meta.id)?;
        let existing = self.load_config(&meta.id).ok();
        let parsed = portable::parse_import(&text, existing.as_ref());
        if !parsed.can_apply() {
            bail!(
                "openp2p subscription {} is invalid: {}",
                meta.id,
                describe_issues(&parsed.issues)
            );
        }
        render_parsed(meta, &parsed)
    }

    pub fn prepare_import(
        &self,
        vault: &SecretVault,
        kind: OpenP2PImportSourceKind,
        text: &str,
        source_url: Option<&str>,
        profiles: &mut IProfiles,
    ) -> Result<PreparedSubscriptionImport> {
        self.prepare_import_inner(vault, kind, text, source_url, None, profiles)
    }

    pub fn prepare_update(
        &self,
        vault: &SecretVault,
        text: &str,
        id: &str,
        profiles: &mut IProfiles,
    ) -> Result<PreparedSubscriptionImport> {
        let meta = self.load_meta(id)?;
        self.prepare_import_inner(vault, meta.source, text, meta.source_url.as_deref(), Some(id), profiles)
    }

    #[allow(clippy::too_many_lines)]
    fn prepare_import_inner(
        &self,
        vault: &SecretVault,
        kind: OpenP2PImportSourceKind,
        text: &str,
        source_url: Option<&str>,
        expected_id: Option<&str>,
        profiles: &mut IProfiles,
    ) -> Result<PreparedSubscriptionImport> {
        if text.len() > portable::MAX_IMPORT_BYTES {
            bail!("the configuration exceeds 1 MiB");
        }
        let derived_name = derive_name(kind, source_url, text);
        let id = match expected_id {
            Some(id) => {
                validate_id(id)?;
                id.to_owned()
            }
            None => stable_id(kind, source_url, &derived_name, text),
        };
        let exists = self.dir(&id).is_dir();
        if expected_id.is_some() && !exists {
            bail!("openp2p subscription {id} is missing");
        }
        let existing_meta = exists.then(|| self.load_meta(&id)).transpose()?;
        let existing_config = exists.then(|| self.load_config(&id)).transpose()?;
        let parsed = portable::parse_import(text, existing_config.as_ref());
        if !parsed.can_apply() {
            bail!("the subscription is invalid: {}", describe_issues(&parsed.issues));
        }

        let name = existing_meta.as_ref().map_or(derived_name, |meta| meta.name.clone());
        let meta = SubscriptionMeta {
            id,
            name,
            source: kind,
            source_url: source_url
                .map(canonical_url)
                .filter(|_| kind == OpenP2PImportSourceKind::Url),
            imported_at: existing_meta.as_ref().map_or_else(now_secs, |meta| meta.imported_at),
            account_count: parsed.config.accounts.len(),
        };
        let rendered = render_parsed(&meta, &parsed)?;

        let profile_uid = meta.profile_uid();
        let profile_path = self.profile_path(&profile_uid);
        let undo = ImportUndo::snapshot(self, &meta.id, !exists, &profile_path)?;
        let stale_secrets = stale_refs(existing_config.as_ref(), &parsed.config);
        let config_store = ProfileStore::with_config(self.root.clone(), self.config_path(&meta.id));
        let transaction = ImportTransaction::begin(&config_store, vault, &parsed)?;
        let written = (|| -> Result<bool> {
            account::atomic_write(&self.source_path(&meta.id), text.as_bytes(), 0o600)?;
            account::atomic_write(
                &self.meta_path(&meta.id),
                serde_yaml_ng::to_string(&meta)?.as_bytes(),
                0o600,
            )?;
            account::atomic_write(&profile_path, rendered.yaml.as_bytes(), 0o600)?;
            upsert_profile_item(profiles, &meta)
        })();
        let items_changed = match written {
            Ok(changed) => changed,
            Err(error) => {
                let transaction_error = transaction.rollback().err();
                let undo_error = undo.restore().err();
                let rollback = transaction_error.or(undo_error).map(|rollback| format!("{rollback:#}"));
                return match rollback {
                    Some(rollback) => Err(error.context(format!("rollback failed: {rollback}"))),
                    None => Err(error),
                };
            }
        };
        Ok(PreparedSubscriptionImport {
            import: SubscriptionImport {
                meta,
                profile_uid,
                created: !exists,
                profile_changed: undo.profile_changed(&rendered.yaml),
                items_changed,
                warnings: rendered.warnings,
            },
            store: self.clone(),
            stale_secrets,
            transaction: Some(transaction),
            undo: Some(undo),
        })
    }

    /// Removes one subscription after the sidecar stopped its accounts.
    ///
    /// The directory is first renamed to a tombstone so no listing can observe a
    /// half-deleted subscription, then account working directories (which hold
    /// rendered credentials), vault files and the tombstone are removed.
    pub fn remove(&self, id: &str) -> Result<()> {
        validate_id(id)?;
        let dir = self.dir(id);
        if !dir.is_dir() {
            bail!("openp2p subscription {id} is missing");
        }
        let config = self.load_config(id).ok();
        let tombstone = self.subscriptions_dir().join(format!(".{id}.deleting-{}", now_secs()));
        std::fs::rename(&dir, &tombstone).with_context(|| format!("rename {}", dir.display()))?;
        if let Some(config) = &config {
            for account in &config.accounts {
                if !self.account_used_elsewhere(id, &account.id) {
                    self.purge_account_workdir(&account.id);
                }
            }
            self.remove_secrets(&refs_of(config));
        }
        match std::fs::remove_dir_all(&tombstone) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| format!("remove {}", tombstone.display()));
            }
        }
        Ok(())
    }

    fn account_used_elsewhere(&self, id: &str, account_id: &str) -> bool {
        for meta in self.list().unwrap_or_default() {
            if meta.id == id {
                continue;
            }
            if self
                .load_config(&meta.id)
                .is_ok_and(|config| config.accounts.iter().any(|account| account.id == account_id))
            {
                return true;
            }
        }
        false
    }

    fn purge_account_workdir(&self, account_id: &str) {
        if !schema::valid_slug(account_id, 32) {
            return;
        }
        let dir = self.openp2p_dir().join("accounts").join(account_id);
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => logging!(
                warn,
                Type::Network,
                "failed to remove an openp2p account directory: {error}"
            ),
        }
    }

    fn remove_secrets(&self, refs: &BTreeSet<SecretRef>) {
        if refs.is_empty() {
            return;
        }
        let used_elsewhere = self.refs_used_elsewhere();
        for secret in refs {
            if used_elsewhere.contains(secret) {
                continue;
            }
            let dir = match secret.kind {
                SecretKind::Token => "tokens",
                SecretKind::Key => "keys",
            };
            if !schema::valid_ref(&secret.reference) {
                continue;
            }
            let path = self.openp2p_dir().join(dir).join(&secret.reference);
            match std::fs::remove_file(&path) {
                Ok(()) => {
                    let root = self.openp2p_dir().join(match secret.kind {
                        SecretKind::Token => "tokens",
                        SecretKind::Key => "keys",
                    });
                    prune_empty_parents(path.parent(), &root);
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    logging!(
                        warn,
                        Type::Network,
                        "failed to remove an openp2p credential file: {error}"
                    );
                }
            }
        }
    }

    fn refs_used_elsewhere(&self) -> BTreeSet<SecretRef> {
        let mut used = BTreeSet::new();
        for meta in self.list().unwrap_or_default() {
            if let Ok(config) = self.load_config(&meta.id) {
                used.extend(refs_of(&config));
            }
        }
        used
    }
}

#[derive(Debug)]
pub struct PreparedSubscriptionImport {
    import: SubscriptionImport,
    store: SubscriptionStore,
    stale_secrets: BTreeSet<SecretRef>,
    transaction: Option<ImportTransaction>,
    undo: Option<ImportUndo>,
}

impl PreparedSubscriptionImport {
    #[cfg(test)]
    pub const fn outcome(&self) -> &SubscriptionImport {
        &self.import
    }

    /// Keeps the new files and credentials after the caller persisted the profile
    /// item list, then drops credential files the update made unreachable.
    pub fn commit(mut self) -> SubscriptionImport {
        self.undo.take();
        if let Some(transaction) = self.transaction.take() {
            transaction.finish();
        }
        self.store.remove_secrets(&self.stale_secrets);
        self.import.clone()
    }

    /// Restores the previous subscription files, credentials and generated
    /// profile after the profile item list could not be persisted.
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
struct ImportUndo {
    created: bool,
    sub_dir: PathBuf,
    source: (PathBuf, Option<Vec<u8>>),
    meta: (PathBuf, Option<Vec<u8>>),
    profile: (PathBuf, Option<Vec<u8>>),
}

impl ImportUndo {
    fn snapshot(store: &SubscriptionStore, id: &str, created: bool, profile_path: &Path) -> Result<Self> {
        let source_path = store.source_path(id);
        let meta_path = store.meta_path(id);
        Ok(Self {
            created,
            sub_dir: store.dir(id),
            source: (source_path.clone(), read_optional(&source_path)?),
            meta: (meta_path.clone(), read_optional(&meta_path)?),
            profile: (profile_path.to_path_buf(), read_optional(profile_path)?),
        })
    }

    fn profile_changed(&self, yaml: &str) -> bool {
        self.profile.1.as_deref() != Some(yaml.as_bytes())
    }

    fn restore(self) -> Result<()> {
        let mut first_error = None;
        for (path, previous) in [&self.source, &self.meta, &self.profile] {
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

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("read {}", path.display())),
    }
}

fn render_parsed(meta: &SubscriptionMeta, parsed: &ParsedImport) -> Result<RenderedProfile> {
    let header = format!(
        "# Generated by Clash Verge OpenP2P for subscription {} ({}). Edit the imported YAML, not this file.\n",
        meta.name, meta.id
    );
    profile_render::render_subscription_profile_with(
        &parsed.config,
        parsed.mihomo.as_deref(),
        &header,
        |reference| parsed.carries_token(reference) || account::token_present(reference),
        |reference| parsed.carries_key(reference) || account::key_present(reference),
    )
}

pub(crate) fn upsert_profile_item(profiles: &mut IProfiles, meta: &SubscriptionMeta) -> Result<bool> {
    let uid = meta.profile_uid();
    let file = format!("{uid}.yaml");
    let desc = format!("{PROFILE_DESC_PREFIX}{}", meta.id);
    let updated = Some(meta.imported_at as usize);
    let items = profiles.items.get_or_insert_with(Vec::new);
    let Some(item) = items.iter_mut().find(|item| item.uid.as_deref() == Some(uid.as_str())) else {
        items.push(PrfItem {
            uid: Some(uid.into()),
            itype: Some("local".into()),
            name: Some(meta.name.clone().into()),
            file: Some(file.into()),
            desc: Some(desc.into()),
            updated,
            ..PrfItem::default()
        });
        return Ok(true);
    };
    if item.itype.as_deref() != Some("local") || item.file.as_deref() != Some(file.as_str()) {
        bail!("profile {uid} is already used by a different profile item");
    }
    let name_changed = item.name.as_deref() != Some(meta.name.as_str());
    if name_changed {
        item.name = Some(meta.name.clone().into());
    }
    let desc_changed = item.desc.as_deref() != Some(desc.as_str());
    if desc_changed {
        item.desc = Some(desc.into());
    }
    let updated_changed = item.updated != updated;
    if updated_changed {
        item.updated = updated;
    }
    let option_changed = item.option.is_some();
    if option_changed {
        item.option = None;
    }
    Ok(name_changed || desc_changed || updated_changed || option_changed)
}

pub fn purge_account_dir(account_id: &str) {
    let Ok(paths) = account::account_paths(account_id) else {
        return;
    };
    match std::fs::remove_dir_all(&paths.workdir) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => logging!(
            warn,
            Type::Network,
            "failed to remove an openp2p account directory: {error}"
        ),
    }
}

pub fn profile_uid(id: &str) -> std::string::String {
    format!("{PROFILE_UID_PREFIX}{id}")
}

pub fn is_subscription_profile_uid(uid: &str) -> bool {
    id_from_profile_uid(uid).is_some()
}

pub fn id_from_profile_uid(uid: &str) -> Option<&str> {
    uid.strip_prefix(PROFILE_UID_PREFIX)
        .filter(|id| schema::valid_slug(id, 32))
}

/// A Profile Bundle may carry at most one OpenP2P subscription member; it is
/// then the sidecar source while the bundle is the current profile.
pub fn single_subscription_member<M: AsRef<str>>(members: &[M]) -> Result<Option<std::string::String>> {
    let mut subscriptions = members.iter().filter(|uid| is_subscription_profile_uid(uid.as_ref()));
    let first = subscriptions.next();
    if subscriptions.next().is_some() {
        bail!("a profile bundle can contain at most one OpenP2P subscription");
    }
    Ok(first.map(|uid| uid.as_ref().to_owned()))
}

fn validate_id(id: &str) -> Result<()> {
    if !schema::valid_slug(id, 32) {
        bail!("invalid openp2p subscription id");
    }
    Ok(())
}

fn valid_id(id: &str) -> bool {
    schema::valid_slug(id, 32)
}

fn stable_id(kind: OpenP2PImportSourceKind, source_url: Option<&str>, name: &str, text: &str) -> std::string::String {
    let material = match kind {
        OpenP2PImportSourceKind::Url => format!("url\n{}\n", canonical_url(source_url.unwrap_or_default())),
        OpenP2PImportSourceKind::Local => format!("local\n{name}\n{text}"),
    };
    let digest = Sha256::digest(material.as_bytes());
    digest.iter().take(8).map(|byte| format!("{byte:02x}")).collect()
}

fn canonical_url(raw: &str) -> std::string::String {
    reqwest::Url::parse(raw.trim()).map_or_else(|_| raw.trim().to_owned(), |url| url.to_string())
}

fn derive_name(kind: OpenP2PImportSourceKind, source_url: Option<&str>, text: &str) -> std::string::String {
    let raw = match kind {
        // The URL path may carry a capability token; only the host becomes a card name.
        OpenP2PImportSourceKind::Url => source_url
            .and_then(|url| reqwest::Url::parse(url.trim()).ok())
            .and_then(|url| url.host_str().map(str::to_owned))
            .unwrap_or_default(),
        OpenP2PImportSourceKind::Local => document_display_name(text),
    };
    normalize_name(raw)
}

fn document_display_name(text: &str) -> std::string::String {
    let Ok(value) = serde_yaml_ng::from_str::<serde_yaml_ng::Value>(text) else {
        return std::string::String::new();
    };
    let Some(document) = value.as_mapping() else {
        return std::string::String::new();
    };
    let block = document
        .get("x-openp2p")
        .and_then(serde_yaml_ng::Value::as_mapping)
        .unwrap_or(document);
    if let Some(name) = block
        .get("name")
        .and_then(serde_yaml_ng::Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        return name.to_owned();
    }
    let Some(account) = block
        .get("accounts")
        .and_then(serde_yaml_ng::Value::as_sequence)
        .and_then(|accounts| accounts.first())
        .and_then(serde_yaml_ng::Value::as_mapping)
    else {
        return std::string::String::new();
    };
    if let Some(label) = account
        .get("label")
        .and_then(serde_yaml_ng::Value::as_str)
        .map(str::trim)
        .filter(|label| !label.is_empty())
    {
        return format!("OpenP2P {label}");
    }
    account
        .get("id")
        .and_then(serde_yaml_ng::Value::as_str)
        .map_or_else(std::string::String::new, |id| format!("OpenP2P {id}"))
}

fn normalize_name(raw: std::string::String) -> std::string::String {
    let cleaned = raw
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect::<std::string::String>();
    let truncated = cleaned.trim().chars().take(64).collect::<std::string::String>();
    if truncated.is_empty() {
        "OpenP2P subscription".into()
    } else {
        truncated
    }
}

fn describe_issues(issues: &[OpenP2PIssue]) -> std::string::String {
    issues
        .iter()
        .map(|issue| format!("{}: {} ({})", issue.path, issue.message, issue.code))
        .collect::<Vec<_>>()
        .join("; ")
}

fn stale_refs(previous: Option<&OpenP2PConfig>, next: &OpenP2PConfig) -> BTreeSet<SecretRef> {
    let Some(previous) = previous else {
        return BTreeSet::new();
    };
    refs_of(previous).difference(&refs_of(next)).cloned().collect()
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum SecretKind {
    Token,
    Key,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct SecretRef {
    kind: SecretKind,
    reference: std::string::String,
}

fn refs_of(config: &OpenP2PConfig) -> BTreeSet<SecretRef> {
    let mut refs = BTreeSet::new();
    for account in &config.accounts {
        refs.insert(SecretRef {
            kind: SecretKind::Token,
            reference: account.token_ref.clone(),
        });
    }
    for proxy in &config.proxies {
        refs.insert(SecretRef {
            kind: SecretKind::Key,
            reference: proxy.private_key_ref.clone(),
        });
    }
    refs
}

fn prune_empty_parents(start: Option<&Path>, stop: &Path) {
    if stop.as_os_str().is_empty() {
        return;
    }
    let mut current = start;
    while let Some(dir) = current {
        if dir == stop || !dir.starts_with(stop) {
            break;
        }
        if std::fs::remove_dir(dir).is_err() {
            break;
        }
        current = dir.parent();
    }
}

#[cfg(test)]
#[path = "subscriptions_tests.rs"]
mod tests;
