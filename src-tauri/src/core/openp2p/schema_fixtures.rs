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
        name: Some("Home".into()),
        accounts: vec![AccountSpec {
            id: "home".into(),
            label: Some("Home".into()),
            enabled: true,
            gateway: Some(GatewaySpec {
                host: "api.openp2p.cn".into(),
                port: 27183,
            }),
            node: "mbp-fa-home".into(),
            token: None,
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
        networks: vec![NetworkSpec {
            name: Some("corp".into()),
            account: Some("home".into()),
            domains: vec!["corp.example".into()],
            cidrs: vec!["10.0.0.0/8".into()],
        }],
    }
}
