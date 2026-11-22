use std::collections::{BTreeMap, BTreeSet};

use super::{
    AccountSpec, AppProtocol, AppSpec, OpenP2PConfig, OpenP2PIssue, normalize_cidr, normalize_dns_server,
    normalize_domain, valid_authorized_key, valid_host, valid_interface, valid_node_name, valid_ref, valid_slug,
    valid_whitelist,
};

/// Canonicalizes user input and reports every independent validation error.
#[allow(
    clippy::cognitive_complexity,
    reason = "one pass validates every independent field group"
)]
pub fn normalize_and_validate(config: &mut OpenP2PConfig) -> Vec<OpenP2PIssue> {
    let mut issues = Vec::new();
    if config.version != 1 {
        issues.push(OpenP2PIssue::error(
            "unsupported_version",
            "version",
            format!("unsupported x-openp2p version {}; expected 1", config.version),
        ));
    }
    if config.accounts.len() > 32 {
        issues.push(OpenP2PIssue::error(
            "too_many_accounts",
            "accounts",
            "at most 32 accounts are supported",
        ));
    }
    if config.proxies.len() > 256 {
        issues.push(OpenP2PIssue::error(
            "too_many_proxies",
            "proxies",
            "at most 256 proxies are supported",
        ));
    }
    if config.networks.len() > 64 {
        issues.push(OpenP2PIssue::error(
            "too_many_networks",
            "networks",
            "at most 64 networks are supported",
        ));
    }
    if config.accounts.is_empty() && (!config.proxies.is_empty() || !config.networks.is_empty()) {
        issues.push(OpenP2PIssue::error(
            "accounts_missing",
            "accounts",
            "at least one account is required when proxies or networks are configured",
        ));
    }

    let mut account_ids = BTreeSet::new();
    let mut token_refs = BTreeSet::new();
    let mut src_ports: BTreeMap<u16, std::string::String> = BTreeMap::new();
    let mut public_ports: BTreeMap<u16, std::string::String> = BTreeMap::new();

    for (index, account) in config.accounts.iter_mut().enumerate() {
        let base = format!("accounts[{index}]");
        if !valid_slug(&account.id, 32) {
            issues.push(OpenP2PIssue::error(
                "invalid_id",
                format!("{base}.id"),
                "account id must match [a-z0-9][a-z0-9_-]* (max 32)",
            ));
        } else if !account_ids.insert(account.id.clone()) {
            issues.push(OpenP2PIssue::error(
                "duplicate_account",
                format!("{base}.id"),
                "duplicate account id",
            ));
        }
        if account.label.as_ref().is_some_and(|label| label.chars().count() > 64) {
            issues.push(OpenP2PIssue::error(
                "invalid_label",
                format!("{base}.label"),
                "label is longer than 64 characters",
            ));
        }
        if account
            .gateway
            .as_ref()
            .is_none_or(|gateway| gateway.host.trim().is_empty())
        {
            issues.push(OpenP2PIssue::error(
                "gateway_invalid",
                format!("{base}.gateway.host"),
                "gateway.host is required",
            ));
        }
        if let Some(gateway) = account.gateway.as_ref() {
            if gateway.port == 0 {
                issues.push(OpenP2PIssue::error(
                    "gateway_invalid",
                    format!("{base}.gateway.port"),
                    "gateway.port must be between 1 and 65535",
                ));
            }
            if !valid_host(&gateway.host) {
                issues.push(OpenP2PIssue::error(
                    "gateway_invalid",
                    format!("{base}.gateway.host"),
                    "gateway.host must be a hostname or IP address without scheme or port",
                ));
            }
        }
        if !valid_node_name(&account.node) {
            issues.push(OpenP2PIssue::error(
                "invalid_node",
                format!("{base}.node"),
                "node must be 8-31 characters of [A-Za-z0-9._-]",
            ));
        }
        if !valid_ref(&account.token_ref) {
            issues.push(OpenP2PIssue::error(
                "token_ref_invalid",
                format!("{base}.token_ref"),
                "token_ref must be a relative path of [a-z0-9/_-] without ..",
            ));
        } else if !token_refs.insert(account.token_ref.clone()) {
            issues.push(OpenP2PIssue::error(
                "duplicate_token_ref",
                format!("{base}.token_ref"),
                "two accounts cannot share the same token_ref",
            ));
        }
        if !(1024..=65535).contains(&account.public_ip_port) {
            issues.push(OpenP2PIssue::error(
                "invalid_port",
                format!("{base}.public_ip_port"),
                "public_ip_port must be between 1024 and 65535",
            ));
        } else if let Some(other) = public_ports.insert(account.public_ip_port, account.id.clone()) {
            issues.push(OpenP2PIssue::error(
                "duplicate_port",
                format!("{base}.public_ip_port"),
                format!(
                    "public port {} is already used by account {other}",
                    account.public_ip_port
                ),
            ));
        }
        if !valid_interface(account.interface()) {
            issues.push(OpenP2PIssue::error(
                "invalid_interface",
                format!("{base}.interface"),
                "interface must be auto or a network interface name of [A-Za-z0-9_.:-] (max 32)",
            ));
        }
        for (dns_index, server) in account.dns.iter_mut().enumerate() {
            match normalize_dns_server(server) {
                Some(normalized) => *server = normalized,
                None => issues.push(OpenP2PIssue::error(
                    "invalid_dns",
                    format!("{base}.dns[{dns_index}]"),
                    "DNS servers must be IP or IP:port",
                )),
            }
        }

        let mut app_names = BTreeSet::new();
        for (app_index, app) in account.apps.iter_mut().enumerate() {
            let app_base = format!("{base}.apps[{app_index}]");
            if !valid_slug(&app.name, 64) {
                issues.push(OpenP2PIssue::error(
                    "invalid_app_name",
                    format!("{app_base}.name"),
                    "app name must match [a-z0-9][a-z0-9_.-]* (max 64)",
                ));
            } else if !app_names.insert(app.name.clone()) {
                issues.push(OpenP2PIssue::error(
                    "duplicate_app",
                    format!("{app_base}.name"),
                    "duplicate app name in the same account",
                ));
            }
            if app.protocol.is_none() {
                issues.push(OpenP2PIssue::error(
                    "invalid_protocol",
                    format!("{app_base}.protocol"),
                    "protocol must be tcp or udp",
                ));
            }
            if !(20000..=29999).contains(&app.src_port) {
                issues.push(OpenP2PIssue::error(
                    "invalid_port",
                    format!("{app_base}.src_port"),
                    "src_port must be between 20000 and 29999",
                ));
            } else if let Some(other) = src_ports.get(&app.src_port) {
                issues.push(OpenP2PIssue::error(
                    "duplicate_port",
                    format!("{app_base}.src_port"),
                    format!("src_port {} is already used by account {other}", app.src_port),
                ));
            } else if let Some(other) = public_ports.get(&app.src_port) {
                issues.push(OpenP2PIssue::error(
                    "duplicate_port",
                    format!("{app_base}.src_port"),
                    format!(
                        "src_port {} collides with a public port of account {other}",
                        app.src_port
                    ),
                ));
            } else {
                src_ports.insert(app.src_port, account.id.clone());
            }
            if !valid_node_name(&app.peer_node) {
                issues.push(OpenP2PIssue::error(
                    "invalid_peer_node",
                    format!("{app_base}.peer_node"),
                    "peer_node must be 8-31 characters of [A-Za-z0-9._-]",
                ));
            }
            if app.dst_host.trim().is_empty() || !valid_host(&app.dst_host) {
                issues.push(OpenP2PIssue::error(
                    "invalid_dst_host",
                    format!("{app_base}.dst_host"),
                    "dst_host must be a hostname or IP address",
                ));
            }
            if app.dst_port == 0 {
                issues.push(OpenP2PIssue::error(
                    "invalid_port",
                    format!("{app_base}.dst_port"),
                    "dst_port must be between 1 and 65535",
                ));
            }
            if !valid_whitelist(app.whitelist()) {
                issues.push(OpenP2PIssue::error(
                    "invalid_whitelist",
                    format!("{app_base}.whitelist"),
                    "whitelist must be a comma-separated list of IP addresses or CIDRs",
                ));
            }
        }
    }

    let accounts_by_id: BTreeMap<std::string::String, AccountSpec> = config
        .accounts
        .iter()
        .cloned()
        .map(|account| (account.id.clone(), account))
        .collect();
    let mut proxy_names = BTreeSet::new();
    for (index, proxy) in config.proxies.iter_mut().enumerate() {
        let base = format!("proxies[{index}]");
        if !valid_slug(&proxy.name, 64) {
            issues.push(OpenP2PIssue::error(
                "invalid_proxy_name",
                format!("{base}.name"),
                "proxy name must match [a-z0-9][a-z0-9_.-]* (max 64)",
            ));
        } else if !proxy_names.insert(proxy.name.clone()) {
            issues.push(OpenP2PIssue::error(
                "duplicate_proxy",
                format!("{base}.name"),
                "duplicate proxy name",
            ));
        }
        let Some(account) = accounts_by_id.get(&proxy.account).cloned() else {
            issues.push(OpenP2PIssue::error(
                "proxy_account_missing",
                format!("{base}.account"),
                "proxy references an unknown account",
            ));
            continue;
        };
        if !account.enabled {
            issues.push(OpenP2PIssue::warning(
                "proxy_account_disabled",
                format!("{base}.account"),
                "the referenced account is disabled; this node is not generated",
            ));
        }
        let tcp_apps: Vec<&AppSpec> = account
            .apps
            .iter()
            .filter(|app| app.enabled.is_enabled() && app.protocol() == AppProtocol::Tcp)
            .collect();
        let selected = match proxy.app.as_deref() {
            Some(name) => tcp_apps.iter().copied().find(|app| app.name == name),
            None if tcp_apps.len() == 1 => tcp_apps.first().copied(),
            None => None,
        };
        match selected {
            Some(_) => {}
            None if proxy.app.is_some() => issues.push(OpenP2PIssue::error(
                "proxy_app_missing",
                format!("{base}.app"),
                "app must name an enabled TCP app of the referenced account",
            )),
            None => issues.push(OpenP2PIssue::error(
                "proxy_app_ambiguous",
                format!("{base}.app"),
                "the account has no single enabled TCP app; set app to choose one",
            )),
        }
        if !valid_slug(&proxy.ssh_user, 32) {
            issues.push(OpenP2PIssue::error(
                "invalid_ssh_user",
                format!("{base}.ssh_user"),
                "ssh_user must match [a-z0-9][a-z0-9_.-]* (max 32)",
            ));
        }
        if !valid_ref(&proxy.private_key_ref) {
            issues.push(OpenP2PIssue::error(
                "key_ref_invalid",
                format!("{base}.private_key_ref"),
                "private_key_ref must be a relative path of [a-z0-9/_-] without ..",
            ));
        }
        match proxy.host_key.as_deref() {
            Some(host_key) if valid_authorized_key(host_key) => {}
            Some(_) => issues.push(OpenP2PIssue::error(
                "host_key_invalid",
                format!("{base}.host_key"),
                "host_key must be one authorized_keys line (type + base64 key)",
            )),
            None => issues.push(OpenP2PIssue::error(
                "host_key_missing",
                format!("{base}.host_key"),
                "host_key is required",
            )),
        }
    }

    for (index, network) in config.networks.iter_mut().enumerate() {
        let base = format!("networks[{index}]");
        for (domain_index, domain) in network.domains.iter_mut().enumerate() {
            match normalize_domain(domain) {
                Some(normalized) => *domain = normalized,
                None => issues.push(OpenP2PIssue::error(
                    "invalid_domain",
                    format!("{base}.domains[{domain_index}]"),
                    "domain must be a hostname (a leading +. or . is allowed)",
                )),
            }
        }
        for (cidr_index, cidr) in network.cidrs.iter_mut().enumerate() {
            match normalize_cidr(cidr) {
                Some(normalized) => *cidr = normalized,
                None => issues.push(OpenP2PIssue::error(
                    "invalid_cidr",
                    format!("{base}.cidrs[{cidr_index}]"),
                    "cidr must be IP/prefix",
                )),
            }
        }
        for (dns_index, server) in network.dns.iter_mut().enumerate() {
            match normalize_dns_server(server) {
                Some(normalized) => *server = normalized,
                None => issues.push(OpenP2PIssue::error(
                    "invalid_dns",
                    format!("{base}.dns[{dns_index}]"),
                    "DNS servers must be IP or IP:port",
                )),
            }
        }
        if !network.dns.is_empty() && network.domains.is_empty() {
            issues.push(OpenP2PIssue::error(
                "dns_without_domains",
                format!("{base}.dns"),
                "DNS policy needs at least one domain in the same network",
            ));
        }
        if let Some(health) = network.health.as_mut() {
            health.host = health
                .host
                .trim()
                .trim_start_matches("+.")
                .trim_start_matches('.')
                .to_ascii_lowercase();
            if !valid_host(&health.host) {
                issues.push(OpenP2PIssue::error(
                    "invalid_health",
                    format!("{base}.health.host"),
                    "health.host must be a hostname or IP address",
                ));
            }
            if health.port == Some(0) {
                issues.push(OpenP2PIssue::error(
                    "invalid_health",
                    format!("{base}.health.port"),
                    "health.port must be between 1 and 65535",
                ));
            }
            let host = health.host.trim_start_matches("+.").trim_start_matches('.');
            if !network.domains.iter().any(|domain| {
                let normalized = domain.trim_start_matches("+.").trim_start_matches('.');
                host == normalized || host.ends_with(&format!(".{normalized}"))
            }) {
                issues.push(OpenP2PIssue::error(
                    "health_not_in_domains",
                    format!("{base}.health.host"),
                    "health.host must match a domain in the same network so the probe uses the tunnel",
                ));
            }
        }
    }

    issues
}
