use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use serde_yaml_ng::{Mapping, Sequence, Value};

use super::{ArtifactSet, ChainPlan, ProfileArtifacts};
use crate::core::openp2p::account;
use crate::core::openp2p::schema::{GROUP_NAME, OpenP2PConfig};
use crate::enhance::seq::SeqMap;

#[derive(Default)]
pub(super) struct FileSnapshot {
    entries: Vec<(PathBuf, Option<Vec<u8>>)>,
}

impl FileSnapshot {
    pub(super) fn capture(paths: &[PathBuf]) -> Self {
        let mut entries = Vec::new();
        for path in paths {
            if entries.iter().any(|(known, _)| known == path) {
                continue;
            }
            entries.push((path.clone(), std::fs::read(path).ok()));
        }
        Self { entries }
    }

    pub(super) fn restore(&self) -> Result<()> {
        let mut failures = Vec::new();
        for (path, previous) in &self.entries {
            let result = match previous {
                Some(bytes) => std::fs::write(path, bytes),
                None => match std::fs::remove_file(path) {
                    Ok(()) => Ok(()),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Err(error) => Err(error),
                },
            };
            if let Err(error) = result {
                failures.push(format!("{}: {error}", path.display()));
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            bail!("failed to roll back openp2p artifacts: {}", failures.join("; "))
        }
    }
}

pub(super) fn read_seq(path: &Path) -> Result<SeqMap> {
    if !path.exists() {
        return Ok(SeqMap::default());
    }
    let raw = std::fs::read_to_string(path).with_context(|| format!("failed to read chain item {}", path.display()))?;
    if raw.trim().is_empty() {
        return Ok(SeqMap::default());
    }
    let value: Value =
        serde_yaml_ng::from_str(&raw).with_context(|| format!("failed to parse chain item {}", path.display()))?;
    if value.is_null() {
        return Ok(SeqMap::default());
    }
    serde_yaml_ng::from_value(value).with_context(|| format!("failed to parse chain item {}", path.display()))
}

pub(super) fn read_mapping_sync(path: &Path) -> Result<Mapping> {
    let raw = std::fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    if raw.trim().is_empty() {
        return Ok(Mapping::new());
    }
    match serde_yaml_ng::from_str::<Value>(&raw) {
        Ok(Value::Null) => Ok(Mapping::new()),
        Ok(mut value) => {
            value
                .apply_merge()
                .with_context(|| format!("failed to apply merge in {}", path.display()))?;
            match value {
                Value::Mapping(mapping) => Ok(mapping),
                _ => bail!("{} is not a YAML mapping", path.display()),
            }
        }
        Err(error) => Err(anyhow::Error::new(error).context(format!("failed to parse {}", path.display()))),
    }
}

fn save_mapping_sync(path: &Path, mapping: &Mapping) -> Result<()> {
    let raw = serde_yaml_ng::to_string(mapping)?;
    account::atomic_write_file(path, raw.as_bytes())
}

/// Index mappings by Value so lookups never depend on string-key borrowing details.
pub(super) fn get_key<'a>(mapping: &'a Mapping, key: &str) -> Option<&'a Value> {
    mapping.get(Value::from(key))
}

fn remove_key(mapping: &mut Mapping, key: &str) -> Option<Value> {
    mapping.remove(Value::from(key))
}

pub(super) fn save_seq_sync(path: &Path, seq: &SeqMap) -> Result<()> {
    let raw = serde_yaml_ng::to_string(seq)?;
    account::atomic_write_file(path, raw.as_bytes())
}

fn remove_named(sequence: &mut Sequence, names: &BTreeSet<std::string::String>) {
    sequence.retain(|value| match value {
        Value::Mapping(map) => map
            .get("name")
            .and_then(Value::as_str)
            .is_none_or(|name| !names.contains(name)),
        Value::String(name) => !names.contains(name.as_str()),
        _ => true,
    });
}

fn remove_rule_strings(sequence: &mut Sequence, rules: &BTreeSet<std::string::String>) {
    sequence.retain(|value| value.as_str().is_none_or(|rule| !rules.contains(rule)));
}

pub(super) fn cleanup_seq(
    seq: &mut SeqMap,
    old: Option<&ProfileArtifacts>,
    new_proxy_names: &BTreeSet<std::string::String>,
    new_rules: &BTreeSet<std::string::String>,
) {
    let mut names = new_proxy_names.clone();
    let mut rules = new_rules.clone();
    if let Some(old) = old {
        names.extend(old.proxy_names.iter().cloned());
        if let Some(group) = old.group_name.as_ref() {
            names.insert(group.clone());
        }
        rules.extend(old.rules.iter().cloned());
    }
    remove_named(&mut seq.prepend, &names);
    remove_named(&mut seq.append, &names);
    seq.delete.retain(|name| !names.contains(name));
    remove_rule_strings(&mut seq.prepend, &rules);
    remove_rule_strings(&mut seq.append, &rules);
    seq.delete.retain(|rule| !rules.contains(rule));
}

pub(super) fn write_merge(
    path: &Path,
    config: &OpenP2PConfig,
    old: Option<&ProfileArtifacts>,
    dns: &BTreeMap<std::string::String, Value>,
) -> Result<BTreeMap<std::string::String, Option<Value>>> {
    let mut mapping = if path.exists() {
        read_mapping_sync(path)?
    } else {
        Mapping::new()
    };
    mapping.insert(Value::from("x-openp2p"), serde_yaml_ng::to_value(config)?);

    let old_domains: BTreeSet<std::string::String> = old
        .map(|old| old.dns_domains.iter().cloned().collect())
        .unwrap_or_default();
    let mut previous: BTreeMap<std::string::String, Option<Value>> =
        old.map(|old| old.dns_previous.clone()).unwrap_or_default();
    for domain in &old_domains {
        previous.entry(domain.clone()).or_insert(None);
    }

    let mut dns_mapping = mapping
        .get("dns")
        .and_then(Value::as_mapping)
        .cloned()
        .unwrap_or_default();
    let mut policy = dns_mapping
        .get("nameserver-policy")
        .and_then(Value::as_mapping)
        .cloned()
        .unwrap_or_default();

    for domain in &old_domains {
        if dns.contains_key(domain) {
            continue;
        }
        match previous.get(domain).cloned().flatten() {
            Some(value) => {
                policy.insert(Value::from(domain.as_str()), value);
            }
            None => {
                policy.remove(Value::from(domain.as_str()));
            }
        }
    }
    for (domain, value) in dns {
        if !old_domains.contains(domain) {
            previous.insert(domain.clone(), get_key(&policy, domain).cloned());
        }
        policy.insert(Value::from(domain.as_str()), value.clone());
    }
    previous.retain(|domain, _| dns.contains_key(domain));

    if policy.is_empty() {
        remove_key(&mut dns_mapping, "nameserver-policy");
    } else {
        dns_mapping.insert("nameserver-policy".into(), Value::Mapping(policy));
    }
    if dns_mapping.is_empty() {
        remove_key(&mut mapping, "dns");
    } else {
        mapping.insert("dns".into(), Value::Mapping(dns_mapping));
    }
    save_mapping_sync(path, &mapping)?;
    Ok(previous)
}

pub(super) fn cleanup_merge(path: &Path, old: &ProfileArtifacts) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let mut mapping = read_mapping_sync(path)?;
    remove_key(&mut mapping, "x-openp2p");
    let mut dns_mapping = mapping
        .get("dns")
        .and_then(Value::as_mapping)
        .cloned()
        .unwrap_or_default();
    let mut policy = dns_mapping
        .get("nameserver-policy")
        .and_then(Value::as_mapping)
        .cloned()
        .unwrap_or_default();
    for domain in &old.dns_domains {
        match old.dns_previous.get(domain).cloned().flatten() {
            Some(value) => {
                policy.insert(Value::from(domain.as_str()), value);
            }
            None => {
                policy.remove(Value::from(domain.as_str()));
            }
        }
    }
    if policy.is_empty() {
        remove_key(&mut dns_mapping, "nameserver-policy");
    } else {
        dns_mapping.insert("nameserver-policy".into(), Value::Mapping(policy));
    }
    if dns_mapping.is_empty() {
        remove_key(&mut mapping, "dns");
    } else {
        mapping.insert("dns".into(), Value::Mapping(dns_mapping));
    }
    save_mapping_sync(path, &mapping)
}

pub(super) fn seq_names(seq: &SeqMap) -> BTreeSet<std::string::String> {
    let mut names = BTreeSet::new();
    for value in seq.prepend.iter().chain(seq.append.iter()) {
        match value {
            Value::Mapping(mapping) => {
                if let Some(name) = mapping.get("name").and_then(Value::as_str) {
                    names.insert(name.to_owned());
                }
            }
            Value::String(name) => {
                names.insert(name.clone());
            }
            _ => {}
        }
    }
    names
}

pub(super) fn reject_proxy_conflicts(
    seq: &SeqMap,
    old: Option<&ProfileArtifacts>,
    new_names: &BTreeSet<std::string::String>,
) -> Result<()> {
    let existing = seq_names(seq);
    let managed: BTreeSet<std::string::String> = old
        .map(|old| old.proxy_names.iter().cloned().collect())
        .unwrap_or_default();
    for name in new_names {
        if existing.contains(name) && !managed.contains(name) {
            bail!("proxy name {name} is already defined by user chain items");
        }
    }
    Ok(())
}

pub(super) fn reject_group_conflict(seq: &SeqMap, old: Option<&ProfileArtifacts>) -> Result<()> {
    let existing = seq_names(seq);
    let managed = old.and_then(|old| old.group_name.clone());
    if existing.contains(GROUP_NAME) && managed.as_deref() != Some(GROUP_NAME) {
        bail!("proxy group {GROUP_NAME} is already defined by user chain items");
    }
    Ok(())
}

pub(super) fn apply_seq_files(plan: &ChainPlan, old: Option<&ProfileArtifacts>, artifacts: &ArtifactSet) -> Result<()> {
    let proxy_names: BTreeSet<std::string::String> = artifacts.proxy_names.iter().cloned().collect();
    let rule_names: BTreeSet<std::string::String> = artifacts.rule_texts.iter().cloned().collect();

    let mut proxies = read_seq(&plan.proxies.path)?;
    reject_proxy_conflicts(&proxies, old, &proxy_names)?;
    cleanup_seq(&mut proxies, old, &proxy_names, &BTreeSet::new());
    proxies.prepend.splice(0..0, artifacts.proxies.iter().cloned());
    save_seq_sync(&plan.proxies.path, &proxies)?;

    let mut groups = read_seq(&plan.groups.path)?;
    reject_group_conflict(&groups, old)?;
    let mut group_names = BTreeSet::new();
    if artifacts.group.is_some() {
        group_names.insert(GROUP_NAME.to_owned());
    }
    cleanup_seq(&mut groups, old, &group_names, &BTreeSet::new());
    if let Some(group) = artifacts.group.as_ref() {
        groups.prepend.insert(0, group.clone());
    }
    save_seq_sync(&plan.groups.path, &groups)?;

    let mut rules = read_seq(&plan.rules.path)?;
    cleanup_seq(&mut rules, old, &BTreeSet::new(), &rule_names);
    rules.prepend.splice(0..0, artifacts.rules.iter().cloned());
    save_seq_sync(&plan.rules.path, &rules)?;
    Ok(())
}
