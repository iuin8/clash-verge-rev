//! Pure OpenP2P → mihomo profile projection.
//!
//! One subscription renders one mihomo profile: the subscription's own mihomo
//! section plus the SSH nodes, fallback groups, route rules and DNS policies of
//! every account in the subscription. The renderer never writes credentials;
//! generated SSH nodes reference the staged key below `openp2p/keys/`.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, bail};
use serde_yaml_ng::{Mapping, Value};

use crate::core::openp2p::schema::{OpenP2PConfig, OpenP2PIssue};

#[path = "profile_render_build.rs"]
mod build;

#[derive(Debug)]
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
}

pub(crate) struct RenderedProfile {
    /// Ready-to-write YAML, header included.
    pub yaml: std::string::String,
    pub warnings: Vec<OpenP2PIssue>,
}

/// Merges one subscription into a single mihomo profile.
///
/// `mihomo_base` is the non-OpenP2P part of the imported document, if any.
/// Generated proxies and groups replace same-named base entries so the
/// subscription's own tunnel wins; generated rules are prepended before the
/// base rules and no MATCH rule is ever added.
pub(crate) fn render_subscription_profile_with<T, K>(
    config: &OpenP2PConfig,
    mihomo_base: Option<&str>,
    header: &str,
    token_ok: T,
    key_ok: K,
) -> Result<RenderedProfile>
where
    T: Fn(&str) -> bool,
    K: Fn(&str) -> bool,
{
    let mut profile = match mihomo_base {
        Some(text) if !text.trim().is_empty() => match serde_yaml_ng::from_str::<Value>(text)? {
            Value::Mapping(mapping) => mapping,
            _ => bail!("the mihomo section of the imported document must be a YAML mapping"),
        },
        _ => Mapping::new(),
    };
    profile.remove("x-openp2p");
    profile.remove("x-openp2p-secrets");

    let mut proxies = Vec::new();
    let mut groups = Vec::new();
    let mut rules = Vec::new();
    let mut warnings = Vec::new();
    let mut dns: BTreeMap<std::string::String, Vec<Value>> = BTreeMap::new();
    for account in &config.accounts {
        if !account.enabled {
            continue;
        }
        let set = build::build_artifacts_for_account(config, &account.id, true, &token_ok, &key_ok);
        warnings.extend(set.warnings);
        proxies.extend(set.proxies);
        groups.extend(set.group);
        rules.extend(set.rules);
        for (domain, servers) in set.dns {
            let entry = dns.entry(domain).or_default();
            if let Value::Sequence(servers) = servers {
                for server in servers {
                    if !entry.contains(&server) {
                        entry.push(server);
                    }
                }
            }
        }
    }

    merge_named_section(&mut profile, "proxies", proxies)?;
    merge_named_section(&mut profile, "proxy-groups", groups)?;
    let base_rules = take_sequence(&mut profile, "rules")?;
    let mut merged_rules = rules;
    merged_rules.extend(base_rules);
    profile.insert(Value::from("rules"), Value::Sequence(merged_rules));
    merge_dns(&mut profile, dns)?;

    if contains_private_key_body(&Value::Mapping(profile.clone())) {
        bail!(
            "the mihomo section embeds a private key; move the key to an ssh private-key file reference before importing"
        );
    }

    let body = serde_yaml_ng::to_string(&Value::Mapping(profile))?;
    Ok(RenderedProfile {
        yaml: format!("{header}{body}"),
        warnings,
    })
}

fn merge_named_section(profile: &mut Mapping, key: &str, generated: Vec<Value>) -> Result<()> {
    let base = take_sequence(profile, key)?;
    let generated_names: BTreeSet<&str> = generated
        .iter()
        .filter_map(|item| item.get("name").and_then(Value::as_str))
        .collect();
    let mut merged: Vec<Value> = base
        .into_iter()
        .filter(|item| {
            item.get("name")
                .and_then(Value::as_str)
                .is_none_or(|name| !generated_names.contains(name))
        })
        .collect();
    merged.extend(generated);
    profile.insert(Value::from(key), Value::Sequence(merged));
    Ok(())
}

fn take_sequence(profile: &mut Mapping, key: &str) -> Result<Vec<Value>> {
    match profile.remove(key) {
        None => Ok(Vec::new()),
        Some(Value::Sequence(items)) => Ok(items),
        Some(_) => bail!("{key} in the mihomo section must be a list"),
    }
}

fn merge_dns(profile: &mut Mapping, generated: BTreeMap<std::string::String, Vec<Value>>) -> Result<()> {
    if generated.is_empty() && !profile.contains_key("dns") {
        return Ok(());
    }
    let mut dns = match profile.remove("dns") {
        None => Mapping::new(),
        Some(Value::Mapping(mapping)) => mapping,
        Some(_) => bail!("dns in the mihomo section must be a mapping"),
    };
    if !generated.is_empty() {
        let mut policy = match dns.remove("nameserver-policy") {
            None => Mapping::new(),
            Some(Value::Mapping(mapping)) => mapping,
            Some(_) => bail!("dns.nameserver-policy in the mihomo section must be a mapping"),
        };
        for (domain, servers) in generated {
            match policy.get_mut(domain.as_str()) {
                Some(Value::Sequence(existing)) => {
                    for server in servers {
                        if !existing.contains(&server) {
                            existing.push(server);
                        }
                    }
                }
                Some(_) => {
                    policy.insert(Value::from(domain.as_str()), Value::Sequence(servers));
                }
                None => {
                    policy.insert(Value::from(domain.as_str()), Value::Sequence(servers));
                }
            }
        }
        dns.insert(Value::from("nameserver-policy"), Value::Mapping(policy));
    }
    profile.insert(Value::from("dns"), Value::Mapping(dns));
    Ok(())
}

fn contains_private_key_body(value: &Value) -> bool {
    match value {
        Value::String(text) => text.contains("PRIVATE KEY"),
        Value::Sequence(items) => items.iter().any(contains_private_key_body),
        Value::Mapping(mapping) => mapping
            .iter()
            .any(|(key, value)| contains_private_key_body(key) || contains_private_key_body(value)),
        _ => false,
    }
}
