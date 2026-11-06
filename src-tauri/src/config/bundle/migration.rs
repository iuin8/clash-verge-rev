//! One-shot migration of the removed `merged.json` UID store into a Profile Bundle.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result};
use clash_verge_draft::Draft;
use clash_verge_logging::{Type, logging};
use serde::Deserialize;
use smartstring::alias::String;
use tokio::fs;

use super::{BUNDLE_UID, MIN_MEMBERS, generate, is_bundle_uid, upsert_item, validate_member_item, write};
use crate::config::{IProfiles, PrfItem};
use crate::enhance::multi_merge;
use crate::utils::dirs;

const LEGACY_MERGED_FILE: &str = "merged.json";

#[derive(Debug, Default)]
pub struct MigrationOutcome {
    pub migrated: bool,
    pub archived: Option<PathBuf>,
}

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationPlan {
    pub members: Vec<String>,
    pub dropped: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
struct LegacyMergedStore {
    #[serde(default)]
    uids: Vec<String>,
}

pub(super) fn parse_legacy(body: &str) -> Result<Vec<String>> {
    let store: LegacyMergedStore = serde_json::from_str(body).context("invalid merged.json")?;
    Ok(store.uids)
}

/// Keep only legacy uids that still exist as subscription profiles, preserving
/// order. Returns `None` when fewer than two usable members remain.
pub fn plan_migration(items: &[PrfItem], legacy: &[String]) -> Option<MigrationPlan> {
    let mut members: Vec<String> = Vec::new();
    let mut dropped: Vec<String> = Vec::new();
    for uid in legacy {
        if is_bundle_uid(uid) || members.iter().any(|member| member == uid) {
            dropped.push(uid.clone());
            continue;
        }
        let usable = items
            .iter()
            .find(|item| item.uid.as_deref() == Some(uid.as_str()))
            .is_some_and(|item| validate_member_item(uid, item).is_ok());
        if usable {
            members.push(uid.clone());
        } else {
            dropped.push(uid.clone());
        }
    }
    (members.len() >= MIN_MEMBERS).then_some(MigrationPlan { members, dropped })
}

fn archive_path(path: &Path, suffix: &str) -> PathBuf {
    let file = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(LEGACY_MERGED_FILE);
    path.with_file_name(format!("{file}.{suffix}"))
}

async fn archive_legacy(path: &Path, suffix: &str) -> Result<PathBuf> {
    let mut target = archive_path(path, suffix);
    if fs::try_exists(&target).await.unwrap_or(false) {
        target = archive_path(path, &format!("{suffix}-{}", now_millis()));
    }
    fs::rename(path, &target)
        .await
        .with_context(|| format!("failed to archive {} to {}", path.display(), target.display()))?;
    Ok(target)
}

/// One-shot migration of the removed `merged.json` store into a bundle. Never
/// deletes the old file: valid state is archived as `.migrated`, corrupt state
/// as `.corrupt-<ts>` so it can be inspected/recovered.
pub async fn migrate_legacy(profiles: &Draft<IProfiles>) -> Result<MigrationOutcome> {
    let legacy_path = dirs::app_home_dir()?.join(LEGACY_MERGED_FILE);
    if !fs::try_exists(&legacy_path).await.unwrap_or(false) {
        return Ok(MigrationOutcome::default());
    }

    let snapshot = profiles.latest_arc();
    if snapshot.get_item(BUNDLE_UID).is_ok() {
        let archived = archive_legacy(&legacy_path, "migrated").await?;
        logging!(
            info,
            Type::Config,
            "legacy merged.json archived next to an existing bundle: {}",
            archived.display()
        );
        return Ok(MigrationOutcome {
            migrated: false,
            archived: Some(archived),
        });
    }
    drop(snapshot);

    let body = fs::read_to_string(&legacy_path)
        .await
        .with_context(|| format!("failed to read {}", legacy_path.display()))?;
    let legacy_uids = match parse_legacy(&body) {
        Ok(uids) => uids,
        Err(error) => {
            let archived = archive_legacy(&legacy_path, &format!("corrupt-{}", now_millis())).await?;
            logging!(
                error,
                Type::Config,
                "legacy merged.json is corrupt ({error:#}); archived at {} without creating a bundle",
                archived.display()
            );
            return Ok(MigrationOutcome {
                migrated: false,
                archived: Some(archived),
            });
        }
    };

    let snapshot = profiles.latest_arc();
    let Some(plan) = plan_migration(snapshot.items.as_deref().unwrap_or_default(), &legacy_uids) else {
        let archived = archive_legacy(&legacy_path, "migrated").await?;
        logging!(
            warn,
            Type::Config,
            "legacy merged.json has fewer than {MIN_MEMBERS} usable members; archived at {}",
            archived.display()
        );
        return Ok(MigrationOutcome {
            migrated: false,
            archived: Some(archived),
        });
    };

    let generated = match generate(&snapshot, &plan.members).await {
        Ok(generated) => generated,
        Err(error) => {
            logging!(
                error,
                Type::Config,
                "failed to migrate the legacy merge selection into a bundle: {error:#}"
            );
            return Ok(MigrationOutcome::default());
        }
    };
    write(&generated).await?;

    if let Err(error) = profiles
        .with_data_modify(|mut candidate| async move {
            upsert_item(&mut candidate);
            candidate.current = Some(BUNDLE_UID.into());
            candidate.save_file().await?;
            Ok((candidate, ()))
        })
        .await
    {
        logging!(
            error,
            Type::Config,
            "failed to persist the migrated bundle item: {error:#}"
        );
        return Ok(MigrationOutcome::default());
    }

    multi_merge::publish_conflicts(generated.conflicts.clone());
    let archived = archive_legacy(&legacy_path, "migrated").await?;
    if !plan.dropped.is_empty() {
        logging!(
            warn,
            Type::Config,
            "legacy merged.json dropped {} unavailable member(s): {}",
            plan.dropped.len(),
            plan.dropped.join(", ")
        );
    }
    logging!(
        info,
        Type::Config,
        "migrated {} merged profiles into a profile bundle",
        plan.members.len()
    );
    Ok(MigrationOutcome {
        migrated: true,
        archived: Some(archived),
    })
}
