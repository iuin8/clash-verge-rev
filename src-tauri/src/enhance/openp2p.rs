//! Applies the OpenP2P overlay after normal profile/bundle enhancement.
//!
//! A profile bundle has no per-profile chain items, so the OpenP2P manager keeps
//! its generated nodes in a separate overlay file. The overlay is keyed by the
//! active profile uid and ignored for every other profile.

use serde_yaml_ng::{Mapping, Sequence, Value};

use crate::core::openp2p::account;

pub fn apply(config: Mapping, profile_uid: &str) -> Mapping {
    let Some(overlay) = read_overlay(profile_uid) else {
        return config;
    };
    apply_mapping(config, &overlay)
}

fn read_overlay(profile_uid: &str) -> Option<Mapping> {
    let path = account::overlay_path().ok()?;
    let raw = std::fs::read_to_string(path).ok()?;
    let overlay = serde_yaml_ng::from_str::<Mapping>(&raw).ok()?;
    if overlay.get(Value::from("profile")).and_then(Value::as_str) != Some(profile_uid) {
        return None;
    }
    Some(overlay)
}

fn apply_mapping(mut config: Mapping, overlay: &Mapping) -> Mapping {
    if let Some(proxies) = overlay.get(Value::from("proxies")).and_then(Value::as_sequence) {
        append_sequence(&mut config, "proxies", proxies);
    }
    if let Some(groups) = overlay.get(Value::from("proxy-groups")).and_then(Value::as_sequence) {
        append_sequence(&mut config, "proxy-groups", groups);
    }
    if let Some(rules) = overlay.get(Value::from("rules")).and_then(Value::as_sequence) {
        prepend_sequence(&mut config, "rules", rules);
    }
    if let Some(policy) = overlay
        .get(Value::from("dns"))
        .and_then(Value::as_mapping)
        .and_then(|dns| dns.get(Value::from("nameserver-policy")))
        .and_then(Value::as_mapping)
    {
        extend_nameserver_policy(&mut config, policy);
    }
    config
}

fn append_sequence(config: &mut Mapping, key: &str, items: &Sequence) {
    if items.is_empty() {
        return;
    }
    let entry = config
        .entry(Value::from(key))
        .or_insert_with(|| Value::Sequence(Sequence::new()));
    let Some(sequence) = entry.as_sequence_mut() else {
        *entry = Value::Sequence(items.clone());
        return;
    };
    for item in items {
        if let Some(name) = item.get("name").and_then(Value::as_str) {
            sequence.retain(|existing| existing.get("name").and_then(Value::as_str) != Some(name));
        }
        sequence.push(item.clone());
    }
}

fn prepend_sequence(config: &mut Mapping, key: &str, items: &Sequence) {
    if items.is_empty() {
        return;
    }
    let entry = config
        .entry(Value::from(key))
        .or_insert_with(|| Value::Sequence(Sequence::new()));
    let Some(sequence) = entry.as_sequence_mut() else {
        *entry = Value::Sequence(items.clone());
        return;
    };
    let old = std::mem::take(sequence);
    sequence.extend(items.iter().cloned());
    sequence.extend(old);
}

fn extend_nameserver_policy(config: &mut Mapping, policy: &Mapping) {
    if policy.is_empty() {
        return;
    }
    let dns = config
        .entry(Value::from("dns"))
        .or_insert_with(|| Value::Mapping(Mapping::new()));
    let Some(dns) = dns.as_mapping_mut() else {
        return;
    };
    let nameserver = dns
        .entry(Value::from("nameserver-policy"))
        .or_insert_with(|| Value::Mapping(Mapping::new()));
    let Some(nameserver) = nameserver.as_mapping_mut() else {
        return;
    };
    for (domain, servers) in policy {
        nameserver.insert(domain.clone(), servers.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::apply_mapping;
    use serde_yaml_ng::{Mapping, Value};

    fn mapping(raw: &str) -> Mapping {
        serde_yaml_ng::from_str(raw).expect("mapping")
    }

    #[test]
    fn overlay_appends_nodes_groups_and_prepends_rules() {
        let config = mapping(
            "proxies: [{name: airport}]
proxy-groups: [{name: MAIN}]
rules: [MATCH,MAIN]
dns: {nameserver: [1.1.1.1]}",
        );
        let overlay = mapping(
            "proxies: [{name: op2p-home, type: ssh}]
proxy-groups: [{name: OP2P, type: fallback}]
rules: ['DOMAIN-SUFFIX,corp.example,OP2P']
dns: {nameserver-policy: {'+.corp.example': ['tcp://10.0.0.53:53#OP2P']}}",
        );
        let result = apply_mapping(config, &overlay);
        let proxies = result
            .get(Value::from("proxies"))
            .and_then(Value::as_sequence)
            .expect("proxies");
        assert_eq!(proxies.len(), 2);
        assert_eq!(proxies[1].get("name").and_then(Value::as_str), Some("op2p-home"));
        let rules = result
            .get(Value::from("rules"))
            .and_then(Value::as_sequence)
            .expect("rules");
        assert_eq!(rules[0].as_str(), Some("DOMAIN-SUFFIX,corp.example,OP2P"));
        let groups = result
            .get(Value::from("proxy-groups"))
            .and_then(Value::as_sequence)
            .expect("groups");
        assert_eq!(groups[1].get("name").and_then(Value::as_str), Some("OP2P"));
        let dns = result.get(Value::from("dns")).and_then(Value::as_mapping).expect("dns");
        assert!(dns.get(Value::from("nameserver-policy")).is_some());
        assert!(dns.get(Value::from("nameserver")).is_some());
    }

    #[test]
    fn overlay_replaces_same_named_generated_nodes() {
        let config = mapping("proxies: [{name: op2p-home, port: 1}]");
        let overlay = mapping("proxies: [{name: op2p-home, port: 2}]");
        let result = apply_mapping(config, &overlay);
        let proxies = result
            .get(Value::from("proxies"))
            .and_then(Value::as_sequence)
            .expect("proxies");
        assert_eq!(proxies.len(), 1);
        assert_eq!(proxies[0].get("port").and_then(Value::as_u64), Some(2));
    }
}
