//! FORK: 多订阅合并（multi-profile merge）的用户选择。
//!
//! 选择结果单独存放 `merged.json`，而不是给上游的 `IProfiles` 加字段：上游结构没有
//! `deny_unknown_fields`，一旦某次同步退回纯上游结构，这个键会被静默丢弃，并由下一次
//! `save_file()` 从磁盘抹掉——用户的多订阅选择会无声消失。

use anyhow::{Context as _, Result};
use clash_verge_logging::{Type, logging};
use serde::{Deserialize, Serialize};
use smartstring::alias::String;
use std::path::PathBuf;
use tokio::fs;

use crate::utils::dirs;

const MERGED_FILE: &str = "merged.json";
const PROFILES_FILE: &str = "profiles.yaml";
/// 历史版本把合并列表写在上游 `profiles.yaml` 的这个键里。
const LEGACY_KEY: &str = "merged";

#[derive(Debug, Default, Serialize, Deserialize)]
struct MergedStore {
    #[serde(default)]
    uids: Vec<String>,
}

fn store_path() -> Result<PathBuf> {
    Ok(dirs::app_home_dir()?.join(MERGED_FILE))
}

/// 读取合并列表；文件不存在时尝试从历史 `profiles.yaml` 做一次性迁移。
pub async fn load() -> Vec<String> {
    match read_store().await {
        Ok(Some(uids)) => uids,
        Ok(None) => migrate_legacy().await,
        Err(error) => {
            logging!(warn, Type::Config, "failed to read {MERGED_FILE}: {error:#}");
            Vec::new()
        }
    }
}

pub async fn save(uids: &[String]) -> Result<()> {
    let path = store_path()?;
    let body = serde_json::to_string_pretty(&MergedStore { uids: uids.to_vec() })?;
    fs::write(&path, body)
        .await
        .with_context(|| format!("failed to write {}", path.display()))?;
    logging!(debug, Type::Config, "merged profile uids saved: {}", uids.len());
    Ok(())
}

pub async fn clear() -> Result<()> {
    save(&[]).await
}

async fn read_store() -> Result<Option<Vec<String>>> {
    let path = store_path()?;
    if !fs::try_exists(&path).await.unwrap_or(false) {
        return Ok(None);
    }
    let body = fs::read_to_string(&path)
        .await
        .with_context(|| format!("failed to read {}", path.display()))?;
    let store: MergedStore = serde_json::from_str(&body).with_context(|| format!("invalid {}", path.display()))?;
    Ok(Some(store.uids))
}

/// 迁移历史键。上游 `IProfiles` 会忽略未知键，所以旧版本写入的 `merged` 可能仍留在
/// `profiles.yaml` 里；迁移后不再回写该键，下一次 `save_file()` 自然会清掉它。
async fn migrate_legacy() -> Vec<String> {
    let Ok(path) = dirs::app_home_dir().map(|dir| dir.join(PROFILES_FILE)) else {
        return Vec::new();
    };
    if !fs::try_exists(&path).await.unwrap_or(false) {
        return Vec::new();
    }
    let Ok(body) = fs::read_to_string(&path).await else {
        return Vec::new();
    };

    let uids: Vec<String> = serde_yaml_ng::from_str::<serde_yaml_ng::Mapping>(&body)
        .ok()
        .and_then(|mapping| mapping.get(LEGACY_KEY).cloned())
        .and_then(|value| value.as_sequence().cloned())
        .map(|sequence| {
            sequence
                .iter()
                .filter_map(|value| value.as_str())
                .map(Into::into)
                .collect()
        })
        .unwrap_or_default();

    if uids.is_empty() {
        return Vec::new();
    }
    match save(&uids).await {
        Ok(()) => {
            logging!(
                info,
                Type::Config,
                "migrated {} merged profile uids out of {PROFILES_FILE}",
                uids.len()
            );
            uids
        }
        Err(error) => {
            logging!(warn, Type::Config, "failed to migrate merged uids: {error:#}");
            Vec::new()
        }
    }
}
