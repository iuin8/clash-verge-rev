//! OpenP2P subscription store: one imported YAML is one subscription/profile.
//!
//! Layout below openp2p/:
//!
//! - subscriptions/<id>/source.yaml — the imported document exactly as supplied
//!   (0600). It may carry inline credentials; only the content command reads it.
//! - subscriptions/<id>/config.yaml — the normalized, secret-free x-openp2p
//!   wrapper. Keeping it also keeps generated node names and ports stable.
//! - subscriptions/<id>/meta.yaml — id, source-derived name, source kind, source
//!   URL, import time, account count and the user's display name/description.
//! - subscriptions/<id>/secrets.yaml — the staged token/key manifest (0600).
//! - subscriptions/<id>/tokens/<account-id>, keys/<sha256>.key — 0600 secrets.
//! - profiles/<profile_uid>.yaml — the projected mihomo profile (0600).
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
//! Credentials are scoped to the subscription directory, so removing the
//! subscription removes them without reference counting.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};

use crate::config::{IProfiles, PrfItem, PrfOption};
use crate::core::openp2p::account;
use crate::core::openp2p::import_transaction::{ImportUndo, PreparedSubscriptionImport, SecretTransaction};
use crate::core::openp2p::portable::{self, OpenP2PImportSourceKind, ParsedImport};
use crate::core::openp2p::profile_store::ProfileStore;
use crate::core::openp2p::schema::{self, OpenP2PConfig, OpenP2PIssue};
use crate::core::openp2p::secrets::{SecretManifest, SecretStore};
use crate::core::openp2p::state::OpenP2PSource;
use crate::core::openp2p::subscription_identity::{canonical_url, derive_name, stable_id};
use crate::core::openp2p::subscription_io::{RenderedProfile, describe_issues, inspect_source, render_parsed};
use crate::utils::dirs;
use clash_verge_logging::{Type, logging};

pub const PROFILE_UID_PREFIX: &str = "op2p-sub-";

const SOURCE_FILE: &str = "source.yaml";
const CONFIG_FILE: &str = "config.yaml";
const META_FILE: &str = "meta.yaml";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubscriptionMeta {
    pub id: std::string::String,
    /// The name derived from the imported document or URL.
    pub name: std::string::String,
    /// The card name the user chose; absent means `name` is shown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<std::string::String>,
    /// Free-form card description; never generated from the source document.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<std::string::String>,
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

    /// The card name: the user's display overlay, or the source-derived name.
    pub fn card_name(&self) -> &str {
        self.display_name.as_deref().unwrap_or(&self.name)
    }

    /// Applies a display edit; returns whether anything changed. `Some(None)` in
    /// the patch clears the custom value.
    pub(crate) fn apply_display_patch(&mut self, patch: &DisplayPatch) -> bool {
        let name_changed = patch
            .name
            .as_ref()
            .is_some_and(|name| self.display_name.as_ref() != name.as_ref());
        if name_changed {
            self.display_name = patch.name.clone().flatten();
        }
        let description_changed = patch
            .description
            .as_ref()
            .is_some_and(|description| self.description.as_ref() != description.as_ref());
        if description_changed {
            self.description = patch.description.clone().flatten();
        }
        name_changed || description_changed
    }
}

/// Display fields from a profile patch. The outer `None` means the field was not
/// part of the patch; the inner `None` clears the custom value.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct DisplayPatch {
    pub name: Option<Option<std::string::String>>,
    pub description: Option<Option<std::string::String>>,
}

impl DisplayPatch {
    /// Blank text clears the custom value so the card falls back to the source
    /// name and carries no description.
    pub(crate) fn from_fields(name: Option<&str>, description: Option<&str>) -> Self {
        fn value(raw: &str) -> Option<std::string::String> {
            (!raw.trim().is_empty()).then(|| raw.to_owned())
        }
        Self {
            name: name.map(value),
            description: description.map(value),
        }
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

/// One parsed import that has not touched disk yet. Import-time core validation
/// runs between `preview_*` and `commit_preview`.
pub struct SubscriptionPreview {
    meta: SubscriptionMeta,
    profile_uid: std::string::String,
    source: std::string::String,
    profile_yaml: std::string::String,
    created: bool,
    warnings: Vec<OpenP2PIssue>,
    parsed: ParsedImport,
    previous_manifest: SecretManifest,
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

    pub fn dir(&self, id: &str) -> PathBuf {
        self.subscriptions_dir().join(id)
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

    pub fn secrets(&self, id: &str) -> SecretStore {
        SecretStore::at(self.dir(id))
    }

    /// The subscription's staged token path for one account.
    #[cfg(test)]
    pub fn token_path(&self, id: &str, account_id: &str) -> Result<PathBuf> {
        self.secrets(id).token_path(account_id)
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

    pub fn save_meta(&self, meta: &SubscriptionMeta) -> Result<()> {
        validate_id(&meta.id)?;
        account::atomic_write(
            &self.meta_path(&meta.id),
            serde_yaml_ng::to_string(meta)?.as_bytes(),
            0o600,
        )
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

    /// Source metadata for the OpenP2P view. Reads the raw source once; the
    /// result is cached by the manager instead of being rebuilt per event.
    pub fn source_view(&self, meta: &SubscriptionMeta) -> OpenP2PSource {
        let (version, contains_mihomo, self_contained) = self
            .read_source(&meta.id)
            .map(|raw| inspect_source(&raw, &self.secrets(&meta.id)))
            .unwrap_or((None, false, false));
        OpenP2PSource {
            kind: match meta.source {
                OpenP2PImportSourceKind::Url => "url".into(),
                OpenP2PImportSourceKind::Local => "local".into(),
            },
            name: Some(meta.name.clone()),
            version,
            applied_at: Some(meta.imported_at),
            status: Some("applied".into()),
            contains_mihomo,
            self_contained,
        }
    }

    pub fn render(&self, meta: &SubscriptionMeta) -> Result<RenderedProfile> {
        let text = self.read_source(&meta.id)?;
        let existing = self.load_config(&meta.id).ok();
        let parsed = portable::parse_import(&text, existing.as_ref(), &meta.id);
        if !parsed.can_apply() {
            bail!(
                "openp2p subscription {} is invalid: {}",
                meta.id,
                describe_issues(&parsed.issues)
            );
        }
        render_parsed(meta, &parsed)
    }

    pub fn preview_import(
        &self,
        kind: OpenP2PImportSourceKind,
        text: &str,
        source_url: Option<&str>,
    ) -> Result<SubscriptionPreview> {
        self.preview_import_inner(kind, text, source_url, None)
    }

    pub fn preview_update(&self, text: &str, id: &str) -> Result<SubscriptionPreview> {
        let meta = self.load_meta(id)?;
        self.preview_import_inner(meta.source, text, meta.source_url.as_deref(), Some(id))
    }

    fn preview_import_inner(
        &self,
        kind: OpenP2PImportSourceKind,
        text: &str,
        source_url: Option<&str>,
        expected_id: Option<&str>,
    ) -> Result<SubscriptionPreview> {
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
        let parsed = portable::parse_import(text, existing_config.as_ref(), &id);
        if !parsed.can_apply() {
            bail!("the subscription is invalid: {}", describe_issues(&parsed.issues));
        }
        let name = derived_name;
        let meta = SubscriptionMeta {
            id,
            name,
            display_name: existing_meta.as_ref().and_then(|meta| meta.display_name.clone()),
            description: existing_meta.as_ref().and_then(|meta| meta.description.clone()),
            source: kind,
            source_url: source_url
                .map(canonical_url)
                .filter(|_| kind == OpenP2PImportSourceKind::Url),
            imported_at: existing_meta.as_ref().map_or_else(now_secs, |meta| meta.imported_at),
            account_count: parsed.config.accounts.len(),
        };
        let profile_uid = meta.profile_uid();
        let profile_yaml = render_parsed(&meta, &parsed)?.yaml;
        let previous_manifest = self.secrets(&meta.id).load_manifest();
        let mut warnings = parsed.warnings();
        if !parsed.has_mihomo {
            warnings.push(OpenP2PIssue::warning(
                "mihomo_section_missing",
                "",
                "the document has no top-level mihomo config; the generated profile is empty",
            ));
        }
        Ok(SubscriptionPreview {
            meta,
            profile_uid,
            source: text.to_owned(),
            profile_yaml,
            created: !exists,
            warnings,
            parsed,
            previous_manifest,
        })
    }

    pub fn commit_preview(
        &self,
        preview: SubscriptionPreview,
        profiles: &mut IProfiles,
    ) -> Result<PreparedSubscriptionImport> {
        let id = preview.meta.id.clone();
        let profile_uid = preview.profile_uid.clone();
        let profile_path = self.profile_path(&profile_uid);
        let updated_at = if preview.created {
            preview.meta.imported_at as usize
        } else {
            now_secs() as usize
        };
        let undo = ImportUndo::snapshot(self, &id, preview.created, &profile_path)?;
        let transaction = SecretTransaction::begin(&self.secrets(&id), &preview.parsed, &preview.previous_manifest)?;
        let written = (|| -> Result<bool> {
            account::atomic_write(&self.source_path(&id), preview.source.as_bytes(), 0o600)?;
            self.save_meta(&preview.meta)?;
            ProfileStore::with_config(self.root.clone(), self.config_path(&id)).save_config(&preview.parsed.config)?;
            account::atomic_write(&profile_path, preview.profile_yaml.as_bytes(), 0o600)?;
            upsert_profile_item(profiles, &preview.meta, updated_at)
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
        Ok(PreparedSubscriptionImport::new(
            SubscriptionImport {
                meta: preview.meta,
                profile_uid,
                created: preview.created,
                profile_changed: undo.profile_changed(&preview.profile_yaml),
                items_changed,
                warnings: preview.warnings,
            },
            transaction,
            undo,
        ))
    }

    #[cfg(test)]
    pub fn prepare_import(
        &self,
        kind: OpenP2PImportSourceKind,
        text: &str,
        source_url: Option<&str>,
        profiles: &mut IProfiles,
    ) -> Result<PreparedSubscriptionImport> {
        let preview = self.preview_import(kind, text, source_url)?;
        self.commit_preview(preview, profiles)
    }

    #[cfg(test)]
    pub fn prepare_update(&self, text: &str, id: &str, profiles: &mut IProfiles) -> Result<PreparedSubscriptionImport> {
        let preview = self.preview_update(text, id)?;
        self.commit_preview(preview, profiles)
    }

    /// Removes one subscription after the sidecar stopped its accounts.
    ///
    /// The directory is first renamed to a tombstone so no listing can observe a
    /// half-deleted subscription, then account working directories (which hold
    /// rendered credentials) and the tombstone (which holds the staged secrets)
    /// are removed.
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
}

/// The update settings an imported OpenP2P card starts with. The interval stays
/// unset so auto-update is off until the user edits the profile.
fn generated_profile_option() -> PrfOption {
    PrfOption {
        allow_auto_update: Some(true),
        ..PrfOption::default()
    }
}

/// `updated_at` is the profile card's last-write time: the initial import time
/// on creation, and the write time for a re-import or raw-source save so the
/// timer and the card's "last updated" text agree with what just happened.
pub(crate) fn upsert_profile_item(
    profiles: &mut IProfiles,
    meta: &SubscriptionMeta,
    updated_at: usize,
) -> Result<bool> {
    let uid = meta.profile_uid();
    let file = format!("{uid}.yaml");
    let name = meta.card_name();
    let description = meta.description.as_deref();
    let updated = Some(updated_at);
    let items = profiles.items.get_or_insert_with(Vec::new);
    let Some(item) = items.iter_mut().find(|item| item.uid.as_deref() == Some(uid.as_str())) else {
        items.push(PrfItem {
            uid: Some(uid.into()),
            itype: Some("local".into()),
            name: Some(name.into()),
            file: Some(file.into()),
            desc: description.map(Into::into),
            updated,
            option: Some(generated_profile_option()),
            ..PrfItem::default()
        });
        return Ok(true);
    };
    if item.itype.as_deref() != Some("local") || item.file.as_deref() != Some(file.as_str()) {
        bail!("profile {uid} is already used by a different profile item");
    }
    let name_changed = item.name.as_deref() != Some(name);
    if name_changed {
        item.name = Some(name.into());
    }
    let desc_changed = item.desc.as_deref() != description;
    if desc_changed {
        item.desc = description.map(Into::into);
    }
    let updated_changed = item.updated != updated;
    if updated_changed {
        item.updated = updated;
    }
    // Re-import preserves the user's update settings; only a missing option (or
    // missing auto-update flag) is repaired to the generated default.
    let option_changed = match item.option.as_mut() {
        Some(option) if option.allow_auto_update.is_none() => {
            option.allow_auto_update = Some(true);
            true
        }
        Some(_) => false,
        None => {
            item.option = Some(generated_profile_option());
            true
        }
    };
    Ok(name_changed || desc_changed || updated_changed || option_changed)
}

/// Mirrors a managed display/option patch onto the generated profile card. The
/// caller persists the subscription meta first; the card only carries a copy.
pub(crate) fn patch_profile_item(
    profiles: &mut IProfiles,
    meta: &SubscriptionMeta,
    display: &DisplayPatch,
    option: Option<PrfOption>,
) -> Result<()> {
    let uid = meta.profile_uid();
    let item = profiles
        .items
        .as_mut()
        .and_then(|items| items.iter_mut().find(|item| item.uid.as_deref() == Some(uid.as_str())))
        .ok_or_else(|| anyhow::anyhow!("the openp2p profile card {uid} is missing"))?;
    if display.name.is_some() {
        item.name = Some(meta.card_name().into());
    }
    if display.description.is_some() {
        item.desc = meta.description.as_ref().map(|description| description.as_str().into());
    }
    if let Some(option) = option {
        item.option = Some(option);
    }
    Ok(())
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

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "subscriptions_tests.rs"]
mod tests;
