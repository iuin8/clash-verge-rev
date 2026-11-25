use anyhow::{Context as _, Result, bail};
use serde::Serialize;

use crate::core::openp2p::account::{self, token_path};
use crate::core::openp2p::schema::{AccountSpec, OpenP2PConfig};

/// The account token. No Debug/Display output ever exposes the value.
#[derive(Clone, Copy)]
pub struct TokenValue(u64);

impl TokenValue {
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

impl std::fmt::Debug for TokenValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TokenValue(***)")
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
struct RenderedNetwork {
    token: u64,
    node: std::string::String,
    user: std::string::String,
    share_bandwidth: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    interface: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dns: Option<Vec<std::string::String>>,
    server_host: std::string::String,
    server_ip: std::string::String,
    server_port: u16,
    public_ip_port: u16,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
struct RenderedApp {
    app_name: std::string::String,
    protocol: std::string::String,
    underlay_protocol: std::string::String,
    punch_priority: i32,
    whitelist: std::string::String,
    src_port: u16,
    peer_node: std::string::String,
    dst_port: u16,
    dst_host: std::string::String,
    peer_user: std::string::String,
    relay_node: std::string::String,
    force_relay: i32,
    enabled: i32,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
struct RenderedConfig {
    network: RenderedNetwork,
    apps: Vec<RenderedApp>,
}

pub fn read_token(token_ref: &str) -> Result<TokenValue> {
    let path = token_path(token_ref)?;
    read_token_at(&path)
}

fn read_token_at(path: &std::path::Path) -> Result<TokenValue> {
    account::ensure_private_file(path)?;
    let raw = std::fs::read_to_string(path).with_context(|| "openp2p token file is missing".to_string())?;
    parse_token(raw.trim())
}

pub fn parse_token(raw: &str) -> Result<TokenValue> {
    let token: u64 = raw
        .parse()
        .map_err(|_| anyhow::anyhow!("openp2p token must be a positive decimal integer"))?;
    if token == 0 {
        bail!("openp2p token must be a positive decimal integer");
    }
    Ok(TokenValue(token))
}

#[cfg(test)]
fn write_token_file(path: &std::path::Path, token: TokenValue) -> Result<()> {
    if let Some(parent) = path.parent() {
        account::create_private_dir(parent)?;
    }
    let mut bytes = token.as_u64().to_string().into_bytes();
    bytes.push(b'\n');
    account::atomic_write(path, &bytes, 0o600)
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

fn render_config(account: &AccountSpec, token: TokenValue) -> Result<RenderedConfig> {
    let gateway = account
        .gateway
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("account gateway is missing"))?;
    let apps = account
        .apps
        .iter()
        .map(|app| RenderedApp {
            app_name: app.name.clone(),
            protocol: app.protocol().as_str().into(),
            underlay_protocol: "quic".into(),
            punch_priority: 0,
            whitelist: app.whitelist().into(),
            src_port: app.src_port,
            peer_node: app.peer_node.clone(),
            dst_port: app.dst_port,
            dst_host: app.dst_host.clone(),
            peer_user: std::string::String::new(),
            relay_node: std::string::String::new(),
            force_relay: 0,
            enabled: i32::from(app.enabled.is_enabled()),
        })
        .collect();
    Ok(RenderedConfig {
        network: RenderedNetwork {
            token: token.as_u64(),
            node: account.node.clone(),
            user: std::string::String::new(),
            share_bandwidth: 10,
            interface: Some(account.interface().into()).filter(|value: &std::string::String| !value.is_empty()),
            dns: (!account.dns.is_empty()).then(|| account.dns.clone()),
            server_host: gateway.host.clone(),
            server_ip: std::string::String::new(),
            server_port: gateway.port,
            public_ip_port: account.public_ip_port,
        },
        apps,
    })
}

pub fn write_rendered_config(account: &AccountSpec, token: TokenValue) -> Result<()> {
    let paths = account::account_paths(&account.id)?;
    paths.ensure()?;
    let rendered = render_config(account, token)?;
    let bytes = serde_json::to_vec_pretty(&rendered)?;
    account::atomic_write(&paths.config_path, &bytes, 0o600)?;
    account::enforce_permissions(&paths);
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenReconcile {
    Match,
    /// config.json was missing or unreadable and was rendered again from the token file.
    Repaired,
    /// openp2p saved a different token; report it instead of adopting it.
    Mismatch,
}

/// Compares the live config.json with the token file after login. The token file stays
/// authoritative for every spawn; a server-side difference is only reported.
pub fn reconcile_after_login(account: &AccountSpec, token: TokenValue) -> Result<TokenReconcile> {
    let paths = account::account_paths(&account.id)?;
    account::ensure_private_file(&paths.config_path)?;
    let raw = std::fs::read_to_string(&paths.config_path).unwrap_or_default();
    let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap_or(serde_json::Value::Null);
    let stored = parsed
        .get("Network")
        .and_then(|network| network.get("Token"))
        .and_then(serde_json::Value::as_u64);
    match stored {
        Some(stored) if stored == token.as_u64() => Ok(TokenReconcile::Match),
        Some(_) => Ok(TokenReconcile::Mismatch),
        None => {
            write_rendered_config(account, token)?;
            Ok(TokenReconcile::Repaired)
        }
    }
}

pub fn account_secrets(config: &OpenP2PConfig, account: &AccountSpec) -> (bool, bool) {
    (
        account::token_present(&account.token_ref),
        account::account_key_present(config, &account.id),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::openp2p::schema::fixtures::sample_config;

    fn scratch(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("cvr-openp2p-config-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("scratch");
        path
    }

    #[test]
    fn rendered_config_uses_a_json_number_for_the_token_and_int_for_enabled() {
        let config = sample_config();
        let account = &config.accounts[0];
        let rendered = render_config(account, TokenValue(1234567890123)).expect("render");
        let value = serde_json::to_value(&rendered).expect("json");
        assert_eq!(value["Network"]["Token"], 1234567890123u64);
        assert_eq!(value["Network"]["Interface"], "auto");
        assert_eq!(value["Network"]["ServerPort"], 27183);
        assert_eq!(value["Apps"][0]["Enabled"], 1);
        assert_eq!(value["Apps"][0]["Whitelist"], "127.0.0.1");
    }

    #[test]
    fn a_token_file_is_written_0600_and_read_back() {
        let root = scratch("token");
        let path = root.join("tokens/o2p/home");
        write_token_file(&path, TokenValue(42)).expect("write");
        let read = read_token_at(&path).expect("read");
        assert_eq!(read.as_u64(), 42);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path).expect("metadata").permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
            let parent = std::fs::metadata(path.parent().expect("parent")).expect("metadata");
            assert_eq!(parent.permissions().mode() & 0o777, 0o700);
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn token_parsing_rejects_zero_and_garbage() {
        assert!(parse_token("0").is_err());
        assert!(parse_token("not-a-token").is_err());
        assert_eq!(parse_token("7").map(TokenValue::as_u64).expect("token"), 7);
    }

    #[test]
    fn private_keys_must_be_pem() {
        validate_private_key("garbage").expect_err("must reject");
        validate_private_key("-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n-----END OPENSSH PRIVATE KEY-----")
            .expect("pem accepted");
    }
}
