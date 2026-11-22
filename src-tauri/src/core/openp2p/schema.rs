use std::net::IpAddr;

use serde::{Deserialize, Serialize};
use serde_yaml_ng::Value;

pub const GROUP_NAME: &str = "OP2P";
pub const RULE_TARGET: &str = GROUP_NAME;

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
    /// Dotted path inside x-openp2p (never a value).
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
}

pub fn has_errors(issues: &[OpenP2PIssue]) -> bool {
    issues.iter().any(|issue| issue.severity == IssueSeverity::Error)
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
    pub token_ref: std::string::String,
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
            token_ref: std::string::String::new(),
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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProxySpec {
    pub name: std::string::String,
    pub account: std::string::String,
    /// Which forwarded app this SSH node reaches. Optional when the account has one enabled TCP app.
    pub app: Option<std::string::String>,
    pub ssh_user: std::string::String,
    pub private_key_ref: std::string::String,
    pub host_key: Option<std::string::String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HealthSpec {
    pub host: std::string::String,
    pub port: Option<u16>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NetworkSpec {
    pub name: Option<std::string::String>,
    pub domains: Vec<std::string::String>,
    pub cidrs: Vec<std::string::String>,
    pub dns: Vec<std::string::String>,
    #[serde(default, deserialize_with = "deserialize_health")]
    pub health: Option<HealthSpec>,
}

fn deserialize_health<'de, D>(deserializer: D) -> std::result::Result<Option<HealthSpec>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize as _;
    let value = Option::<Value>::deserialize(deserializer)?;
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(host)) => Ok(Some(HealthSpec { host, port: None })),
        Some(Value::Mapping(mapping)) => serde_yaml_ng::from_value::<HealthSpec>(Value::Mapping(mapping))
            .map(Some)
            .map_err(serde::de::Error::custom),
        Some(_) => Err(serde::de::Error::custom(
            "health must be a hostname string or a {host, port} mapping",
        )),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OpenP2PConfig {
    pub version: u32,
    pub enabled: bool,
    pub accounts: Vec<AccountSpec>,
    pub proxies: Vec<ProxySpec>,
    pub networks: Vec<NetworkSpec>,
}

impl Default for OpenP2PConfig {
    fn default() -> Self {
        Self {
            version: 1,
            enabled: true,
            accounts: Vec::new(),
            proxies: Vec::new(),
            networks: Vec::new(),
        }
    }
}

impl OpenP2PConfig {
    pub fn account(&self, id: &str) -> Option<&AccountSpec> {
        self.accounts.iter().find(|account| account.id == id)
    }

    pub fn proxies_for<'a>(&'a self, account_id: &'a str) -> impl Iterator<Item = &'a ProxySpec> + 'a {
        self.proxies.iter().filter(move |proxy| proxy.account == account_id)
    }

    pub const fn is_empty(&self) -> bool {
        self.accounts.is_empty() && self.proxies.is_empty() && self.networks.is_empty()
    }
}

pub struct ParsedSeed {
    pub config: OpenP2PConfig,
    pub issues: Vec<OpenP2PIssue>,
}

/// Extracts and validates x-openp2p. A bare document with a known top-level key is also accepted.
pub fn parse_seed(yaml: &str) -> ParsedSeed {
    if yaml.len() > 1024 * 1024 {
        return ParsedSeed {
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
            return ParsedSeed {
                config: OpenP2PConfig::default(),
                issues: vec![OpenP2PIssue::error("yaml_syntax", "", error.to_string())],
            };
        }
    };
    let Some(document) = value.as_mapping() else {
        return ParsedSeed {
            config: OpenP2PConfig::default(),
            issues: vec![OpenP2PIssue::error(
                "yaml_syntax",
                "",
                "the document must be a YAML mapping",
            )],
        };
    };

    let from_profile = document.contains_key("x-openp2p");
    let seed = if from_profile {
        document.get("x-openp2p").cloned().unwrap_or(Value::Null)
    } else if document.contains_key("accounts") || document.contains_key("proxies") || document.contains_key("networks")
    {
        Value::Mapping(document.clone())
    } else {
        return ParsedSeed {
            config: OpenP2PConfig::default(),
            issues: vec![OpenP2PIssue::error(
                "x_openp2p_missing",
                "",
                "the document has no x-openp2p block",
            )],
        };
    };

    let mut issues = reject_secret_fields(&seed, "x-openp2p");
    let mut config = match serde_yaml_ng::from_value::<OpenP2PConfig>(seed) {
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
    issues.extend(normalize_and_validate(&mut config));
    ParsedSeed { config, issues }
}

/// Rejects inline secrets. Only token_ref / private_key_ref may appear.
fn reject_secret_fields(value: &Value, path: &str) -> Vec<OpenP2PIssue> {
    let mut issues = Vec::new();
    reject_secret_fields_inner(value, path, &mut issues);
    issues
}

fn reject_secret_fields_inner(value: &Value, path: &str, issues: &mut Vec<OpenP2PIssue>) {
    match value {
        Value::Mapping(mapping) => {
            for (key, child) in mapping {
                let Some(key) = key.as_str() else {
                    continue;
                };
                let lower = key.to_ascii_lowercase();
                let child_path = format!("{path}.{key}");
                match lower.as_str() {
                    "token" => issues.push(OpenP2PIssue::error(
                        "token_in_config",
                        &child_path,
                        "x-openp2p must not contain token values; use token_ref",
                    )),
                    "private_key" | "private-key" | "privatekey" | "password" | "passphrase" => {
                        issues.push(OpenP2PIssue::error(
                            "key_in_config",
                            &child_path,
                            "x-openp2p must not contain key material; import it with save_openp2p_key",
                        ));
                    }
                    _ => {}
                }
                reject_secret_fields_inner(child, &child_path, issues);
            }
        }
        Value::Sequence(sequence) => {
            for (index, child) in sequence.iter().enumerate() {
                reject_secret_fields_inner(child, &format!("{path}[{index}]"), issues);
            }
        }
        Value::String(text) if text.contains("PRIVATE KEY") => issues.push(OpenP2PIssue::error(
            "key_in_config",
            path,
            "x-openp2p must not contain key material",
        )),
        _ => {}
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

pub fn dns_policy_key(domain: &str) -> std::string::String {
    format!("+.{domain}")
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

pub fn valid_ref(value: &str) -> bool {
    if value.is_empty() || value.len() > 128 || value.starts_with('/') || value.ends_with('/') {
        return false;
    }
    if value.contains("..") || value.contains("//") {
        return false;
    }
    value.split('/').all(|segment| {
        !segment.is_empty()
            && segment
                .chars()
                .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' || ch == '-')
    }) && value
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit())
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

fn valid_host(value: &str) -> bool {
    let value = value.trim();
    (!value.is_empty()
        && value.len() <= 253
        && !value.contains(char::is_whitespace)
        && !value.contains('/')
        && !value.contains(':')
        && value.parse::<IpAddr>().is_ok())
        || normalize_domain(value).is_some()
}

fn valid_interface(value: &str) -> bool {
    if value == "auto" {
        return true;
    }
    (1..=32).contains(&value.len())
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '.' || ch == ':' || ch == '-')
}

fn valid_whitelist(value: &str) -> bool {
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
#[path = "schema_validate.rs"]
mod validate;
pub use validate::normalize_and_validate;

#[cfg(test)]
#[path = "schema_fixtures.rs"]
pub(crate) mod fixtures;

#[cfg(test)]
#[path = "schema_tests.rs"]
mod tests;
