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

#[derive(Debug, Default, Serialize, Deserialize)]
struct MergedStore {
    #[serde(default)]
    uids: Vec<String>,
}

fn store_path() -> Result<PathBuf> {
    Ok(dirs::app_home_dir()?.join(MERGED_FILE))
}

/// 读取合并列表；文件不存在时返回空列表。
pub async fn load() -> Vec<String> {
    match read_store().await {
        Ok(Some(uids)) => uids,
        Ok(None) => Vec::new(),
        Err(error) => {
            logging!(warn, Type::Config, "failed to read {MERGED_FILE}: {error:#}");
            Vec::new()
        }
    }
}

pub async fn save(uids: &[String]) -> Result<()> {
    let path = store_path()?;
    let body = serde_json::to_string_pretty(&MergedStore { uids: uids.to_vec() })?;
    crate::utils::help::save_yaml_str(&path, &body).await?;
    logging!(debug, Type::Config, "merged profile uids saved: {}", uids.len());
    Ok(())
}

pub async fn clear() -> Result<()> {
    save(&[]).await
}

/// 剔除指定 uid（删除 profile 后调用）。返回是否发生变化。
fn prune_uids(uids: &mut Vec<String>, removed: &str) -> bool {
    let before = uids.len();
    uids.retain(|uid| uid != removed);
    uids.len() != before
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

#[cfg(test)]
mod tests {
    use super::*;

    fn uids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).into()).collect()
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
