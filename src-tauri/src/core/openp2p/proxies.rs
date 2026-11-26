//! Validation of the mihomo SSH proxies that OpenP2P manages.
//!
//! A top-level `type: ssh` proxy whose server is loopback and whose port equals
//! an enabled TCP app's `src_port` is managed: OpenP2P launches the sidecar
//! listener on that loopback port, so the node must carry a username, a host key
//! and an inline private key. Every other proxy is ordinary mihomo config and is
//! only staged by the profile projection.

use std::net::IpAddr;

use serde_yaml_ng::{Mapping, Value};

use super::schema::{AccountSpec, AppProtocol, AppSpec, OpenP2PConfig, OpenP2PIssue, valid_authorized_key};
use super::secrets::validate_private_key;

pub fn validate_proxies(document: &Mapping, config: &OpenP2PConfig) -> Vec<OpenP2PIssue> {
    let mut issues = Vec::new();
    let Some(Value::Sequence(proxies)) = document.get("proxies") else {
        return issues;
    };
    for (index, value) in proxies.iter().enumerate() {
        let Some(proxy) = value.as_mapping() else {
            continue;
        };
        if proxy.get("type").and_then(Value::as_str) != Some("ssh") {
            continue;
        }
        let Some(server) = proxy.get("server").and_then(Value::as_str) else {
            continue;
        };
        if !is_loopback_server(server) {
            continue;
        }
        let Some(port) = proxy_port(proxy) else {
            continue;
        };
        let matching = matching_apps(config, port);
        let base = format!("proxies[{index}]");
        match matching.len() {
            0 => issues.push(OpenP2PIssue::warning(
                "proxy_app_unmatched",
                format!("{base}.port"),
                format!(
                    "loopback SSH proxy port {port} matches no enabled TCP app; it is kept as an ordinary mihomo proxy"
                ),
            )),
            1 => validate_managed(proxy, &base, matching[0], &mut issues),
            _ => issues.push(OpenP2PIssue::error(
                "duplicate_port",
                format!("{base}.port"),
                format!("port {port} matches more than one enabled TCP app"),
            )),
        }
    }
    issues
}

fn validate_managed(
    proxy: &Mapping,
    base: &str,
    (account, app): (&AccountSpec, &AppSpec),
    issues: &mut Vec<OpenP2PIssue>,
) {
    if !valid_username(proxy) {
        issues.push(OpenP2PIssue::error(
            "invalid_ssh_user",
            format!("{base}.username"),
            format!("a managed proxy for {}.{} requires a username", account.id, app.name),
        ));
    }
    match proxy.get("host-key") {
        Some(Value::String(host_key)) if valid_authorized_key(host_key) => {}
        Some(Value::Sequence(keys)) if keys.len() == 1 => {
            let valid = keys[0].as_str().is_some_and(valid_authorized_key);
            if !valid {
                issues.push(OpenP2PIssue::error(
                    "host_key_invalid",
                    format!("{base}.host-key"),
                    "host-key must be one authorized_keys line (type + base64 key)",
                ));
            }
        }
        Some(_) => issues.push(OpenP2PIssue::error(
            "host_key_invalid",
            format!("{base}.host-key"),
            "host-key must be one authorized_keys line (type + base64 key)",
        )),
        None => issues.push(OpenP2PIssue::error(
            "host_key_missing",
            format!("{base}.host-key"),
            format!("a managed proxy for {}.{} requires host-key", account.id, app.name),
        )),
    }
    match proxy.get("private-key").and_then(Value::as_str) {
        Some(private_key) if !private_key.trim().is_empty() => {
            if let Err(error) = validate_private_key(private_key) {
                issues.push(OpenP2PIssue::error(
                    "private_key_invalid",
                    format!("{base}.private-key"),
                    format!("{error:#}"),
                ));
            }
        }
        _ => issues.push(OpenP2PIssue::error(
            "private_key_missing",
            format!("{base}.private-key"),
            format!(
                "a managed proxy for {}.{} must carry an inline PEM private-key",
                account.id, app.name
            ),
        )),
    }
    for field in ["password", "private-key-passphrase"] {
        if proxy.contains_key(field) {
            issues.push(OpenP2PIssue::error(
                "proxy_password_not_allowed",
                format!("{base}.{field}"),
                "managed SSH proxies use an inline private key; passwords and passphrases are not allowed",
            ));
        }
    }
}

fn valid_username(proxy: &Mapping) -> bool {
    proxy
        .get("username")
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|username| {
            !username.is_empty()
                && username.len() <= 128
                && !username.chars().any(|ch| ch.is_whitespace() || ch.is_control())
        })
}

fn proxy_port(proxy: &Mapping) -> Option<u16> {
    #[allow(clippy::cast_possible_truncation)]
    proxy
        .get("port")
        .and_then(Value::as_u64)
        .filter(|port| (1..=u16::MAX as u64).contains(port))
        .map(|port| port as u16)
}

fn matching_apps(config: &OpenP2PConfig, port: u16) -> Vec<(&AccountSpec, &AppSpec)> {
    config
        .accounts
        .iter()
        .filter(|account| account.enabled)
        .flat_map(|account| {
            account
                .apps
                .iter()
                .filter(move |app| {
                    app.enabled.is_enabled() && app.protocol() == AppProtocol::Tcp && app.src_port == port
                })
                .map(move |app| (account, app))
        })
        .collect()
}

/// Loopback as mihomo would reach the sidecar listener, including the hostname
/// aliases macOS resolves locally.
pub fn is_loopback_server(server: &str) -> bool {
    let server = server.trim().trim_start_matches('[').trim_end_matches(']');
    if server.eq_ignore_ascii_case("localhost") {
        return true;
    }
    match server.parse::<IpAddr>() {
        Ok(IpAddr::V4(address)) => address.is_loopback(),
        Ok(IpAddr::V6(address)) => address.is_loopback(),
        Err(_) => false,
    }
}
