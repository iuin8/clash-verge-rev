//! FORK: Profile Bundle chain-item selection.
//!
//! A generated bundle has no per-profile Merge/Script/Rules/Proxies/Groups
//! chain items; applying the global fallback here would run them twice
//! (`process_global_items` already did).

use serde_yaml_ng::Mapping;
use smartstring::alias::String;

use super::{
    chain::{ChainItem, ChainType},
    chain_item_or_default,
    seq::SeqMap,
};
use crate::config::{IProfiles, PrfItem};
use crate::utils::tmpl;

pub(super) struct ChainItems {
    pub(crate) merge_item: ChainItem,
    pub(crate) script_item: ChainItem,
    pub(crate) rules_item: ChainItem,
    pub(crate) proxies_item: ChainItem,
    pub(crate) groups_item: ChainItem,
    pub(crate) global_merge: ChainItem,
    pub(crate) global_script: ChainItem,
}

fn profile_chain_uids(current_profile_uid: &String, current_item: &PrfItem) -> [Option<String>; 5] {
    let is_bundle = crate::config::bundle::is_bundle_uid(current_profile_uid);
    [
        (!is_bundle).then(|| current_item.current_merge().cloned().unwrap_or_else(|| "Merge".into())),
        (!is_bundle).then(|| {
            current_item
                .current_script()
                .cloned()
                .unwrap_or_else(|| "Script".into())
        }),
        (!is_bundle).then(|| current_item.current_rules().cloned().unwrap_or_else(|| "Rules".into())),
        (!is_bundle).then(|| {
            current_item
                .current_proxies()
                .cloned()
                .unwrap_or_else(|| "Proxies".into())
        }),
        (!is_bundle).then(|| {
            current_item
                .current_groups()
                .cloned()
                .unwrap_or_else(|| "Groups".into())
        }),
    ]
}

pub(super) async fn collect_chain_items(
    profiles: &IProfiles,
    current_item: &PrfItem,
    current_profile_uid: &String,
) -> ChainItems {
    let [merge_uid, script_uid, rules_uid, proxies_uid, groups_uid] =
        profile_chain_uids(current_profile_uid, current_item);

    let (merge_item, script_item, rules_item, proxies_item, groups_item, global_merge, global_script) = tokio::join!(
        chain_item_or_default(merge_uid.as_ref().and_then(|uid| profiles.get_item(uid).ok()), || {
            ChainItem {
                uid: "".into(),
                data: ChainType::Merge(Mapping::new()),
            }
        },),
        chain_item_or_default(script_uid.as_ref().and_then(|uid| profiles.get_item(uid).ok()), || {
            ChainItem {
                uid: "".into(),
                data: ChainType::Script(tmpl::ITEM_SCRIPT.into()),
            }
        },),
        chain_item_or_default(rules_uid.as_ref().and_then(|uid| profiles.get_item(uid).ok()), || {
            ChainItem {
                uid: "".into(),
                data: ChainType::Rules(SeqMap::default()),
            }
        },),
        chain_item_or_default(proxies_uid.as_ref().and_then(|uid| profiles.get_item(uid).ok()), || {
            ChainItem {
                uid: "".into(),
                data: ChainType::Proxies(SeqMap::default()),
            }
        },),
        chain_item_or_default(groups_uid.as_ref().and_then(|uid| profiles.get_item(uid).ok()), || {
            ChainItem {
                uid: "".into(),
                data: ChainType::Groups(SeqMap::default()),
            }
        },),
        chain_item_or_default(profiles.get_item("Merge").ok(), || ChainItem {
            uid: "Merge".into(),
            data: ChainType::Merge(Mapping::new()),
        },),
        chain_item_or_default(profiles.get_item("Script").ok(), || ChainItem {
            uid: "Script".into(),
            data: ChainType::Script(tmpl::ITEM_SCRIPT.into()),
        },),
    );

    ChainItems {
        merge_item,
        script_item,
        rules_item,
        proxies_item,
        groups_item,
        global_merge,
        global_script,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::profile_chain_uids;
    use crate::config::PrfItem;
    use smartstring::alias::String;

    #[test]
    fn bundle_profiles_skip_per_profile_chain_uids() {
        let item = PrfItem::default();
        let bundle = String::from(crate::config::bundle::BUNDLE_UID);
        let [merge, script, rules, proxies, groups] = profile_chain_uids(&bundle, &item);
        assert!(merge.is_none());
        assert!(script.is_none());
        assert!(rules.is_none());
        assert!(proxies.is_none());
        assert!(groups.is_none());
    }

    #[test]
    fn normal_profiles_keep_upstream_fallback_uids() {
        let item = PrfItem::default();
        let normal = String::from("normal-profile");
        let [merge, script, rules, proxies, groups] = profile_chain_uids(&normal, &item);
        assert_eq!(merge.as_deref(), Some("Merge"));
        assert_eq!(script.as_deref(), Some("Script"));
        assert_eq!(rules.as_deref(), Some("Rules"));
        assert_eq!(proxies.as_deref(), Some("Proxies"));
        assert_eq!(groups.as_deref(), Some("Groups"));
    }
}
