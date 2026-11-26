use std::collections::{BTreeMap, BTreeSet};

use super::schema::{
    AccountSpec, AppSpec, OpenP2PConfig, OpenP2PIssue, normalize_cidr, normalize_dns_server, normalize_domain,
    valid_host, valid_interface, valid_node_name, valid_slug, valid_whitelist,
};
use super::secrets::parse_token;

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
    if config.networks.len() > 64 {
        issues.push(OpenP2PIssue::error(
            "too_many_networks",
            "networks",
            "at most 64 networks are supported",
        ));
    }
    if config.accounts.is_empty() && !config.networks.is_empty() {
        issues.push(OpenP2PIssue::error(
            "accounts_missing",
            "accounts",
            "at least one account is required when networks are listed",
        ));
    }

    let mut account_ids = BTreeSet::new();
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
        if let Some(token) = account.token.as_mut() {
            match parse_token(token) {
                Ok(parsed) => *token = parsed.as_u64().to_string(),
                Err(error) => issues.push(OpenP2PIssue::error(
                    "invalid_secret",
                    format!("{base}.token"),
                    format!("{error:#}"),
                )),
            }
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
            validate_app(app, &app_base, &mut issues);
            if valid_slug(&app.name, 64) && !app_names.insert(app.name.clone()) {
                issues.push(OpenP2PIssue::error(
                    "duplicate_app",
                    format!("{app_base}.name"),
                    "duplicate app name in the same account",
                ));
            }
            if (20000..=29999).contains(&app.src_port) {
                if let Some(other) = src_ports.get(&app.src_port) {
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
            }
        }
    }

    let accounts_by_id: BTreeMap<&str, &AccountSpec> = config
        .accounts
        .iter()
        .map(|account| (account.id.as_str(), account))
        .collect();
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
        if let Some(account) = network.account.as_deref()
            && !accounts_by_id.contains_key(account)
        {
            issues.push(OpenP2PIssue::error(
                "network_account_missing",
                format!("{base}.account"),
                "network references an unknown account",
            ));
        }
    }

    issues
}

fn validate_app(app: &AppSpec, app_base: &str, issues: &mut Vec<OpenP2PIssue>) {
    if !valid_slug(&app.name, 64) {
        issues.push(OpenP2PIssue::error(
            "invalid_app_name",
            format!("{app_base}.name"),
            "app name must match [a-z0-9][a-z0-9_.-]* (max 64)",
        ));
    }
    if app.protocol.is_none() {
        issues.push(OpenP2PIssue::error(
            "invalid_protocol",
            format!("{app_base}.protocol"),
            "protocol must be tcp or udp",
        ));
    }
    if app.src_port == 0 {
        issues.push(OpenP2PIssue::error(
            "src_port_required",
            format!("{app_base}.src_port"),
            "src_port is required and is never allocated automatically",
        ));
    } else if !(20000..=29999).contains(&app.src_port) {
        issues.push(OpenP2PIssue::error(
            "invalid_port",
            format!("{app_base}.src_port"),
            "src_port must be between 20000 and 29999",
        ));
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
