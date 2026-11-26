//! Projects the canonical document onto the runtime mihomo profile.
//!
//! The projection is the top-level document minus `x-openp2p`, with every inline
//! SSH private key staged below the subscription directory and rewritten to a
//! relative path under the app home. The generated profile therefore never
//! contains a PEM body, and the manifest written by the store owns the files.

use std::collections::BTreeMap;

use anyhow::{Result, bail};
use serde_yaml_ng::{Mapping, Value};

use super::proxies;
use super::schema::{AppProtocol, DOCUMENT_KEY, OpenP2PConfig};
use super::secrets::{key_digest, validate_private_key};

pub struct ProjectedProfile {
    /// The projected mihomo document without a header.
    pub yaml: std::string::String,
    /// SHA-256 digest to inline key body.
    pub keys: BTreeMap<std::string::String, std::string::String>,
}

/// The path mihomo resolves against the app home and the runtime bundle copies.
pub fn relative_key_path(subscription_id: &str, digest: &str) -> std::string::String {
    format!("openp2p/subscriptions/{subscription_id}/keys/{digest}.key")
}

pub fn project(document: &Mapping, subscription_id: &str, config: &OpenP2PConfig) -> Result<ProjectedProfile> {
    if !super::schema::valid_slug(subscription_id, 32) {
        bail!("invalid openp2p subscription id");
    }
    let mut projection = document.clone();
    projection.remove(DOCUMENT_KEY);
    projection.remove("x-openp2p-secrets");
    let mut keys = BTreeMap::new();

    if let Some(Value::Sequence(proxies)) = projection.get_mut("proxies") {
        for (index, proxy) in proxies.iter_mut().enumerate() {
            stage_private_key(
                proxy,
                &format!("proxies[{index}].private-key"),
                subscription_id,
                &mut keys,
            )?;
            normalize_managed_host_key(proxy, config);
        }
    }
    if let Some(Value::Mapping(providers)) = projection.get_mut("proxy-providers") {
        for (name, provider) in providers.iter_mut() {
            let Some(payload) = provider.get_mut("payload") else {
                continue;
            };
            let Some(payload) = payload.as_sequence_mut() else {
                continue;
            };
            let provider = name.as_str().unwrap_or_default();
            for (index, proxy) in payload.iter_mut().enumerate() {
                stage_private_key(
                    proxy,
                    &format!("proxy-providers.{provider}.payload[{index}].private-key"),
                    subscription_id,
                    &mut keys,
                )?;
            }
        }
    }

    let yaml = serde_yaml_ng::to_string(&Value::Mapping(projection))?;
    if yaml.contains("PRIVATE KEY") {
        bail!("the projected mihomo profile still contains a private key body");
    }
    Ok(ProjectedProfile { yaml, keys })
}

/// mihomo decodes `host-key` as a string list while the canonical document
/// allows one authorized_keys line. Only managed loopback nodes are rewritten.
fn normalize_managed_host_key(proxy: &mut Value, config: &OpenP2PConfig) {
    let Some(proxy) = proxy.as_mapping_mut() else {
        return;
    };
    if proxy.get("type").and_then(Value::as_str) != Some("ssh") {
        return;
    }
    let Some(server) = proxy.get("server").and_then(Value::as_str) else {
        return;
    };
    if !proxies::is_loopback_server(server) {
        return;
    }
    #[allow(clippy::cast_possible_truncation)]
    let Some(port) = proxy
        .get("port")
        .and_then(Value::as_u64)
        .filter(|port| (1..=u16::MAX as u64).contains(port))
        .map(|port| port as u16)
    else {
        return;
    };
    let managed = config.accounts.iter().filter(|account| account.enabled).any(|account| {
        account
            .apps
            .iter()
            .any(|app| app.enabled.is_enabled() && app.protocol() == AppProtocol::Tcp && app.src_port == port)
    });
    if !managed {
        return;
    }
    let Some(Value::String(host_key)) = proxy.get("host-key") else {
        return;
    };
    let host_key = host_key.clone();
    proxy.insert(Value::from("host-key"), Value::Sequence(vec![Value::String(host_key)]));
}

fn stage_private_key(
    proxy: &mut Value,
    path: &str,
    subscription_id: &str,
    keys: &mut BTreeMap<std::string::String, std::string::String>,
) -> Result<()> {
    let Some(proxy) = proxy.as_mapping_mut() else {
        return Ok(());
    };
    let Some(value) = proxy.get_mut("private-key") else {
        return Ok(());
    };
    let Some(raw) = value.as_str() else {
        return Ok(());
    };
    if !raw.contains("PRIVATE KEY") {
        return Ok(());
    }
    validate_private_key(raw).map_err(|error| anyhow::anyhow!("{path}: {error:#}"))?;
    let digest = key_digest(raw);
    keys.insert(digest.clone(), raw.to_owned());
    *value = Value::String(relative_key_path(subscription_id, &digest));
    Ok(())
}
