use super::*;

#[cfg(test)]
pub(crate) fn sample_host_key() -> std::string::String {
    let kind = "ssh-ed25519";
    let mut blob = Vec::new();
    blob.extend((kind.len() as u32).to_be_bytes());
    blob.extend(kind.as_bytes());
    blob.extend([7u8; 32]);
    use base64::Engine as _;
    format!("{kind} {}", base64::engine::general_purpose::STANDARD.encode(blob))
}

#[cfg(test)]
pub(crate) fn sample_config() -> OpenP2PConfig {
    OpenP2PConfig {
        version: 1,
        enabled: true,
        accounts: vec![AccountSpec {
            id: "home".into(),
            label: Some("Home".into()),
            enabled: true,
            gateway: Some(GatewaySpec {
                host: "api.openp2p.cn".into(),
                port: 27183,
            }),
            node: "mbp-fa-home".into(),
            token_ref: "o2p/home".into(),
            public_ip_port: 2065,
            interface: None,
            dns: vec!["223.5.5.5:53".into()],
            apps: vec![AppSpec {
                name: "ssh-home".into(),
                protocol: Some(AppProtocol::Tcp),
                src_port: 21001,
                peer_node: "home-exit-01".into(),
                dst_host: "127.0.0.1".into(),
                dst_port: 22,
                whitelist: None,
                enabled: Enabled::Bool(true),
            }],
        }],
        proxies: vec![ProxySpec {
            name: "op2p-home".into(),
            account: "home".into(),
            app: Some("ssh-home".into()),
            ssh_user: "tunnel".into(),
            private_key_ref: "o2p/home/id_ed25519".into(),
            host_key: Some(sample_host_key()),
        }],
        networks: vec![NetworkSpec {
            name: Some("corp".into()),
            account: Some("home".into()),
            domains: vec!["corp.example".into()],
            cidrs: vec!["10.0.0.0/8".into()],
            dns: vec!["10.0.0.53:53".into()],
            health: Some(HealthSpec {
                host: "health.corp.example".into(),
                port: Some(80),
            }),
        }],
    }
}
