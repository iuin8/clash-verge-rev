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

/// 把 `active` 挪到 `over` 的位置（与 fork 原先 `IProfiles::reorder` 对合并数组的做法一致）。
/// 返回是否发生变化。
fn reorder_uids(uids: &mut Vec<String>, active: &str, over: &str) -> bool {
    let (Some(from), Some(to)) = (
        uids.iter().position(|uid| uid == active),
        uids.iter().position(|uid| uid == over),
    ) else {
        return false;
    };
    if from == to {
        return false;
    }
    let uid = uids.remove(from);
    uids.insert(to, uid);
    true
}

/// 剔除指定 uid（删除 profile 后调用）。返回是否发生变化。
fn prune_uids(uids: &mut Vec<String>, removed: &str) -> bool {
    let before = uids.len();
    uids.retain(|uid| uid != removed);
    uids.len() != before
}

/// profile 重排后同步合并顺序。不同步的话，前端切回订阅页会看到旧顺序，
/// 而配置生成用的是文件里的旧顺序（primary = 第一项），两边就对不上了。
pub async fn reorder(active: &str, over: &str) -> Result<()> {
    let mut uids = load().await;
    if reorder_uids(&mut uids, active, over) {
        save(&uids).await?;
    }
    Ok(())
}

/// 删除 profile 后把它的 uid 从合并列表里剔除。
pub async fn remove(uid: &str) -> Result<()> {
    let mut uids = load().await;
    if prune_uids(&mut uids, uid) {
        save(&uids).await?;
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn uids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).into()).collect()
    }

    #[test]
    fn reorder_moves_active_to_over_position() {
        let mut list = uids(&["a", "b", "c"]);
        assert!(reorder_uids(&mut list, "a", "c"));
        assert_eq!(list, uids(&["b", "c", "a"]));

        // 反向：把末位挪到首位
        let mut list = uids(&["a", "b", "c"]);
        assert!(reorder_uids(&mut list, "c", "a"));
        assert_eq!(list, uids(&["c", "a", "b"]));
    }

    #[test]
    fn reorder_ignores_unknown_or_identical_uids() {
        let mut list = uids(&["a", "b"]);
        assert!(!reorder_uids(&mut list, "a", "missing"));
        assert!(!reorder_uids(&mut list, "missing", "b"));
        assert!(!reorder_uids(&mut list, "a", "a"));
        assert_eq!(list, uids(&["a", "b"]));
    }

    #[test]
    fn prune_removes_only_the_deleted_uid() {
        let mut list = uids(&["a", "b", "c"]);
        assert!(prune_uids(&mut list, "b"));
        assert_eq!(list, uids(&["a", "c"]));

        assert!(!prune_uids(&mut list, "missing"));
        assert_eq!(list, uids(&["a", "c"]));

        assert!(prune_uids(&mut list, "a"));
        assert!(prune_uids(&mut list, "c"));
        assert!(list.is_empty());
    }
}
