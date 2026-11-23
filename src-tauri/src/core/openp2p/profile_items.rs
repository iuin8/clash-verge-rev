//! Managed PrfItem projection for OpenP2P accounts (M2, not wired yet).
#![allow(dead_code)] // wired by profile sync in M2/M3

use anyhow::{Result, bail};

use crate::config::{IProfiles, PrfItem};

pub const MANAGED_DESC_PREFIX: &str = "openp2p-managed:";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedProfile {
    pub account_id: std::string::String,
    pub uid: std::string::String,
    pub name: std::string::String,
    pub file: std::string::String,
    pub updated: u64,
}

pub fn sync_managed_items(
    profiles: &mut IProfiles,
    managed: &[ManagedProfile],
    previous_uids: &[std::string::String],
) -> Result<bool> {
    if profiles.items.is_none() {
        profiles.items = Some(Vec::new());
    }
    let Some(items) = profiles.items.as_mut() else {
        bail!("profiles.items must be initialized");
    };
    let mut changed = false;
    let current_uids = managed
        .iter()
        .map(|entry| entry.uid.as_str())
        .collect::<std::collections::BTreeSet<_>>();

    for entry in managed {
        if let Some(item) = items
            .iter_mut()
            .find(|item| item.uid.as_deref() == Some(entry.uid.as_str()))
        {
            if item.itype.as_deref() != Some("local") || item.file.as_deref() != Some(entry.file.as_str()) {
                bail!("managed profile {} conflicts with an existing item", entry.uid);
            }
            if item.option.is_some() {
                item.option = None;
                changed = true;
            }
            if item.desc.as_deref() != Some(format!("{MANAGED_DESC_PREFIX}{}", entry.account_id).as_str()) {
                item.desc = Some(format!("{MANAGED_DESC_PREFIX}{}", entry.account_id).into());
                changed = true;
            }
            if item.updated != Some(entry.updated as usize) {
                item.updated = Some(entry.updated as usize);
                changed = true;
            }
        } else {
            items.push(PrfItem {
                uid: Some(entry.uid.clone().into()),
                itype: Some("local".into()),
                name: Some(entry.name.clone().into()),
                file: Some(entry.file.clone().into()),
                desc: Some(format!("{MANAGED_DESC_PREFIX}{}", entry.account_id).into()),
                updated: Some(entry.updated as usize),
                ..PrfItem::default()
            });
            changed = true;
        }
    }

    let before = items.len();
    items.retain(|item| {
        let Some(uid) = item.uid.as_deref() else {
            return true;
        };
        if current_uids.contains(uid) {
            return true;
        }
        let previously_managed = previous_uids.iter().any(|previous| previous == uid);
        !(previously_managed
            && item
                .desc
                .as_deref()
                .is_some_and(|desc| desc.starts_with(MANAGED_DESC_PREFIX)))
    });
    changed |= items.len() != before;
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::{ManagedProfile, sync_managed_items};
    use crate::config::IProfiles;

    fn entry(id: &str, updated: u64) -> ManagedProfile {
        ManagedProfile {
            account_id: id.into(),
            uid: format!("op2p-{id}"),
            name: format!("OpenP2P {id}"),
            file: format!("op2p-{id}.yaml"),
            updated,
        }
    }

    #[test]
    fn adds_managed_items_without_stealing_current() {
        let mut profiles = IProfiles::default();
        assert!(sync_managed_items(&mut profiles, &[entry("home", 1)], &[]).unwrap());
        assert!(profiles.current.is_none());
        let items = profiles.items.as_ref().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].itype.as_deref(), Some("local"));
        assert!(items[0].option.is_none());
        assert_eq!(items[0].uid.as_deref(), Some("op2p-home"));
        assert!(
            items[0]
                .desc
                .as_deref()
                .unwrap()
                .starts_with(super::MANAGED_DESC_PREFIX)
        );
    }

    #[test]
    fn sync_is_idempotent_and_removal_is_explicit() {
        let mut profiles = IProfiles::default();
        let one = entry("home", 1);
        assert!(sync_managed_items(&mut profiles, std::slice::from_ref(&one), &[]).unwrap());
        assert!(!sync_managed_items(&mut profiles, &[one], &["op2p-home".into()]).unwrap());
        assert!(sync_managed_items(&mut profiles, &[], &["op2p-home".into()]).unwrap());
        assert!(profiles.items.as_ref().unwrap().is_empty());
    }

    #[test]
    fn does_not_overwrite_a_foreign_item_with_the_same_uid() {
        let mut profiles = IProfiles {
            current: None,
            items: Some(vec![crate::config::PrfItem {
                uid: Some("op2p-home".into()),
                itype: Some("remote".into()),
                file: Some("foreign.yaml".into()),
                ..Default::default()
            }]),
        };
        assert!(sync_managed_items(&mut profiles, &[entry("home", 1)], &[]).is_err());
        assert_eq!(profiles.items.as_ref().unwrap()[0].itype.as_deref(), Some("remote"));
    }
}
