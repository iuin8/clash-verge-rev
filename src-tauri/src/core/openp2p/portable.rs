//! Single-document OpenP2P import.
//!
//! Accepts either a pure OpenP2P document (`accounts`/`proxies`/`networks`) or a
//! mihomo profile carrying an `x-openp2p` block. Credentials may be inline
//! (`account.token`, `proxy.private_key`) or listed by reference under
//! `x-openp2p-secrets`. Secrets are validated, written to their 0600 files and
//! never enter the stored source or generated profiles.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use serde_yaml_ng::{Mapping, Value};

use crate::core::openp2p::account;
use crate::core::openp2p::config as o2p_config;
use crate::core::openp2p::profile_store::ProfileStore;
use crate::core::openp2p::schema::{
    self, OpenP2PConfig, OpenP2PIssue, has_errors, valid_node_name, valid_ref, valid_slug,
};

pub const MAX_IMPORT_BYTES: usize = 1024 * 1024;
const PUBLIC_PORT_MIN: u16 = 10_000;
const PUBLIC_PORT_MAX: u16 = 19_999;
const SRC_PORT_MIN: u16 = 20_000;
const SRC_PORT_MAX: u16 = 29_999;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum OpenP2PImportSourceKind {
    Local,
    Url,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretKind {
    Token,
    Key,
}

#[derive(Clone)]
struct SecretEntry {
    value: std::string::String,
}

impl std::fmt::Debug for SecretEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretEntry(***)")
    }
}

#[derive(Debug)]
pub struct ParsedImport {
    pub config: OpenP2PConfig,
    pub issues: Vec<OpenP2PIssue>,
    pub has_mihomo: bool,
    tokens: BTreeMap<std::string::String, SecretEntry>,
    keys: BTreeMap<std::string::String, SecretEntry>,
}

impl ParsedImport {
    pub fn can_apply(&self) -> bool {
        !has_errors(&self.issues)
    }

    pub(crate) fn carries_token(&self, reference: &str) -> bool {
        self.tokens.contains_key(reference)
    }

    pub(crate) fn carries_key(&self, reference: &str) -> bool {
        self.keys.contains_key(reference)
    }

    pub(crate) fn self_contained(&self) -> bool {
        if self.config.accounts.is_empty() {
            return false;
        }
        let tokens_ok = self
            .config
            .accounts
            .iter()
            .filter(|account| account.enabled)
            .all(|account| self.tokens.contains_key(&account.token_ref));
        let keys_ok = self
            .config
            .proxies
            .iter()
            .filter(|proxy| {
                self.config
                    .account(&proxy.account)
                    .is_some_and(|account| account.enabled)
            })
            .all(|proxy| self.keys.contains_key(&proxy.private_key_ref));
        tokens_ok && keys_ok
    }
}

/// Parses one document, merging device fields that this document omits from the
/// existing store so a policy-only re-import keeps the local binding.
pub fn parse_import(yaml: &str, existing: Option<&OpenP2PConfig>) -> ParsedImport {
    if yaml.len() > MAX_IMPORT_BYTES {
        return ParsedImport::failed("config_too_large", "", "the configuration exceeds 1 MiB");
    }
    let value = match serde_yaml_ng::from_str::<Value>(yaml) {
        Ok(value) => value,
        Err(error) => return ParsedImport::failed("yaml_syntax", "", error.to_string()),
    };
    let Some(document) = value.as_mapping() else {
        return ParsedImport::failed("yaml_syntax", "", "the document must be a YAML mapping");
    };

    let has_block = document.contains_key("x-openp2p");
    let is_pure =
        document.contains_key("accounts") || document.contains_key("proxies") || document.contains_key("networks");
    let (mut seed, secrets_block, has_mihomo) = if has_block {
        let Some(seed) = document.get("x-openp2p").and_then(Value::as_mapping) else {
            return ParsedImport::failed("schema_invalid", "x-openp2p", "x-openp2p must be a mapping");
        };
        let has_mihomo = document
            .keys()
            .filter_map(Value::as_str)
            .any(|key| key != "x-openp2p" && key != "x-openp2p-secrets");
        (seed.clone(), document.get("x-openp2p-secrets").cloned(), has_mihomo)
    } else if is_pure {
        (document.clone(), document.get("x-openp2p-secrets").cloned(), false)
    } else {
        return ParsedImport::failed(
            "openp2p_missing",
            "",
            "the document has no x-openp2p block and no accounts/proxies/networks",
        );
    };
    seed.remove("x-openp2p-secrets");

    let mut issues = Vec::new();
    let (mut tokens, mut keys) = read_secrets_block(secrets_block.as_ref(), &mut issues);
    if let Some(existing) = existing {
        merge_device_fields(&mut seed, existing);
    }
    ensure_default_refs(&mut seed);
    extract_inline_secrets(&mut seed, &mut issues, &mut tokens, &mut keys);
    retain_referenced(&seed, &mut tokens, &mut keys, &mut issues);
    if let Err(error) = assign_generated_fields(&mut seed) {
        issues.push(OpenP2PIssue::error(
            "device_generation_failed",
            "",
            format!("failed to allocate a local node or port: {error:#}"),
        ));
    }

    let mut config = match serde_yaml_ng::from_value::<OpenP2PConfig>(Value::Mapping(seed)) {
        Ok(config) => config,
        Err(error) => {
            let message = error.to_string();
            let code = if message.contains("unknown field") {
                "unknown_field"
            } else {
                "schema_invalid"
            };
            issues.push(OpenP2PIssue::error(code, "x-openp2p", message));
            OpenP2PConfig::default()
        }
    };
    issues.extend(schema::normalize_and_validate(&mut config));
    ParsedImport {
        config,
        issues,
        has_mihomo,
        tokens,
        keys,
    }
}

impl ParsedImport {
    fn failed(code: &str, path: impl Into<std::string::String>, message: impl Into<std::string::String>) -> Self {
        Self {
            config: OpenP2PConfig::default(),
            issues: vec![OpenP2PIssue::error(code, path, message)],
            has_mihomo: false,
            tokens: BTreeMap::new(),
            keys: BTreeMap::new(),
        }
    }
}

#[allow(clippy::type_complexity)]
fn read_secrets_block(
    value: Option<&Value>,
    issues: &mut Vec<OpenP2PIssue>,
) -> (
    BTreeMap<std::string::String, SecretEntry>,
    BTreeMap<std::string::String, SecretEntry>,
) {
    let mut tokens = BTreeMap::new();
    let mut keys = BTreeMap::new();
    let Some(value) = value else {
        return (tokens, keys);
    };
    let Some(block) = value.as_mapping() else {
        issues.push(OpenP2PIssue::error(
            "schema_invalid",
            "x-openp2p-secrets",
            "x-openp2p-secrets must be a mapping with tokens and private_keys",
        ));
        return (tokens, keys);
    };
    for key in block.keys().filter_map(Value::as_str) {
        if key != "tokens" && key != "private_keys" {
            issues.push(OpenP2PIssue::error(
                "unknown_field",
                format!("x-openp2p-secrets.{key}"),
                "x-openp2p-secrets accepts only tokens and private_keys",
            ));
        }
    }
    read_secret_map(block.get("tokens"), SecretKind::Token, "tokens", issues, &mut tokens);
    read_secret_map(
        block.get("private_keys"),
        SecretKind::Key,
        "private_keys",
        issues,
        &mut keys,
    );
    (tokens, keys)
}

fn read_secret_map(
    value: Option<&Value>,
    kind: SecretKind,
    section: &str,
    issues: &mut Vec<OpenP2PIssue>,
    out: &mut BTreeMap<std::string::String, SecretEntry>,
) {
    let Some(value) = value else {
        return;
    };
    let Some(map) = value.as_mapping() else {
        issues.push(OpenP2PIssue::error(
            "schema_invalid",
            format!("x-openp2p-secrets.{section}"),
            format!("x-openp2p-secrets.{section} must be a mapping keyed by reference"),
        ));
        return;
    };
    for (key, value) in map {
        let Some(reference) = key.as_str() else {
            issues.push(OpenP2PIssue::error(
                "invalid_ref",
                format!("x-openp2p-secrets.{section}"),
                "secret references must be strings",
            ));
            continue;
        };
        let path = format!("x-openp2p-secrets.{section}.{reference}");
        if !valid_ref(reference) {
            issues.push(OpenP2PIssue::error(
                "invalid_ref",
                path,
                "secret references must match the token_ref/private_key_ref format",
            ));
            continue;
        }
        let Some(raw) = secret_text(value) else {
            issues.push(OpenP2PIssue::error(
                "invalid_secret",
                path,
                "secret values must be strings",
            ));
            continue;
        };
        if let Err(error) = validate_secret(kind, &raw) {
            issues.push(OpenP2PIssue::error("invalid_secret", path, format!("{error:#}")));
            continue;
        }
        out.insert(reference.to_owned(), SecretEntry { value: raw });
    }
}

fn secret_text(value: &Value) -> Option<std::string::String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn validate_secret(kind: SecretKind, raw: &str) -> Result<()> {
    match kind {
        SecretKind::Token => o2p_config::parse_token(raw.trim()).map(|_| ()),
        SecretKind::Key => o2p_config::validate_private_key(raw),
    }
}

fn merge_device_fields(seed: &mut Mapping, existing: &OpenP2PConfig) {
    if let Some(Value::Sequence(accounts)) = seed.get_mut("accounts") {
        for entry in accounts.iter_mut() {
            let Some(account) = entry.as_mapping_mut() else {
                continue;
            };
            let Some(id) = account.get("id").and_then(Value::as_str).map(str::to_owned) else {
                continue;
            };
            let Some(old) = existing.account(&id) else {
                continue;
            };
            if is_auto(account.get("node")) && valid_node_name(&old.node) {
                account.insert(Value::from("node"), Value::from(old.node.as_str()));
            }
            if is_auto_port(account.get("public_ip_port")) {
                account.insert(Value::from("public_ip_port"), Value::from(old.public_ip_port));
            }
            if is_missing_string(account.get("token_ref")) && valid_ref(&old.token_ref) {
                account.insert(Value::from("token_ref"), Value::from(old.token_ref.as_str()));
            }
            if !account.contains_key("interface")
                && let Some(interface) = old.interface.as_ref()
            {
                account.insert(Value::from("interface"), Value::from(interface.as_str()));
            }
            if !account.contains_key("enabled") {
                account.insert(Value::from("enabled"), Value::Bool(old.enabled));
            }
            if let Some(Value::Sequence(apps)) = account.get_mut("apps") {
                for entry in apps.iter_mut() {
                    let Some(app) = entry.as_mapping_mut() else {
                        continue;
                    };
                    let Some(name) = app.get("name").and_then(Value::as_str) else {
                        continue;
                    };
                    let Some(old_app) = old.apps.iter().find(|candidate| candidate.name == name) else {
                        continue;
                    };
                    if is_auto_port(app.get("src_port")) {
                        app.insert(Value::from("src_port"), Value::from(old_app.src_port));
                    }
                    if !app.contains_key("enabled") {
                        app.insert(Value::from("enabled"), Value::Bool(old_app.enabled.is_enabled()));
                    }
                }
            }
        }
    }
    if let Some(Value::Sequence(proxies)) = seed.get_mut("proxies") {
        for entry in proxies.iter_mut() {
            let Some(proxy) = entry.as_mapping_mut() else {
                continue;
            };
            let Some(name) = proxy.get("name").and_then(Value::as_str) else {
                continue;
            };
            let Some(old) = existing.proxies.iter().find(|candidate| candidate.name == name) else {
                continue;
            };
            if is_missing_string(proxy.get("private_key_ref")) && valid_ref(&old.private_key_ref) {
                proxy.insert(
                    Value::from("private_key_ref"),
                    Value::from(old.private_key_ref.as_str()),
                );
            }
        }
    }
}

fn is_auto(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => true,
        Some(Value::String(text)) => {
            let text = text.trim();
            text.is_empty() || text.eq_ignore_ascii_case("auto")
        }
        _ => false,
    }
}

fn is_auto_port(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => true,
        Some(Value::String(text)) => {
            let text = text.trim();
            text.is_empty() || text.eq_ignore_ascii_case("auto")
        }
        Some(Value::Number(number)) => number.as_u64().is_none_or(|port| port == 0),
        _ => false,
    }
}

fn is_missing_string(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => true,
        Some(Value::String(text)) => text.trim().is_empty(),
        _ => false,
    }
}

fn ensure_default_refs(seed: &mut Mapping) {
    if let Some(Value::Sequence(accounts)) = seed.get_mut("accounts") {
        for entry in accounts.iter_mut() {
            let Some(account) = entry.as_mapping_mut() else {
                continue;
            };
            if !is_missing_string(account.get("token_ref")) {
                continue;
            }
            let Some(id) = account.get("id").and_then(Value::as_str) else {
                continue;
            };
            if valid_slug(id, 32) {
                account.insert(Value::from("token_ref"), Value::from(format!("o2p/{id}/token")));
            }
        }
    }
    if let Some(Value::Sequence(proxies)) = seed.get_mut("proxies") {
        for entry in proxies.iter_mut() {
            let Some(proxy) = entry.as_mapping_mut() else {
                continue;
            };
            if !is_missing_string(proxy.get("private_key_ref")) {
                continue;
            }
            let account = proxy.get("account").and_then(Value::as_str).unwrap_or_default();
            let name = proxy.get("name").and_then(Value::as_str).unwrap_or_default();
            if let Some(reference) = default_key_ref(account, name) {
                proxy.insert(Value::from("private_key_ref"), Value::from(reference));
            }
        }
    }
}

fn default_key_ref(account: &str, proxy_name: &str) -> Option<std::string::String> {
    if !valid_slug(account, 32) || !valid_slug(proxy_name, 64) {
        return None;
    }
    let sanitized: std::string::String = proxy_name.chars().map(|ch| if ch == '.' { '_' } else { ch }).collect();
    let reference = format!("o2p/{account}/{sanitized}");
    valid_ref(&reference).then_some(reference)
}

fn extract_inline_secrets(
    seed: &mut Mapping,
    issues: &mut Vec<OpenP2PIssue>,
    tokens: &mut BTreeMap<std::string::String, SecretEntry>,
    keys: &mut BTreeMap<std::string::String, SecretEntry>,
) {
    if let Some(Value::Sequence(accounts)) = seed.get_mut("accounts") {
        for (index, entry) in accounts.iter_mut().enumerate() {
            let Some(account) = entry.as_mapping_mut() else {
                continue;
            };
            let Some(value) = account.remove("token") else {
                continue;
            };
            let reference = account
                .get("token_ref")
                .and_then(Value::as_str)
                .filter(|reference| valid_ref(reference));
            let Some(reference) = reference else {
                issues.push(OpenP2PIssue::error(
                    "token_ref_invalid",
                    format!("accounts[{index}].token_ref"),
                    "an inline token needs a valid token_ref",
                ));
                continue;
            };
            let path = format!("accounts[{index}].token");
            insert_inline(tokens, reference, &value, SecretKind::Token, &path, issues);
        }
    }
    if let Some(Value::Sequence(proxies)) = seed.get_mut("proxies") {
        for (index, entry) in proxies.iter_mut().enumerate() {
            let Some(proxy) = entry.as_mapping_mut() else {
                continue;
            };
            let Some(value) = proxy.remove("private_key") else {
                continue;
            };
            let reference = proxy
                .get("private_key_ref")
                .and_then(Value::as_str)
                .filter(|reference| valid_ref(reference));
            let Some(reference) = reference else {
                issues.push(OpenP2PIssue::error(
                    "key_ref_invalid",
                    format!("proxies[{index}].private_key_ref"),
                    "an inline private key needs a valid private_key_ref",
                ));
                continue;
            };
            let path = format!("proxies[{index}].private_key");
            insert_inline(keys, reference, &value, SecretKind::Key, &path, issues);
        }
    }
}

fn insert_inline(
    out: &mut BTreeMap<std::string::String, SecretEntry>,
    reference: &str,
    value: &Value,
    kind: SecretKind,
    path: &str,
    issues: &mut Vec<OpenP2PIssue>,
) {
    let Some(raw) = secret_text(value) else {
        issues.push(OpenP2PIssue::error(
            "invalid_secret",
            path,
            "secret values must be strings",
        ));
        return;
    };
    if let Err(error) = validate_secret(kind, &raw) {
        issues.push(OpenP2PIssue::error("invalid_secret", path, format!("{error:#}")));
        return;
    }
    match out.get(reference) {
        Some(existing) if existing.value != raw => issues.push(OpenP2PIssue::error(
            "secret_conflict",
            path,
            format!("two different secrets are provided for reference {reference}"),
        )),
        Some(_) => {}
        None => {
            out.insert(reference.to_owned(), SecretEntry { value: raw });
        }
    }
}

fn retain_referenced(
    seed: &Mapping,
    tokens: &mut BTreeMap<std::string::String, SecretEntry>,
    keys: &mut BTreeMap<std::string::String, SecretEntry>,
    issues: &mut Vec<OpenP2PIssue>,
) {
    let mut token_refs = BTreeSet::new();
    let mut key_refs = BTreeSet::new();
    if let Some(Value::Sequence(accounts)) = seed.get("accounts") {
        for entry in accounts {
            if let Some(reference) = entry.get("token_ref").and_then(Value::as_str) {
                token_refs.insert(reference.to_owned());
            }
        }
    }
    if let Some(Value::Sequence(proxies)) = seed.get("proxies") {
        for entry in proxies {
            if let Some(reference) = entry.get("private_key_ref").and_then(Value::as_str) {
                key_refs.insert(reference.to_owned());
            }
        }
    }
    retain_used(tokens, &token_refs, "tokens", issues);
    retain_used(keys, &key_refs, "private_keys", issues);
}

fn retain_used(
    secrets: &mut BTreeMap<std::string::String, SecretEntry>,
    referenced: &BTreeSet<std::string::String>,
    section: &str,
    issues: &mut Vec<OpenP2PIssue>,
) {
    secrets.retain(|reference, _| {
        if referenced.contains(reference) {
            return true;
        }
        issues.push(OpenP2PIssue::warning(
            "unused_secret",
            format!("x-openp2p-secrets.{section}.{reference}"),
            "this secret is not referenced by any account or proxy and will not be written",
        ));
        false
    });
}

fn assign_generated_fields(seed: &mut Mapping) -> Result<()> {
    let mut used_ports = BTreeSet::new();
    if let Some(Value::Sequence(accounts)) = seed.get("accounts") {
        for account in accounts {
            if let Some(port) = account.get("public_ip_port").and_then(Value::as_u64) {
                #[allow(clippy::cast_possible_truncation)]
                used_ports.insert(port as u16);
            }
            if let Some(Value::Sequence(apps)) = account.get("apps") {
                for app in apps {
                    if let Some(port) = app.get("src_port").and_then(Value::as_u64) {
                        #[allow(clippy::cast_possible_truncation)]
                        used_ports.insert(port as u16);
                    }
                }
            }
        }
    }
    let mut used_nodes: BTreeSet<std::string::String> = BTreeSet::new();
    if let Some(Value::Sequence(accounts)) = seed.get("accounts") {
        for account in accounts {
            if let Some(node) = account.get("node").and_then(Value::as_str)
                && valid_node_name(node)
            {
                used_nodes.insert(node.to_owned());
            }
        }
    }
    if let Some(Value::Sequence(accounts)) = seed.get_mut("accounts") {
        for entry in accounts.iter_mut() {
            let Some(account) = entry.as_mapping_mut() else {
                continue;
            };
            if is_auto(account.get("node")) {
                account.insert(Value::from("node"), Value::from(generate_node(&mut used_nodes)?));
            }
            if is_auto_port(account.get("public_ip_port")) {
                account.insert(
                    Value::from("public_ip_port"),
                    Value::from(allocate_port(PUBLIC_PORT_MIN, PUBLIC_PORT_MAX, &mut used_ports)?),
                );
            }
            if let Some(Value::Sequence(apps)) = account.get_mut("apps") {
                for entry in apps.iter_mut() {
                    let Some(app) = entry.as_mapping_mut() else {
                        continue;
                    };
                    if is_auto_port(app.get("src_port")) {
                        app.insert(
                            Value::from("src_port"),
                            Value::from(allocate_port(SRC_PORT_MIN, SRC_PORT_MAX, &mut used_ports)?),
                        );
                    }
                }
            }
        }
    }
    Ok(())
}

fn generate_node(used: &mut BTreeSet<std::string::String>) -> Result<std::string::String> {
    for _ in 0..64 {
        let mut bytes = [0u8; 8];
        getrandom::fill(&mut bytes).context("failed to read local entropy")?;
        let suffix = bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<std::string::String>();
        let node = format!("o2p-{}", &suffix[..12]);
        if used.insert(node.clone()) {
            return Ok(node);
        }
    }
    bail!("failed to generate a unique node name");
}

fn allocate_port(min: u16, max: u16, used: &mut BTreeSet<u16>) -> Result<u16> {
    let span = u32::from(max - min) + 1;
    for _ in 0..64 {
        let mut bytes = [0u8; 2];
        getrandom::fill(&mut bytes).context("failed to read local entropy")?;
        #[allow(clippy::cast_possible_truncation)]
        let candidate = min + (u16::from_be_bytes(bytes) as u32 % span) as u16;
        if used.insert(candidate) {
            return Ok(candidate);
        }
    }
    for candidate in min..=max {
        if used.insert(candidate) {
            return Ok(candidate);
        }
    }
    bail!("no free port in {min}-{max}");
}

/// Credential files owned by the app-level store. Tokens live below `tokens/`,
/// private keys below `keys/`, both 0600 with 0700 parents.
#[derive(Debug, Clone)]
pub struct SecretVault {
    root: PathBuf,
}

impl SecretVault {
    pub fn app_default() -> Result<Self> {
        Ok(Self {
            root: account::openp2p_root()?,
        })
    }

    #[cfg(test)]
    pub const fn at(root: PathBuf) -> Self {
        Self { root }
    }

    fn file(&self, kind: SecretKind, reference: &str) -> Result<PathBuf> {
        if !valid_ref(reference) {
            bail!("invalid secret reference");
        }
        let dir = match kind {
            SecretKind::Token => "tokens",
            SecretKind::Key => "keys",
        };
        Ok(self.root.join(dir).join(reference))
    }
}

fn secret_bytes(kind: SecretKind, raw: &str) -> Result<Vec<u8>> {
    match kind {
        SecretKind::Token => {
            let token = o2p_config::parse_token(raw.trim())?;
            Ok(format!("{}\n", token.as_u64()).into_bytes())
        }
        SecretKind::Key => {
            o2p_config::validate_private_key(raw)?;
            Ok(raw.as_bytes().to_vec())
        }
    }
}

struct SecretBackup {
    path: PathBuf,
    previous: Option<Vec<u8>>,
}

pub struct ImportTransaction {
    store: ProfileStore,
    backups: Vec<SecretBackup>,
    store_previous: Option<Vec<u8>>,
    finished: bool,
}

impl std::fmt::Debug for ImportTransaction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImportTransaction")
            .field("backups", &self.backups.len())
            .field("finished", &self.finished)
            .finish()
    }
}

impl ImportTransaction {
    /// Writes credentials and the normalized source. Any failure restores the
    /// previous credential files and source.
    pub fn begin(store: &ProfileStore, vault: &SecretVault, parsed: &ParsedImport) -> Result<Self> {
        if has_errors(&parsed.issues) {
            bail!("refusing to import an invalid configuration");
        }
        let config_path = store.config_path();
        let store_previous = match std::fs::read(&config_path) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error).with_context(|| format!("read {}", config_path.display())),
        };
        let mut transaction = Self {
            store: store.clone(),
            backups: Vec::new(),
            store_previous,
            finished: false,
        };
        let result = transaction.write_secrets(vault, parsed).and_then(|()| {
            store
                .save_config(&parsed.config)
                .context("failed to save the openp2p source")
        });
        if let Err(error) = result {
            return Err(match transaction.rollback() {
                Ok(()) => error,
                Err(rollback_error) => error.context(format!("rollback failed: {rollback_error:#}")),
            });
        }
        Ok(transaction)
    }

    fn write_secrets(&mut self, vault: &SecretVault, parsed: &ParsedImport) -> Result<()> {
        for (kind, secrets) in [(SecretKind::Token, &parsed.tokens), (SecretKind::Key, &parsed.keys)] {
            for (reference, entry) in secrets {
                let path = vault.file(kind, reference)?;
                let bytes = secret_bytes(kind, &entry.value)?;
                if std::fs::read(&path).ok().as_deref() == Some(bytes.as_slice()) {
                    if let Some(parent) = path.parent() {
                        account::create_private_dir(parent)?;
                    }
                    account::ensure_private_file(&path)?;
                    continue;
                }
                let previous = std::fs::read(&path).ok();
                account::atomic_write(&path, &bytes, 0o600)
                    .with_context(|| format!("failed to write credential {reference}"))?;
                self.backups.push(SecretBackup { path, previous });
            }
        }
        Ok(())
    }

    /// Keeps the credentials and source after the caller's projection succeeded.
    pub fn finish(mut self) {
        self.finished = true;
    }

    /// Restores every touched credential file and the previous source.
    pub fn rollback(mut self) -> Result<()> {
        self.finished = true;
        self.rollback_inner()
    }

    fn rollback_inner(&self) -> Result<()> {
        let mut first_error = None;
        for backup in self.backups.iter().rev() {
            let result = match backup.previous.as_ref() {
                Some(bytes) => account::atomic_write(&backup.path, bytes, 0o600),
                None => match std::fs::remove_file(&backup.path) {
                    Ok(()) => Ok(()),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Err(error) => Err(error.into()),
                },
            };
            if let Err(error) = result
                && first_error.is_none()
            {
                first_error = Some(error.context(format!("failed to restore {}", backup.path.display())));
            }
        }
        let config_path = self.store.config_path();
        let result = match self.store_previous.as_ref() {
            Some(bytes) => account::atomic_write(&config_path, bytes, 0o600),
            None => match std::fs::remove_file(&config_path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error.into()),
            },
        };
        if let Err(error) = result
            && first_error.is_none()
        {
            first_error = Some(error.context("failed to restore the openp2p source"));
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

impl Drop for ImportTransaction {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        if let Err(error) = self.rollback_inner() {
            clash_verge_logging::logging!(
                warn,
                clash_verge_logging::Type::Config,
                "openp2p import rollback failed: {error:#}"
            );
        }
    }
}

#[cfg(test)]
#[path = "portable_tests.rs"]
mod tests;
