use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_yaml_ng::Value;

use crate::config::{Config, IProfiles, PrfItem, PrfOption, profiles::PROFILE_WRITE_LOCK};
use crate::core::openp2p::account::{self, artifacts_manifest_path, token_present};
#[path = "artifacts_build.rs"]
mod build;
#[path = "artifacts_files.rs"]
mod files;
use crate::core::CoreManager;
use crate::core::openp2p::schema::{self, GROUP_NAME, OpenP2PConfig, OpenP2PIssue, has_errors};
use crate::core::openp2p::state::{OpenP2PArtifacts, OpenP2PPreviewAction};
use crate::core::validate::ValidationOutcome;
use crate::utils::{dirs, help};
use build::{build_artifacts, network_dns, network_rules};
use clash_verge_logging::{Type, logging};
use files::{
    FileSnapshot, apply_seq_files, cleanup_merge, cleanup_seq, read_mapping_sync, read_seq, save_seq_sync, write_merge,
};

#[derive(Debug)]
pub enum ApplyError {
    Invalid(Vec<OpenP2PIssue>),
    Rejected(ValidationOutcome),
    Profile(std::string::String),
    Io(anyhow::Error),
}

impl std::fmt::Display for ApplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(issues) => {
                let codes: Vec<&str> = issues.iter().map(|issue| issue.code.as_str()).collect();
                write!(f, "x-openp2p validation failed: {}", codes.join(", "))
            }
            Self::Rejected(outcome) => write!(f, "mihomo rejected the generated configuration: {outcome}"),
            Self::Profile(message) => f.write_str(message),
            Self::Io(error) => write!(f, "{error:#}"),
        }
    }
}

impl std::error::Error for ApplyError {}

impl From<anyhow::Error> for ApplyError {
    fn from(error: anyhow::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Clone, Debug)]
pub struct LoadedConfig {
    pub profile_uid: std::string::String,
    pub config: OpenP2PConfig,
    pub issues: Vec<OpenP2PIssue>,
}

#[derive(Clone, Debug)]
pub struct ApplyReport {
    pub artifacts: OpenP2PArtifacts,
    pub validation: ValidationOutcome,
}

#[derive(Clone, Debug)]
pub enum SyncOutcome {
    Applied(ApplyReport),
    Cleared,
    Unchanged,
}

#[derive(Default, Serialize, Deserialize)]
struct ArtifactsManifest {
    #[serde(default)]
    profiles: BTreeMap<std::string::String, ProfileArtifacts>,
}

#[derive(Clone, Serialize, Deserialize)]
struct ProfileArtifacts {
    profile_uid: std::string::String,
    merge_uid: std::string::String,
    proxies_uid: std::string::String,
    groups_uid: std::string::String,
    rules_uid: std::string::String,
    #[serde(default)]
    proxy_names: Vec<std::string::String>,
    #[serde(default)]
    group_name: Option<std::string::String>,
    #[serde(default)]
    rules: Vec<std::string::String>,
    #[serde(default)]
    dns_domains: Vec<std::string::String>,
    #[serde(default)]
    dns_previous: BTreeMap<std::string::String, Option<Value>>,
    applied_at: i64,
}

impl ProfileArtifacts {
    fn view(&self) -> OpenP2PArtifacts {
        OpenP2PArtifacts {
            profile_uid: Some(self.profile_uid.clone()),
            proxies: self.proxy_names.clone(),
            groups: self.group_name.iter().cloned().collect(),
            enhanced: self.group_name.is_some(),
            error: None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum ChainKind {
    Merge,
    Proxies,
    Groups,
    Rules,
}

impl ChainKind {
    const fn prefix(self) -> &'static str {
        match self {
            Self::Merge => "m",
            Self::Proxies => "p",
            Self::Groups => "g",
            Self::Rules => "r",
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Merge => "merge",
            Self::Proxies => "proxies",
            Self::Groups => "groups",
            Self::Rules => "rules",
        }
    }
}

#[derive(Debug, Clone)]
struct ChainSlot {
    uid: std::string::String,
    path: PathBuf,
}

struct ChainPlan {
    committed: IProfiles,
    candidate: IProfiles,
    profile_uid: std::string::String,
    merge: ChainSlot,
    proxies: ChainSlot,
    groups: ChainSlot,
    rules: ChainSlot,
}

fn option_uid(item: &PrfItem, kind: ChainKind) -> Option<&str> {
    let option = item.option.as_ref()?;
    let value = match kind {
        ChainKind::Merge => option.merge.as_ref(),
        ChainKind::Proxies => option.proxies.as_ref(),
        ChainKind::Groups => option.groups.as_ref(),
        ChainKind::Rules => option.rules.as_ref(),
    }?;
    Some(value.as_str())
}

fn set_option_uid(item: &mut PrfItem, kind: ChainKind, uid: std::string::String) {
    let option = item.option.get_or_insert_with(PrfOption::default);
    match kind {
        ChainKind::Merge => option.merge = Some(uid.into()),
        ChainKind::Proxies => option.proxies = Some(uid.into()),
        ChainKind::Groups => option.groups = Some(uid.into()),
        ChainKind::Rules => option.rules = Some(uid.into()),
    }
}

fn reference_count(items: &[PrfItem], kind: ChainKind, uid: &str) -> usize {
    items
        .iter()
        .filter(|item| option_uid(item, kind).is_some_and(|value| value == uid))
        .count()
}

impl ChainPlan {
    async fn prepare() -> Result<Self> {
        let committed_ref = Config::profiles().await.latest_arc();
        let committed: IProfiles = (**committed_ref).clone();
        let Some(profile_uid) = committed.current.as_ref().map(ToString::to_string) else {
            bail!("no profile is selected");
        };
        if profile_uid.as_str() == crate::config::bundle::BUNDLE_UID {
            bail!("profile bundles do not own per-profile chain items");
        }
        let mut items = committed.items.clone().unwrap_or_default();
        let profile_index = items
            .iter()
            .position(|item| item.uid.as_deref() == Some(profile_uid.as_str()))
            .ok_or_else(|| anyhow::anyhow!("the current profile is missing from profiles.yaml"))?;
        let mut profile = items[profile_index].clone();
        let profiles_dir = dirs::app_profiles_dir()?;
        let mut slots = [
            ChainKind::Merge,
            ChainKind::Proxies,
            ChainKind::Groups,
            ChainKind::Rules,
        ]
        .into_iter()
        .map(|kind| -> Result<ChainSlot> {
            let existing = option_uid(&profile, kind).map(str::to_owned);
            let mut usable = None;
            if let Some(uid) = existing.as_ref() {
                let shared = reference_count(&items, kind, uid) != 1;
                let global_merge = matches!(kind, ChainKind::Merge) && uid == "Merge";
                let expected = kind.label();
                if !shared
                    && !global_merge
                    && let Some(item) = items.iter().find(|item| item.uid.as_deref() == Some(uid.as_str()))
                    && item.itype.as_deref() == Some(expected)
                    && let Some(file) = item.file.as_ref()
                {
                    usable = Some((uid.clone(), file.to_string()));
                }
            }
            let (uid, file) = match usable {
                Some((uid, file)) => (uid, file),
                None => {
                    let uid: std::string::String = help::get_uid(kind.prefix());
                    let file = format!("{uid}.yaml");
                    let item = PrfItem {
                        uid: Some(uid.clone().into()),
                        itype: Some(kind.label().into()),
                        file: Some(file.clone().into()),
                        updated: Some(chrono::Local::now().timestamp() as usize),
                        ..PrfItem::default()
                    };
                    items.push(item);
                    (uid, file)
                }
            };
            set_option_uid(&mut profile, kind, uid.clone());
            Ok(ChainSlot {
                path: profiles_dir.join(&file),
                uid,
            })
        })
        .collect::<Result<Vec<ChainSlot>>>()?;
        let rules = slots.pop().expect("four chain slots");
        let groups = slots.pop().expect("four chain slots");
        let proxies = slots.pop().expect("four chain slots");
        let merge = slots.pop().expect("four chain slots");
        items[profile_index] = profile;
        let mut candidate = committed.clone();
        candidate.items = Some(items);

        Ok(Self {
            committed,
            candidate,
            profile_uid,
            merge,
            proxies,
            groups,
            rules,
        })
    }

    fn paths(&self) -> Vec<PathBuf> {
        vec![
            self.merge.path.clone(),
            self.proxies.path.clone(),
            self.groups.path.clone(),
            self.rules.path.clone(),
        ]
    }
}

struct ArtifactSet {
    proxies: Vec<Value>,
    proxy_names: Vec<std::string::String>,
    group: Option<Value>,
    rules: Vec<Value>,
    rule_texts: Vec<std::string::String>,
    dns: BTreeMap<std::string::String, Value>,
    warnings: Vec<OpenP2PIssue>,
}

impl ArtifactSet {
    const fn empty() -> Self {
        Self {
            proxies: Vec::new(),
            proxy_names: Vec::new(),
            group: None,
            rules: Vec::new(),
            rule_texts: Vec::new(),
            dns: BTreeMap::new(),
            warnings: Vec::new(),
        }
    }

    fn names(
        &self,
    ) -> (
        Vec<std::string::String>,
        Vec<std::string::String>,
        Vec<std::string::String>,
        Vec<std::string::String>,
    ) {
        let groups = if self.group.is_some() {
            vec![GROUP_NAME.to_owned()]
        } else {
            Vec::new()
        };
        let dns = self
            .dns
            .iter()
            .map(|(domain, value)| format!("{domain} -> {}", display_dns_value(value)))
            .collect();
        (self.proxy_names.clone(), groups, self.rule_texts.clone(), dns)
    }
}

fn display_dns_value(value: &Value) -> std::string::String {
    match value {
        Value::Sequence(sequence) => sequence
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<&str>>()
            .join(","),
        other => serde_yaml_ng::to_string(other).unwrap_or_default().trim().to_owned(),
    }
}

fn load_manifest() -> ArtifactsManifest {
    let Ok(path) = artifacts_manifest_path() else {
        return ArtifactsManifest::default();
    };
    if !path.exists() {
        return ArtifactsManifest::default();
    }
    match std::fs::read_to_string(&path)
        .ok()
        .and_then(|raw| serde_yaml_ng::from_str(&raw).ok())
    {
        Some(manifest) => manifest,
        None => {
            logging!(
                warn,
                Type::Config,
                "openp2p artifacts manifest is unreadable; ignoring it"
            );
            ArtifactsManifest::default()
        }
    }
}

fn save_manifest(manifest: &ArtifactsManifest) -> Result<()> {
    let path = artifacts_manifest_path()?;
    if let Some(parent) = path.parent() {
        account::create_private_dir(parent)?;
    }
    let raw = serde_yaml_ng::to_string(manifest)?;
    account::atomic_write(&path, raw.as_bytes(), 0o600)
}

async fn commit(plan: &ChainPlan) -> Result<ValidationOutcome> {
    match CoreManager::global()
        .update_config_forced_with_profiles(&plan.candidate, &plan.committed)
        .await
    {
        Ok(Ok(guard)) => {
            drop(guard);
            Ok(ValidationOutcome::Valid)
        }
        Ok(Err(outcome)) => Ok(outcome),
        Err(error) => Err(error),
    }
}

fn new_manifest_entry(
    plan: &ChainPlan,
    artifacts: &ArtifactSet,
    previous: BTreeMap<std::string::String, Option<Value>>,
) -> ProfileArtifacts {
    ProfileArtifacts {
        profile_uid: plan.profile_uid.clone(),
        merge_uid: plan.merge.uid.clone(),
        proxies_uid: plan.proxies.uid.clone(),
        groups_uid: plan.groups.uid.clone(),
        rules_uid: plan.rules.uid.clone(),
        proxy_names: artifacts.proxy_names.clone(),
        group_name: artifacts.group.as_ref().map(|_| GROUP_NAME.to_owned()),
        rules: artifacts.rule_texts.clone(),
        dns_domains: artifacts.dns.keys().cloned().collect(),
        dns_previous: previous,
        applied_at: chrono::Local::now().timestamp(),
    }
}

fn existing_artifacts(plan: &ChainPlan) -> Option<ProfileArtifacts> {
    if !plan.merge.path.exists() {
        return None;
    }
    let mapping = read_mapping_sync(&plan.merge.path).ok()?;
    let value = mapping.get(Value::from("x-openp2p"))?;
    let yaml = serde_yaml_ng::to_string(value).ok()?;
    let parsed = schema::parse_seed(&yaml);
    if schema::has_errors(&parsed.issues) {
        return None;
    }
    let config = parsed.config;
    let generated = !config.proxies.is_empty();
    Some(ProfileArtifacts {
        profile_uid: plan.profile_uid.clone(),
        merge_uid: plan.merge.uid.clone(),
        proxies_uid: plan.proxies.uid.clone(),
        groups_uid: plan.groups.uid.clone(),
        rules_uid: plan.rules.uid.clone(),
        proxy_names: config.proxies.iter().map(|proxy| proxy.name.clone()).collect(),
        group_name: generated.then(|| GROUP_NAME.to_owned()),
        rules: if generated { network_rules(&config) } else { Vec::new() },
        dns_domains: if generated {
            network_dns(&config).keys().cloned().collect()
        } else {
            Vec::new()
        },
        dns_previous: BTreeMap::new(),
        applied_at: 0,
    })
}

async fn apply_locked(config: OpenP2PConfig) -> std::result::Result<ApplyReport, ApplyError> {
    let plan = ChainPlan::prepare()
        .await
        .map_err(|error| ApplyError::Profile(format!("{error:#}")))?;
    let artifacts = build_artifacts(&config, true);
    let old_manifest = load_manifest();
    let old = old_manifest
        .profiles
        .get(&plan.profile_uid)
        .cloned()
        .or_else(|| existing_artifacts(&plan));

    let mut paths = plan.paths();
    paths.push(artifacts_manifest_path()?);
    let snapshot = FileSnapshot::capture(&paths);

    let result = (|| -> Result<()> {
        let previous = write_merge(&plan.merge.path, &config, old.as_ref(), &artifacts.dns)?;
        apply_seq_files(&plan, old.as_ref(), &artifacts)?;
        let mut manifest = load_manifest();
        manifest.profiles.insert(
            plan.profile_uid.clone(),
            new_manifest_entry(&plan, &artifacts, previous),
        );
        save_manifest(&manifest)
    })();

    if let Err(error) = result {
        let rollback = snapshot.restore();
        return Err(match rollback {
            Ok(()) => ApplyError::Io(error),
            Err(rollback_error) => ApplyError::Io(error.context(format!("{rollback_error:#}"))),
        });
    }

    let outcome = commit(&plan).await.map_err(ApplyError::Io)?;
    if !outcome.is_valid() {
        snapshot
            .restore()
            .map_err(|error| ApplyError::Io(error.context(format!("configuration was rejected: {outcome}"))))?;
        return Err(ApplyError::Rejected(outcome));
    }

    let manifest = load_manifest();
    let entry = manifest.profiles.get(&plan.profile_uid);
    Ok(ApplyReport {
        artifacts: entry.map(ProfileArtifacts::view).unwrap_or_default(),
        validation: outcome,
    })
}

/// Removes materialized nodes before the core starts when the credential files are gone
/// (for example after restoring a CVR backup, which intentionally excludes tokens and keys).
pub async fn strip_stale_artifacts() -> Result<bool> {
    let _profile_write = PROFILE_WRITE_LOCK.lock().await;
    let profiles = std::sync::Arc::clone(&Config::profiles().await.latest_arc());
    let Some(loaded) = load_from_profiles(&profiles)? else {
        return Ok(false);
    };
    if has_errors(&loaded.issues) {
        return Ok(false);
    }
    let artifacts = build_artifacts(&loaded.config, true);
    if !artifacts.proxy_names.is_empty() {
        return Ok(false);
    }
    let plan = ChainPlan::prepare()
        .await
        .map_err(|error| anyhow::anyhow!("{error:#}"))?;
    let old = load_manifest()
        .profiles
        .get(&plan.profile_uid)
        .cloned()
        .or_else(|| existing_artifacts(&plan));
    let Some(old) = old else {
        return Ok(false);
    };
    if old.proxy_names.is_empty() && old.rules.is_empty() && old.dns_domains.is_empty() {
        return Ok(false);
    }

    let mut paths = plan.paths();
    paths.push(artifacts_manifest_path()?);
    let snapshot = FileSnapshot::capture(&paths);
    let result = (|| -> Result<()> {
        let previous = write_merge(&plan.merge.path, &loaded.config, Some(&old), &BTreeMap::new())?;
        apply_seq_files(&plan, Some(&old), &ArtifactSet::empty())?;
        let mut manifest = load_manifest();
        manifest.profiles.insert(
            plan.profile_uid.clone(),
            new_manifest_entry(&plan, &ArtifactSet::empty(), previous),
        );
        save_manifest(&manifest)
    })();
    match result {
        Ok(()) => Ok(true),
        Err(error) => {
            let rollback = snapshot.restore();
            Err(match rollback {
                Ok(()) => error,
                Err(rollback_error) => error.context(format!("{rollback_error:#}")),
            })
        }
    }
}

pub async fn apply_seed(yaml: &str) -> std::result::Result<ApplyReport, ApplyError> {
    let parsed = schema::parse_seed(yaml);
    if has_errors(&parsed.issues) {
        return Err(ApplyError::Invalid(parsed.issues));
    }
    let _profile_write = PROFILE_WRITE_LOCK.lock().await;
    apply_locked(parsed.config).await
}

pub async fn load_current() -> Result<Option<LoadedConfig>> {
    let profiles = std::sync::Arc::clone(&Config::profiles().await.latest_arc());
    load_from_profiles(&profiles)
}

/// Imports the one-time `x-openp2p` seed carried by the active profile itself.
///
/// The attached Merge chain item is the runtime source of truth. A profile that
/// was imported with a top-level `x-openp2p` block seeds that Merge item once;
/// later edits go through the OpenP2P page and are not overwritten by this
/// function because an existing Merge block wins.
///
/// Returns `None` when the active profile has no seed or the Merge item is
/// already configured.
pub async fn seed_from_current_profile() -> Result<Option<ApplyReport>> {
    let profiles = std::sync::Arc::clone(&Config::profiles().await.latest_arc());
    let Some(profile_uid) = profiles.current.clone() else {
        return Ok(None);
    };
    if profile_uid.as_str() == crate::config::bundle::BUNDLE_UID {
        return Ok(None);
    }
    if load_from_profiles(&profiles)?.is_some() {
        return Ok(None);
    }
    let profile = profiles.get_item(&profile_uid)?;
    let Some(file) = profile.file.as_ref() else {
        return Ok(None);
    };
    let path = dirs::app_profiles_dir()?.join(file.as_str());
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Ok(None);
    };
    let Ok(value) = serde_yaml_ng::from_str::<Value>(&raw) else {
        return Ok(None);
    };
    let Some(document) = value.as_mapping() else {
        return Ok(None);
    };
    let Some(seed) = document.get(Value::from("x-openp2p")) else {
        return Ok(None);
    };
    let yaml = serde_yaml_ng::to_string(seed)?;
    let report = apply_seed(&yaml).await.map_err(anyhow::Error::new)?;
    Ok(Some(report))
}

fn load_from_profiles(profiles: &IProfiles) -> Result<Option<LoadedConfig>> {
    let Some(profile_uid) = profiles.current.clone() else {
        return Ok(None);
    };
    if profile_uid.as_str() == crate::config::bundle::BUNDLE_UID {
        bail!("the active profile is a bundle; bundles skip per-profile chain items");
    }
    let profile = profiles.get_item(&profile_uid)?;
    let Some(merge_uid) = profile.option.as_ref().and_then(|option| option.merge.clone()) else {
        return Ok(None);
    };
    let merge_item = profiles.get_item(&merge_uid)?;
    let Some(file) = merge_item.file.as_ref() else {
        return Ok(None);
    };
    let path = dirs::app_profiles_dir()?.join(file.as_str());
    if !path.exists() {
        return Ok(None);
    }
    let mapping = read_mapping_sync(&path)?;
    let Some(value) = mapping.get("x-openp2p") else {
        return Ok(None);
    };
    let yaml = serde_yaml_ng::to_string(value)?;
    let parsed = schema::parse_seed(&yaml);
    Ok(Some(LoadedConfig {
        profile_uid: profile_uid.to_string(),
        config: parsed.config,
        issues: parsed.issues,
    }))
}

pub async fn sync_current() -> Result<SyncOutcome> {
    let _profile_write = PROFILE_WRITE_LOCK.lock().await;
    let profiles = std::sync::Arc::clone(&Config::profiles().await.latest_arc());
    let Some(profile_uid) = profiles.current.clone() else {
        return Ok(SyncOutcome::Unchanged);
    };
    match load_from_profiles(&profiles) {
        Ok(Some(loaded)) => {
            if has_errors(&loaded.issues) {
                let codes: Vec<&str> = loaded.issues.iter().map(|issue| issue.code.as_str()).collect();
                bail!("x-openp2p validation failed: {}", codes.join(", "));
            }
            let report = apply_locked(loaded.config).await.map_err(anyhow::Error::new)?;
            Ok(SyncOutcome::Applied(report))
        }
        Ok(None) => cleanup_locked(&profiles, profile_uid.as_str()).await,
        Err(error) => Err(error),
    }
}

async fn cleanup_locked(profiles: &IProfiles, profile_uid: &str) -> Result<SyncOutcome> {
    let manifest = load_manifest();
    let Some(old) = manifest.profiles.get(profile_uid).cloned() else {
        return Ok(SyncOutcome::Unchanged);
    };
    let profile = profiles.get_item(profile_uid)?;
    let profiles_dir = dirs::app_profiles_dir()?;
    let mut paths = vec![artifacts_manifest_path()?];
    let mut chain = Vec::new();
    for (kind, uid) in [
        (ChainKind::Merge, old.merge_uid.clone()),
        (ChainKind::Proxies, old.proxies_uid.clone()),
        (ChainKind::Groups, old.groups_uid.clone()),
        (ChainKind::Rules, old.rules_uid.clone()),
    ] {
        let attached = option_uid(profile, kind).is_some_and(|value| value == uid.as_str());
        if !attached {
            continue;
        }
        if let Ok(item) = profiles.get_item(&uid)
            && let Some(file) = item.file.as_ref()
        {
            let path = profiles_dir.join(file.as_str());
            chain.push((kind, path.clone()));
            paths.push(path);
        }
    }
    let snapshot = FileSnapshot::capture(&paths);
    let result = (|| -> Result<()> {
        for (kind, path) in &chain {
            match kind {
                ChainKind::Merge => cleanup_merge(path, &old)?,
                ChainKind::Proxies => {
                    let mut seq = read_seq(path)?;
                    cleanup_seq(&mut seq, Some(&old), &BTreeSet::new(), &BTreeSet::new());
                    save_seq_sync(path, &seq)?;
                }
                ChainKind::Groups => {
                    let mut seq = read_seq(path)?;
                    cleanup_seq(&mut seq, Some(&old), &BTreeSet::new(), &BTreeSet::new());
                    save_seq_sync(path, &seq)?;
                }
                ChainKind::Rules => {
                    let mut seq = read_seq(path)?;
                    cleanup_seq(&mut seq, Some(&old), &BTreeSet::new(), &BTreeSet::new());
                    save_seq_sync(path, &seq)?;
                }
            }
        }
        let mut manifest = load_manifest();
        manifest.profiles.remove(profile_uid);
        save_manifest(&manifest)
    })();
    if let Err(error) = result {
        let _ = snapshot.restore();
        return Err(error);
    }
    if let Err(error) = CoreManager::global().update_config_checked().await {
        snapshot.restore()?;
        return Err(error);
    }
    Ok(SyncOutcome::Cleared)
}

pub async fn delete_account(account_id: &str) -> std::result::Result<ApplyReport, ApplyError> {
    let _profile_write = PROFILE_WRITE_LOCK.lock().await;
    let loaded = load_current().await.map_err(ApplyError::Io)?;
    let Some(mut loaded) = loaded else {
        return Err(ApplyError::Profile(
            "openp2p is not configured on the current profile".into(),
        ));
    };
    let before = loaded.config.accounts.len();
    loaded.config.accounts.retain(|account| account.id != account_id);
    loaded.config.proxies.retain(|proxy| proxy.account != account_id);
    if loaded.config.accounts.len() == before {
        return Err(ApplyError::Profile("openp2p account not found".into()));
    }
    apply_locked(loaded.config).await
}

pub fn view(profile_uid: Option<&str>) -> OpenP2PArtifacts {
    let Some(profile_uid) = profile_uid else {
        return OpenP2PArtifacts::default();
    };
    load_manifest()
        .profiles
        .get(profile_uid)
        .map(ProfileArtifacts::view)
        .unwrap_or_default()
}

pub async fn current_profile_uid() -> Option<std::string::String> {
    Config::profiles()
        .await
        .latest_arc()
        .current
        .as_ref()
        .map(ToString::to_string)
}

pub fn preview_artifacts(
    config: &OpenP2PConfig,
) -> (
    Vec<std::string::String>,
    Vec<std::string::String>,
    Vec<std::string::String>,
    Vec<std::string::String>,
) {
    build_artifacts(config, false).names()
}

pub async fn preview_account_actions(
    config: &OpenP2PConfig,
) -> Vec<crate::core::openp2p::state::OpenP2PAccountPreview> {
    let previous = load_current().await.ok().flatten().map(|loaded| loaded.config);
    config
        .accounts
        .iter()
        .map(|account| {
            let action = match previous.as_ref().and_then(|old| old.account(&account.id)) {
                None => OpenP2PPreviewAction::Add,
                Some(old) if old == account => OpenP2PPreviewAction::Unchanged,
                Some(_) => OpenP2PPreviewAction::Update,
            };
            let mut warnings = Vec::new();
            if !token_present(&account.token_ref) {
                warnings.push("token_missing".into());
            }
            if !account::account_key_present(config, &account.id) {
                warnings.push("key_missing".into());
            }
            crate::core::openp2p::state::OpenP2PAccountPreview {
                id: account.id.clone(),
                label: account.label().to_owned(),
                action,
                warnings,
            }
        })
        .collect()
}
#[cfg(test)]
#[path = "artifacts_tests.rs"]
mod tests;
