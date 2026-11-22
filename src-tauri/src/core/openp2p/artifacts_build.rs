use std::collections::{BTreeMap, BTreeSet};

use serde_yaml_ng::{Mapping, Value};

use super::ArtifactSet;
use super::schema::{
    self, AppProtocol, GROUP_NAME, OpenP2PConfig, OpenP2PIssue, ProxySpec, RULE_TARGET, dns_policy_key,
};
use crate::core::openp2p::account::{key_path, relative_key_path, token_present};

fn resolve_app<'a>(config: &'a OpenP2PConfig, proxy: &'a ProxySpec) -> Option<&'a schema::AppSpec> {
    let account = config.account(&proxy.account)?;
    let tcp_apps: Vec<&schema::AppSpec> = account
        .apps
        .iter()
        .filter(|app| app.enabled.is_enabled() && app.protocol() == AppProtocol::Tcp)
        .collect();
    match proxy.app.as_deref() {
        Some(name) => tcp_apps.into_iter().find(|app| app.name == name),
        None if tcp_apps.len() == 1 => tcp_apps.into_iter().next(),
        None => None,
    }
}

pub(super) fn build_artifacts(config: &OpenP2PConfig, require_secrets: bool) -> ArtifactSet {
    build_artifacts_with(config, require_secrets, token_present, |reference| {
        key_path(reference).ok().is_some_and(|path| path.is_file())
    })
}

pub(super) fn build_artifacts_with<T, K>(
    config: &OpenP2PConfig,
    require_secrets: bool,
    token_ok: T,
    key_ok: K,
) -> ArtifactSet
where
    T: Fn(&str) -> bool,
    K: Fn(&str) -> bool,
{
    if !config.enabled || config.is_empty() {
        return ArtifactSet::empty();
    }
    let mut warnings = Vec::new();
    let mut missing_secret = false;
    for account in config.accounts.iter().filter(|account| account.enabled) {
        if !token_ok(&account.token_ref) {
            missing_secret = true;
            warnings.push(OpenP2PIssue::warning(
                "token_missing",
                format!("accounts.{}", account.id),
                "token is not saved; no mihomo nodes are generated until it is",
            ));
        }
    }
    for proxy in &config.proxies {
        let Some(account) = config.account(&proxy.account) else {
            continue;
        };
        if !account.enabled {
            continue;
        }
        if !key_ok(&proxy.private_key_ref) {
            missing_secret = true;
            warnings.push(OpenP2PIssue::warning(
                "key_missing",
                format!("proxies.{}", proxy.name),
                "private key is not imported; no mihomo nodes are generated until it is",
            ));
        }
    }
    if missing_secret && require_secrets {
        let mut set = ArtifactSet::empty();
        set.warnings = warnings;
        return set;
    }

    let mut set = ArtifactSet::empty();
    set.warnings = warnings;
    for proxy in &config.proxies {
        let Some(account) = config.account(&proxy.account) else {
            continue;
        };
        if !account.enabled {
            continue;
        }
        let Some(app) = resolve_app(config, proxy) else {
            continue;
        };
        let Some(host_key) = proxy.host_key.as_ref() else {
            continue;
        };
        let Ok(private_key) = relative_key_path(&proxy.private_key_ref) else {
            continue;
        };
        let mut node = Mapping::new();
        node.insert("name".into(), Value::from(proxy.name.as_str()));
        node.insert("type".into(), Value::from("ssh"));
        node.insert("server".into(), Value::from("127.0.0.1"));
        node.insert("port".into(), Value::from(app.src_port));
        node.insert("username".into(), Value::from(proxy.ssh_user.as_str()));
        node.insert("private-key".into(), Value::from(private_key));
        node.insert("host-key".into(), Value::Sequence(vec![Value::from(host_key.as_str())]));
        set.proxy_names.push(proxy.name.clone());
        set.proxies.push(Value::Mapping(node));
    }

    if !set.proxies.is_empty() {
        let mut group = Mapping::new();
        group.insert("name".into(), Value::from(GROUP_NAME));
        group.insert("type".into(), Value::from("fallback"));
        group.insert(
            "proxies".into(),
            Value::Sequence(set.proxy_names.iter().map(|name| Value::from(name.as_str())).collect()),
        );
        group.insert("url".into(), Value::from(group_health_url(config)));
        group.insert("interval".into(), Value::from(60));
        group.insert("lazy".into(), Value::from(false));
        group.insert("max-failed-times".into(), Value::from(1));
        group.insert("empty-fallback".into(), Value::from("REJECT"));
        set.group = Some(Value::Mapping(group));

        set.rule_texts = network_rules(config);
        set.rules = set.rule_texts.iter().map(|rule| Value::from(rule.as_str())).collect();
        set.dns = network_dns(config);
    }
    set
}

pub(super) fn network_rules(config: &OpenP2PConfig) -> Vec<std::string::String> {
    let mut udp_rules = Vec::new();
    let mut tunnel_rules = Vec::new();
    let mut seen_rules = BTreeSet::new();
    for network in &config.networks {
        for domain in &network.domains {
            let udp = format!("AND,((NETWORK,UDP),(DOMAIN-SUFFIX,{domain})),REJECT");
            if seen_rules.insert(udp.clone()) {
                udp_rules.push(udp);
            }
            let rule = format!("DOMAIN-SUFFIX,{domain},{RULE_TARGET}");
            if seen_rules.insert(rule.clone()) {
                tunnel_rules.push(rule);
            }
        }
        for cidr in &network.cidrs {
            let udp = format!("AND,((NETWORK,UDP),(IP-CIDR,{cidr},no-resolve)),REJECT");
            if seen_rules.insert(udp.clone()) {
                udp_rules.push(udp);
            }
            let rule = format!("IP-CIDR,{cidr},{RULE_TARGET},no-resolve");
            if seen_rules.insert(rule.clone()) {
                tunnel_rules.push(rule);
            }
        }
    }
    udp_rules.into_iter().chain(tunnel_rules).collect()
}

pub(super) fn network_dns(config: &OpenP2PConfig) -> BTreeMap<std::string::String, Value> {
    let mut dns_entries: BTreeMap<std::string::String, Vec<std::string::String>> = BTreeMap::new();
    for network in &config.networks {
        if network.dns.is_empty() {
            continue;
        }
        for domain in &network.domains {
            let key = dns_policy_key(domain);
            let servers = dns_entries.entry(key).or_default();
            for server in &network.dns {
                let value = format!("tcp://{server}#{RULE_TARGET}");
                if !servers.contains(&value) {
                    servers.push(value);
                }
            }
        }
    }
    dns_entries
        .into_iter()
        .map(|(domain, servers)| (domain, Value::Sequence(servers.into_iter().map(Value::from).collect())))
        .collect()
}

fn group_health_url(config: &OpenP2PConfig) -> std::string::String {
    for network in &config.networks {
        if let Some(health) = network.health.as_ref() {
            let host = health.host.trim();
            return match health.port.unwrap_or(80) {
                80 => format!("http://{host}/"),
                443 => format!("https://{host}/"),
                port => format!("http://{host}:{port}/"),
            };
        }
    }
    "https://www.gstatic.com/generate_204".into()
}
