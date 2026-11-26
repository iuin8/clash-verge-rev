//! Single-document OpenP2P import.
//!
//! A document is OpenP2P only when the top-level mapping has `x-openp2p`. The
//! block carries accounts (with inline tokens), apps and advisory networks;
//! every proxy, group, rule and DNS entry is ordinary mihomo config. Import
//! stages credentials into the subscription directory, strips them from the
//! normalized config and projects the runtime profile.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use serde_yaml_ng::{Mapping, Value};

use crate::core::openp2p::schema::{DOCUMENT_KEY, OpenP2PConfig, OpenP2PIssue, has_errors, valid_node_name};
use crate::core::openp2p::{projection, proxies, schema, schema_validate};

pub const MAX_IMPORT_BYTES: usize = 1024 * 1024;
const PUBLIC_PORT_MIN: u16 = 10_000;
const PUBLIC_PORT_MAX: u16 = 19_999;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum OpenP2PImportSourceKind {
    Local,
    Url,
}

#[derive(Clone)]
pub(crate) struct SecretEntry {
    pub(crate) value: std::string::String,
}

impl std::fmt::Debug for SecretEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretEntry(***)")
    }
}

pub struct ParsedImport {
    pub config: OpenP2PConfig,
    pub issues: Vec<OpenP2PIssue>,
    pub has_mihomo: bool,
    /// Projected runtime profile body; absent while any error is unresolved.
    profile: Option<std::string::String>,
    tokens: BTreeMap<std::string::String, SecretEntry>,
    keys: BTreeMap<std::string::String, SecretEntry>,
}

impl ParsedImport {
    pub fn can_apply(&self) -> bool {
        !has_errors(&self.issues)
    }

    pub fn warnings(&self) -> Vec<OpenP2PIssue> {
        self.issues.iter().filter(|issue| !issue.is_error()).cloned().collect()
    }

    pub fn profile_yaml(&self) -> Option<&str> {
        self.profile.as_deref()
    }

    pub(crate) fn tokens(&self) -> impl Iterator<Item = (&str, &str)> {
        self.tokens
            .iter()
            .map(|(id, entry)| (id.as_str(), entry.value.as_str()))
    }

    pub(crate) fn keys(&self) -> impl Iterator<Item = (&str, &str)> {
        self.keys
            .iter()
            .map(|(digest, entry)| (digest.as_str(), entry.value.as_str()))
    }
}

/// A document is OpenP2P iff the top-level mapping has `x-openp2p`.
pub fn is_openp2p_document(yaml: &str) -> bool {
    let Ok(value) = serde_yaml_ng::from_str::<Value>(yaml) else {
        return false;
    };
    value
        .as_mapping()
        .is_some_and(|document| document.contains_key(DOCUMENT_KEY))
}

/// Parses one document, merging device fields that this document omits from the
/// existing store so a policy-only re-import keeps the local binding.
pub fn parse_import(yaml: &str, existing: Option<&OpenP2PConfig>, subscription_id: &str) -> ParsedImport {
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
    let Some(block) = document.get(DOCUMENT_KEY) else {
        return ParsedImport::failed(
            "x_openp2p_missing",
            "",
            "the document has no x-openp2p block; OpenP2P accounts must live below x-openp2p and proxies below the top-level mihomo keys",
        );
    };
    let Some(seed) = block.as_mapping() else {
        return ParsedImport::failed("schema_invalid", DOCUMENT_KEY, "x-openp2p must be a mapping");
    };
    let has_mihomo = document.keys().any(|key| key.as_str() != Some(DOCUMENT_KEY));
    let legacy = schema::legacy_document_issues(document, seed);
    if has_errors(&legacy) {
        return ParsedImport::failed_with(legacy, has_mihomo);
    }

    let mut seed = seed.clone();
    if let Some(existing) = existing {
        merge_device_fields(&mut seed, existing);
    }
    if let Err(error) = assign_generated_fields(&mut seed) {
        return ParsedImport::failed(
            "device_generation_failed",
            "",
            format!("failed to allocate a local node or port: {error:#}"),
        );
    }

    let mut config = match serde_yaml_ng::from_value::<OpenP2PConfig>(Value::Mapping(seed)) {
        Ok(config) => config,
        Err(error) => return ParsedImport::failed_with(vec![schema::schema_issue(&error)], has_mihomo),
    };
    let mut issues = schema_validate::normalize_and_validate(&mut config);
    issues.extend(proxies::validate_proxies(document, &config));

    let mut tokens = BTreeMap::new();
    for account in &mut config.accounts {
        let Some(token) = account.token.take() else {
            continue;
        };
        tokens.insert(account.id.clone(), SecretEntry { value: token });
    }

    let (profile, keys) = match projection::project(document, subscription_id, &config) {
        Ok(projected) => {
            let keys = projected
                .keys
                .into_iter()
                .map(|(digest, value)| (digest, SecretEntry { value }))
                .collect();
            (Some(projected.yaml), keys)
        }
        Err(error) => {
            issues.push(OpenP2PIssue::error("invalid_secret", "proxies", format!("{error:#}")));
            (None, BTreeMap::new())
        }
    };
    if has_errors(&issues) {
        return ParsedImport {
            config,
            issues,
            has_mihomo,
            profile: None,
            tokens,
            keys,
        };
    }
    ParsedImport {
        config,
        issues,
        has_mihomo,
        profile,
        tokens,
        keys,
    }
}

impl ParsedImport {
    fn failed(code: &str, path: impl Into<std::string::String>, message: impl Into<std::string::String>) -> Self {
        Self::failed_with(vec![OpenP2PIssue::error(code, path, message)], false)
    }

    fn failed_with(issues: Vec<OpenP2PIssue>, has_mihomo: bool) -> Self {
        Self {
            config: OpenP2PConfig::default(),
            issues,
            has_mihomo,
            profile: None,
            tokens: BTreeMap::new(),
            keys: BTreeMap::new(),
        }
    }
}

fn merge_device_fields(seed: &mut Mapping, existing: &OpenP2PConfig) {
    let Some(Value::Sequence(accounts)) = seed.get_mut("accounts") else {
        return;
    };
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
        if !account.contains_key("interface")
            && let Some(interface) = old.interface.as_ref()
        {
            account.insert(Value::from("interface"), Value::from(interface.as_str()));
        }
        if !account.contains_key("enabled") {
            account.insert(Value::from("enabled"), Value::Bool(old.enabled));
        }
        let Some(Value::Sequence(apps)) = account.get_mut("apps") else {
            continue;
        };
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
            if !app.contains_key("enabled") {
                app.insert(Value::from("enabled"), Value::Bool(old_app.enabled.is_enabled()));
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

#[cfg(test)]
#[path = "portable_tests.rs"]
mod tests;
