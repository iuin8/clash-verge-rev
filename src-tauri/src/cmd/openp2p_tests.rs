use super::{
    SubscriptionUpdateTrigger, fetch_settings, merge_option_patch, sanitize_option_patch, should_update_openp2p,
};
use crate::config::PrfOption;
use crate::utils::network::ProxyType;

fn option(allow_auto_update: Option<bool>) -> PrfOption {
    PrfOption {
        allow_auto_update,
        ..PrfOption::default()
    }
}

#[test]
fn generic_fetch_settings_are_honored() {
    let configured = PrfOption {
        self_proxy: Some(true),
        with_proxy: Some(true),
        timeout_seconds: Some(7),
        user_agent: Some("openp2p-agent".into()),
        danger_accept_invalid_certs: Some(true),
        ..PrfOption::default()
    };

    let (proxy, timeout, user_agent, accept_invalid_certs) = fetch_settings(Some(&configured));

    assert!(
        matches!(proxy, ProxyType::Localhost),
        "self_proxy takes the same precedence as remote profile downloads"
    );
    assert_eq!(timeout, Some(7));
    assert_eq!(user_agent.as_deref(), Some("openp2p-agent"));
    assert!(accept_invalid_certs);

    let (proxy, timeout, user_agent, accept_invalid_certs) = fetch_settings(None);
    assert!(matches!(proxy, ProxyType::None));
    assert_eq!(timeout, Some(super::FETCH_TIMEOUT_SECS));
    assert!(user_agent.is_none());
    assert!(!accept_invalid_certs);
}

#[test]
fn only_a_manual_trigger_reads_local_sources_and_honors_no_auto_flag() {
    assert!(SubscriptionUpdateTrigger::Manual.allows_local());
    assert!(!SubscriptionUpdateTrigger::Automatic.allows_local());
    assert!(!SubscriptionUpdateTrigger::Batch.allows_local());
    assert!(!SubscriptionUpdateTrigger::Manual.honors_auto_update());
    assert!(SubscriptionUpdateTrigger::Automatic.honors_auto_update());
    assert!(
        !SubscriptionUpdateTrigger::Batch.honors_auto_update(),
        "a full bundle refresh intentionally ignores per-profile auto-update flags"
    );
}

#[test]
fn auto_update_defaults_on_and_manual_bypasses_it() {
    assert!(should_update_openp2p(None, false));
    assert!(should_update_openp2p(Some(&option(None)), false));
    assert!(!should_update_openp2p(Some(&option(Some(false))), false));
    assert!(
        should_update_openp2p(Some(&option(Some(false))), true),
        "a manual update must still run when auto-update is off"
    );
}

#[test]
fn a_partial_option_patch_keeps_the_existing_update_settings() {
    let existing = PrfOption {
        user_agent: Some("old-agent".into()),
        update_interval: Some(60),
        timeout_seconds: Some(30),
        allow_auto_update: Some(true),
        ..PrfOption::default()
    };
    let patch = PrfOption {
        timeout_seconds: Some(5),
        merge: Some("m".into()),
        ..PrfOption::default()
    };

    let merged = merge_option_patch(Some(&existing), patch);

    assert_eq!(merged.user_agent.as_deref(), Some("old-agent"));
    assert_eq!(merged.update_interval, Some(60));
    assert_eq!(merged.timeout_seconds, Some(5), "the patch wins where it is set");
    assert_eq!(merged.allow_auto_update, Some(true));
    assert!(merged.merge.is_none(), "generated overlay references never survive");
}

#[test]
fn option_patch_keeps_only_generic_update_settings() {
    let sanitized = sanitize_option_patch(PrfOption {
        user_agent: Some("agent".into()),
        with_proxy: Some(true),
        self_proxy: Some(false),
        update_interval: Some(120),
        timeout_seconds: Some(9),
        danger_accept_invalid_certs: Some(true),
        allow_auto_update: Some(false),
        merge: Some("m".into()),
        script: Some("s".into()),
        rules: Some("r".into()),
        proxies: Some("p".into()),
        groups: Some("g".into()),
    });

    assert_eq!(sanitized.user_agent.as_deref(), Some("agent"));
    assert_eq!(sanitized.with_proxy, Some(true));
    assert_eq!(sanitized.self_proxy, Some(false));
    assert_eq!(sanitized.update_interval, Some(120));
    assert_eq!(sanitized.timeout_seconds, Some(9));
    assert_eq!(sanitized.danger_accept_invalid_certs, Some(true));
    assert_eq!(sanitized.allow_auto_update, Some(false));
    assert!(sanitized.merge.is_none());
    assert!(sanitized.script.is_none());
    assert!(sanitized.rules.is_none());
    assert!(sanitized.proxies.is_none());
    assert!(sanitized.groups.is_none());
}
