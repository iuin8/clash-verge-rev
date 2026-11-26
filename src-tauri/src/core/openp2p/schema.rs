use std::net::IpAddr;

use serde::{Deserialize, Serialize};
use serde_yaml_ng::{Mapping, Value};

/// The only marker that makes a document an OpenP2P document.
pub const DOCUMENT_KEY: &str = "x-openp2p";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum IssueSeverity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct OpenP2PIssue {
    pub code: std::string::String,
    pub severity: IssueSeverity,
    /// Dotted path inside the document (never a value).
    pub path: std::string::String,
    pub message: std::string::String,
}

impl OpenP2PIssue {
    pub fn error(code: &str, path: impl Into<std::string::String>, message: impl Into<std::string::String>) -> Self {
        Self {
            code: code.into(),
            severity: IssueSeverity::Error,
            path: path.into(),
            message: message.into(),
        }
    }

    pub fn warning(code: &str, path: impl Into<std::string::String>, message: impl Into<std::string::String>) -> Self {
        Self {
            code: code.into(),
            severity: IssueSeverity::Warning,
            path: path.into(),
            message: message.into(),
        }
    }

    pub const fn is_error(&self) -> bool {
        matches!(self.severity, IssueSeverity::Error)
    }
}

pub fn has_errors(issues: &[OpenP2PIssue]) -> bool {
    issues.iter().any(OpenP2PIssue::is_error)
}

/// A boolean that also accepts the 0/1 form openp2p stores in config.json.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Enabled {
    Bool(bool),
    Number(i64),
}

impl Enabled {
    pub const fn is_enabled(self) -> bool {
        match self {
            Self::Bool(value) => value,
            Self::Number(value) => value != 0,
        }
    }
}

impl Default for Enabled {
    fn default() -> Self {
        Self::Bool(true)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AppProtocol {
    Tcp,
    Udp,
}

impl AppProtocol {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Tcp => "tcp",
            Self::Udp => "udp",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GatewaySpec {
    pub host: std::string::String,
    pub port: u16,
}

impl Default for GatewaySpec {
    fn default() -> Self {
        Self {
            host: std::string::String::new(),
            port: 27183,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AppSpec {
    pub name: std::string::String,
    pub protocol: Option<AppProtocol>,
    pub src_port: u16,
    pub peer_node: std::string::String,
    pub dst_host: std::string::String,
    pub dst_port: u16,
    pub whitelist: Option<std::string::String>,
    pub enabled: Enabled,
}

impl Default for AppSpec {
    fn default() -> Self {
        Self {
            name: std::string::String::new(),
            protocol: None,
            src_port: 0,
            peer_node: std::string::String::new(),
            dst_host: "127.0.0.1".into(),
            dst_port: 22,
            whitelist: None,
            enabled: Enabled::default(),
        }
    }
}

impl AppSpec {
    pub fn protocol(&self) -> AppProtocol {
        self.protocol.unwrap_or(AppProtocol::Tcp)
    }

    pub fn whitelist(&self) -> &str {
        self.whitelist.as_deref().unwrap_or("127.0.0.1")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AccountSpec {
    pub id: std::string::String,
    pub label: Option<std::string::String>,
    pub enabled: bool,
    pub gateway: Option<GatewaySpec>,
    pub node: std::string::String,
    /// Inline during import; the stored config never carries it. Accepts the
    /// numeric and string spellings openp2p documents use.
    #[serde(
        default,
        deserialize_with = "deserialize_token",
        skip_serializing_if = "Option::is_none"
    )]
    pub token: Option<std::string::String>,
    pub public_ip_port: u16,
    pub interface: Option<std::string::String>,
    pub dns: Vec<std::string::String>,
    pub apps: Vec<AppSpec>,
}

impl Default for AccountSpec {
    fn default() -> Self {
        Self {
            id: std::string::String::new(),
            label: None,
            enabled: true,
            gateway: None,
            node: std::string::String::new(),
            token: None,
            public_ip_port: 0,
            interface: None,
            dns: Vec::new(),
            apps: Vec::new(),
        }
    }
}

impl AccountSpec {
    pub fn label(&self) -> &str {
        self.label.as_deref().unwrap_or(&self.id)
    }

    pub fn interface(&self) -> &str {
        self.interface.as_deref().unwrap_or("auto")
    }
}

fn deserialize_token<'de, D>(deserializer: D) -> std::result::Result<Option<std::string::String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<Value>::deserialize(deserializer)?;
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(token)) => Ok(Some(token)),
        Some(Value::Number(token)) => Ok(Some(token.to_string())),
        Some(_) => Err(serde::de::Error::custom("token must be a number or a string")),
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NetworkSpec {
    pub name: Option<std::string::String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<std::string::String>,
    pub domains: Vec<std::string::String>,
    pub cidrs: Vec<std::string::String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OpenP2PConfig {
    pub version: u32,
    /// Optional display name of the document.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<std::string::String>,
    pub accounts: Vec<AccountSpec>,
    pub networks: Vec<NetworkSpec>,
}

impl Default for OpenP2PConfig {
    fn default() -> Self {
        Self {
            version: 1,
            name: None,
            accounts: Vec::new(),
            networks: Vec::new(),
        }
    }
}

impl OpenP2PConfig {
    pub fn account(&self, id: &str) -> Option<&AccountSpec> {
        self.accounts.iter().find(|account| account.id == id)
    }
}

/// Wraps the normalized config as a re-parseable OpenP2P document. The store
/// keeps this wrapper so `load_config` exercises the same canonical shape as an
/// import, without the mihomo part or any credential value.
pub fn store_document(config: &OpenP2PConfig) -> std::result::Result<std::string::String, serde_yaml_ng::Error> {
    let mut document = Mapping::new();
    document.insert(Value::from(DOCUMENT_KEY), serde_yaml_ng::to_value(config)?);
    serde_yaml_ng::to_string(&Value::Mapping(document))
}

pub struct ParsedStore {
    pub config: OpenP2PConfig,
    pub issues: Vec<OpenP2PIssue>,
}

/// Store parse path: accepts only the `x-openp2p` wrapper and never performs
/// import-time generation or privilege checks.
pub fn parse_store(yaml: &str) -> ParsedStore {
    if yaml.len() > 1024 * 1024 {
        return ParsedStore {
            config: OpenP2PConfig::default(),
            issues: vec![OpenP2PIssue::error(
                "config_too_large",
                "",
                "the configuration exceeds 1 MiB",
            )],
        };
    }
    let value = match serde_yaml_ng::from_str::<Value>(yaml) {
        Ok(value) => value,
        Err(error) => {
            return ParsedStore {
                config: OpenP2PConfig::default(),
                issues: vec![OpenP2PIssue::error("yaml_syntax", "", error.to_string())],
            };
        }
    };
    let Some(document) = value.as_mapping() else {
        return ParsedStore {
            config: OpenP2PConfig::default(),
            issues: vec![OpenP2PIssue::error(
                "yaml_syntax",
                "",
                "the document must be a YAML mapping",
            )],
        };
    };
    let Some(block) = document.get(DOCUMENT_KEY) else {
        return ParsedStore {
            config: OpenP2PConfig::default(),
            issues: vec![OpenP2PIssue::error(
                "x_openp2p_missing",
                "",
                "the document has no x-openp2p block",
            )],
        };
    };
    let Some(block) = block.as_mapping() else {
        return ParsedStore {
            config: OpenP2PConfig::default(),
            issues: vec![OpenP2PIssue::error(
                "schema_invalid",
                DOCUMENT_KEY,
                "x-openp2p must be a mapping",
            )],
        };
    };
    let issues = legacy_document_issues(document, block);
    if has_errors(&issues) {
        return ParsedStore {
            config: OpenP2PConfig::default(),
            issues,
        };
    }
    let mut config = match serde_yaml_ng::from_value::<OpenP2PConfig>(Value::Mapping(block.clone())) {
        Ok(config) => config,
        Err(error) => {
            return ParsedStore {
                config: OpenP2PConfig::default(),
                issues: vec![schema_issue(&error)],
            };
        }
    };
    let mut issues = Vec::new();
    for (index, account) in config.accounts.iter().enumerate() {
        if account.token.is_some() {
            issues.push(OpenP2PIssue::error(
                "invalid_secret",
                format!("accounts[{index}].token"),
                "the stored config must not carry a token",
            ));
        }
    }
    issues.extend(crate::core::openp2p::schema_validate::normalize_and_validate(
        &mut config,
    ));
    ParsedStore { config, issues }
}

pub fn schema_issue(error: &serde_yaml_ng::Error) -> OpenP2PIssue {
    let message = error.to_string();
    let code = if message.contains("unknown field") {
        "unknown_field"
    } else {
        "schema_invalid"
    };
    OpenP2PIssue::error(code, DOCUMENT_KEY, message)
}

/// Explicit failures for the shapes the canonical document replaced.
pub fn legacy_document_issues(document: &Mapping, block: &Mapping) -> Vec<OpenP2PIssue> {
    let mut issues = Vec::new();
    if document.contains_key("x-openp2p-secrets") {
        issues.push(OpenP2PIssue::error(
            "secrets_block_removed",
            "x-openp2p-secrets",
            "x-openp2p-secrets is no longer supported; put account tokens inline under accounts[] and SSH keys inline in the mihomo proxies",
        ));
    }
    for section in ["proxies", "proxy-groups", "proxy-providers", "rules", "dns"] {
        if block.contains_key(section) {
            issues.push(OpenP2PIssue::error(
                "mihomo_section_at_top_level",
                format!("{DOCUMENT_KEY}.{section}"),
                format!("{section} is ordinary mihomo config and must be a top-level key, not part of x-openp2p"),
            ));
        }
    }
    scan_legacy_keys(&Value::Mapping(block.clone()), DOCUMENT_KEY, &mut issues);
    scan_legacy_proxies(document.get("proxies"), "proxies", &mut issues);
    if let Some(Value::Mapping(providers)) = document.get("proxy-providers") {
        for (name, provider) in providers {
            let Some(payload) = provider.get("payload") else {
                continue;
            };
            let path = format!("proxy-providers.{}.payload", name.as_str().unwrap_or_default());
            scan_legacy_proxies(Some(payload), &path, &mut issues);
        }
    }
    issues
}

fn scan_legacy_keys(value: &Value, path: &str, issues: &mut Vec<OpenP2PIssue>) {
    match value {
        Value::Mapping(mapping) => {
            for (key, child) in mapping {
                let Some(key) = key.as_str() else {
                    continue;
                };
                let child_path = format!("{path}.{key}");
                if key == "token_ref" || key == "private_key_ref" {
                    issues.push(OpenP2PIssue::error(
                        "legacy_reference",
                        &child_path,
                        "token_ref/private_key_ref were removed; account tokens are inline and SSH keys are inline private-key values",
                    ));
                }
                scan_legacy_keys(child, &child_path, issues);
            }
        }
        Value::Sequence(sequence) => {
            for (index, child) in sequence.iter().enumerate() {
                scan_legacy_keys(child, &format!("{path}[{index}]"), issues);
            }
        }
        _ => {}
    }
}

fn scan_legacy_proxies(value: Option<&Value>, path: &str, issues: &mut Vec<OpenP2PIssue>) {
    let Some(Value::Sequence(proxies)) = value else {
        return;
    };
    for (index, proxy) in proxies.iter().enumerate() {
        let Some(proxy) = proxy.as_mapping() else {
            continue;
        };
        let legacy: Vec<&str> = ["account", "app", "ssh_user", "private_key", "private_key_ref"]
            .into_iter()
            .filter(|field| proxy.contains_key(*field))
            .collect();
        if legacy.is_empty() {
            continue;
        }
        issues.push(OpenP2PIssue::error(
            "legacy_reference",
            format!("{path}[{index}]"),
            format!(
                "custom OpenP2P proxy fields ({}) were removed; use a standard mihomo type: ssh proxy",
                legacy.join(", ")
            ),
        ));
    }
}

pub fn normalize_domain(raw: &str) -> Option<std::string::String> {
    let domain = raw.trim().trim_start_matches("+.").trim_start_matches('.');
    if domain.is_empty() || domain.len() > 253 || domain.contains(char::is_whitespace) {
        return None;
    }
    if domain.split('.').any(|label| {
        label.is_empty()
            || label.len() > 63
            || !label
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
            || label.starts_with('-')
            || label.ends_with('-')
    }) {
        return None;
    }
    Some(domain.to_ascii_lowercase())
}

pub fn normalize_dns_server(raw: &str) -> Option<std::string::String> {
    let raw = raw.trim();
    let (host, port) = split_host_port(raw)?;
    host.parse::<IpAddr>().ok()?;
    let port = port.unwrap_or(53);
    if port == 0 {
        return None;
    }
    Some(if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    })
}

pub fn normalize_cidr(raw: &str) -> Option<std::string::String> {
    let (address, prefix) = raw.trim().split_once('/')?;
    let address: IpAddr = address.parse().ok()?;
    let prefix: u8 = prefix.parse().ok()?;
    let max = match address {
        IpAddr::V4(_) => 32,
        IpAddr::V6(_) => 128,
    };
    if prefix > max {
        return None;
    }
    Some(format!("{address}/{prefix}"))
}

pub fn valid_authorized_key(raw: &str) -> bool {
    if raw.contains('\n') || raw.contains('\r') {
        return false;
    }
    let mut parts = raw.split_whitespace();
    let (Some(kind), Some(blob)) = (parts.next(), parts.next()) else {
        return false;
    };
    if !(kind.starts_with("ssh-") || kind.starts_with("ecdsa-") || kind.starts_with("sk-")) {
        return false;
    }
    if kind.len() > 64
        || !kind
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '@' || ch == '.')
    {
        return false;
    }
    use base64::Engine as _;
    let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(blob) else {
        return false;
    };
    if decoded.len() < 4 + kind.len() {
        return false;
    }
    let embedded_len = u32::from_be_bytes([decoded[0], decoded[1], decoded[2], decoded[3]]) as usize;
    decoded.len() >= 4 + embedded_len && decoded[4..4 + embedded_len] == *kind.as_bytes()
}

pub fn valid_slug(value: &str, max_len: usize) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    value.len() <= max_len
        && (first.is_ascii_lowercase() || first.is_ascii_digit())
        && chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' || ch == '-' || ch == '.')
}

pub fn valid_node_name(value: &str) -> bool {
    (8..=31).contains(&value.len())
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '.' || ch == '_' || ch == '-')
}

pub fn valid_host(value: &str) -> bool {
    let value = value.trim();
    (!value.is_empty()
        && value.len() <= 253
        && !value.contains(char::is_whitespace)
        && !value.contains('/')
        && !value.contains(':')
        && value.parse::<IpAddr>().is_ok())
        || normalize_domain(value).is_some()
}

pub fn valid_interface(value: &str) -> bool {
    if value == "auto" {
        return true;
    }
    (1..=32).contains(&value.len())
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '.' || ch == ':' || ch == '-')
}

pub fn valid_whitelist(value: &str) -> bool {
    let value = value.trim();
    if value.is_empty() {
        return false;
    }
    value.split(',').all(|entry| {
        let entry = entry.trim();
        if entry.is_empty() {
            return false;
        }
        entry.parse::<IpAddr>().is_ok() || normalize_cidr(entry).is_some()
    })
}

fn split_host_port(raw: &str) -> Option<(&str, Option<u16>)> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    if let Some(rest) = raw.strip_prefix('[') {
        let (host, tail) = rest.split_once(']')?;
        let port = match tail {
            "" => None,
            _ => Some(tail.strip_prefix(':')?.parse().ok()?),
        };
        return Some((host, port));
    }
    match raw.rsplit_once(':') {
        // A bare IPv6 address has more than one colon and no port suffix.
        Some((host, port)) if !host.contains(':') => Some((host, Some(port.parse().ok()?))),
        Some(_) => None,
        None => Some((raw, None)),
    }
}

#[cfg(test)]
#[path = "schema_fixtures.rs"]
pub(crate) mod fixtures;

#[cfg(test)]
#[path = "schema_tests.rs"]
mod tests;
