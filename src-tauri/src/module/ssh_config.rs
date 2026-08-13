use std::collections::HashSet;

use crate::utils::dirs;
use anyhow::{Context as _, Result};
use serde_yaml_ng::{Mapping, Value};
use smartstring::alias::String;
use tokio::fs;

/// 从 profile YAML 中提取 `ssh-config` key，写入托管目录，返回 (剥离后的 YAML, 是否有SSH配置)。
/// 这是 prfitem 导入流程的唯一集成点。
pub async fn extract_and_sync(yaml_str: &str, uid: &str) -> Result<(String, bool)> {
    let mut yaml: Mapping = serde_yaml_ng::from_str(yaml_str).context("invalid yaml")?;

    let ssh_text: Option<String> = yaml.remove("ssh-config").and_then(|v| v.as_str().map(|s| s.into()));

    let has_ssh = ssh_text.is_some();

    if let Some(ref text) = ssh_text {
        let duplicates = check_internal_duplicates(text);
        if !duplicates.is_empty() {
            log::warn!(
                "SSH config duplicate Host entries ({}); only the first definition will be used by SSH",
                duplicates.join(", ")
            );
        }
        let host_names = parse_host_names(text);
        match check_collisions(uid, &host_names).await {
            Ok(conflicts) => {
                for c in &conflicts {
                    log::warn!(
                        "SSH Host '{}' in profile '{}' collides with profile '{}'",
                        c.host_name,
                        uid,
                        c.existing_profile_uid
                    );
                }
            }
            Err(e) => log::warn!("SSH Host collision check failed: {e}"),
        }
        write_ssh_config(text, uid).await?;
        log::info!("SSH config synced for profile '{uid}' ({} Hosts)", host_names.len());
    } else {
        // profile no longer provides SSH config — clean up stale file
        let _ = remove_ssh_config(uid).await;
    }

    let stripped = serde_yaml_ng::to_string(&yaml).context("failed to serialize clash config")?;
    Ok((stripped.into(), has_ssh))
}

/// 注入 `ssh-config-path` 到 config 中所有 `type: ssh` 的代理。
/// 这是 enhance 管道的唯一集成点。
pub fn inject_paths(config: Mapping, profile_uid: &str) -> Mapping {
    let mut config = config;
    let ssh_config_path = match get_ssh_config_path(profile_uid) {
        Some(path) => path,
        None => return config,
    };

    let Some(path_str) = ssh_config_path.to_str() else {
        return config;
    };

    if let Some(proxies) = config.get_mut("proxies")
        && let Some(seq) = proxies.as_sequence_mut()
    {
        for proxy in seq.iter_mut() {
            if let Some(map) = proxy.as_mapping_mut()
                && map.get("type").and_then(|v| v.as_str()) == Some("ssh")
            {
                map.insert("ssh-config-path".into(), Value::from(path_str));
            }
        }
    }

    config
}

/// 从 SSH config 文本中提取所有 Host 名称
pub fn parse_host_names(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("Host ") {
            let rest = rest.trim();
            if !rest.is_empty() && !rest.starts_with('*') {
                for name in rest.split_whitespace() {
                    let name = name.trim();
                    if !name.is_empty() && name != "*" {
                        names.push(name.into());
                    }
                }
            }
        }
    }
    names
}

/// 检测同一 SSH config 文本内的 Host 名重复。
/// 返回重复出现的 Host 名列表。
pub fn check_internal_duplicates(text: &str) -> Vec<String> {
    let names = parse_host_names(text);
    let mut seen = HashSet::new();
    let mut duplicates = Vec::new();
    for name in names {
        if !seen.insert(name.clone()) {
            duplicates.push(name);
        }
    }
    duplicates
}

/// 冲突信息
#[derive(Debug, Clone)]
pub struct Conflict {
    pub host_name: String,
    pub existing_profile_uid: String,
}

/// 检测当前 profile 的 Host 名是否与其他 managed 文件冲突。
/// 返回冲突列表（同名 Host 已存在于其他 profile 的 SSH config 中）。
pub async fn check_collisions(current_uid: &str, names: &[String]) -> Result<Vec<Conflict>> {
    let ssh_configs_dir = dirs::app_home_dir()?.join("ssh-configs");
    if !ssh_configs_dir.exists() {
        return Ok(Vec::new());
    }

    let mut conflicts = Vec::new();
    let mut entries = fs::read_dir(&ssh_configs_dir)
        .await
        .with_context(|| format!("failed to read ssh-configs dir: {}", ssh_configs_dir.display()))?;

    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        let file_name = path.file_stem().and_then(|n| n.to_str()).unwrap_or_default();

        // 跳过当前 profile 自己的文件
        if file_name == current_uid {
            continue;
        }

        if let Ok(content) = fs::read_to_string(&path).await {
            let existing_names: HashSet<String> = parse_host_names(&content).into_iter().collect();
            for name in names {
                if existing_names.contains(name) {
                    conflicts.push(Conflict {
                        host_name: name.clone(),
                        existing_profile_uid: file_name.into(),
                    });
                }
            }
        }
    }

    Ok(conflicts)
}

/// 写入 SSH config 到托管目录
pub async fn write_ssh_config(ssh_text: &str, uid: &str) -> Result<()> {
    let ssh_configs_dir = dirs::app_home_dir()?.join("ssh-configs");
    fs::create_dir_all(&ssh_configs_dir)
        .await
        .with_context(|| format!("failed to create ssh-configs dir: {}", ssh_configs_dir.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mut perms = fs::metadata(&ssh_configs_dir).await?.permissions();
        perms.set_mode(0o700);
        fs::set_permissions(&ssh_configs_dir, perms).await?;
    }

    let path = ssh_configs_dir.join(format!("{uid}.conf"));
    fs::write(&path, ssh_text)
        .await
        .with_context(|| format!("failed to write SSH config: {}", path.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mut perms = fs::metadata(&path).await?.permissions();
        perms.set_mode(0o600);
        fs::set_permissions(&path, perms).await?;
    }

    let path_str = format!("{}", path.display());
    log::info!("SSH config written: {path_str}");
    Ok(())
}

/// 获取指定 profile 的 SSH config 文件路径（若存在）
pub fn get_ssh_config_path(uid: &str) -> Option<std::path::PathBuf> {
    let path = dirs::app_home_dir()
        .ok()?
        .join("ssh-configs")
        .join(format!("{uid}.conf"));
    if path.exists() { Some(path) } else { None }
}

/// 删除指定 profile 的 SSH config
pub async fn remove_ssh_config(uid: &str) -> Result<()> {
    let path = dirs::app_home_dir()?.join("ssh-configs").join(format!("{uid}.conf"));
    if path.exists() {
        let display_path = format!("{}", path.display());
        fs::remove_file(&path)
            .await
            .with_context(|| format!("failed to remove SSH config: {display_path}"))?;
        log::info!("SSH config removed: {display_path}");
    }
    Ok(())
}

/// 清理无对应 profile 的孤立 SSH config 文件
pub async fn cleanup_orphaned(valid_uids: &HashSet<String>) -> Result<()> {
    let ssh_configs_dir = dirs::app_home_dir()?.join("ssh-configs");
    if !ssh_configs_dir.exists() {
        return Ok(());
    }

    let mut entries = fs::read_dir(&ssh_configs_dir)
        .await
        .with_context(|| format!("failed to read ssh-configs dir: {}", ssh_configs_dir.display()))?;

    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if let Some(stem) = path.file_stem().and_then(|n| n.to_str())
            && !valid_uids.contains(stem)
        {
            let cleanup_path = format!("{}", path.display());
            fs::remove_file(&path).await?;
            log::info!("Orphaned SSH config cleaned up: {cleanup_path}");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_host_names() {
        let text = "Host my-vps\n  HostName 1.2.3.4\nHost jump-cn internal\n  User admin\n";
        let names = parse_host_names(text);
        assert!(names.contains(&"my-vps".into()));
        assert!(names.contains(&"jump-cn".into()));
        assert!(names.contains(&"internal".into()));
        assert!(!names.contains(&"*".into()));
    }

    #[test]
    fn test_parse_host_names_ignores_wildcard() {
        let text = "Host *.internal\n  ProxyJump gw\nHost *\n  ConnectTimeout 10\n";
        let names = parse_host_names(text);
        assert!(names.is_empty());
    }

    #[test]
    fn test_internal_duplicates_detected() {
        let text = "Host my-vps\n  HostName 1.2.3.4\nHost my-vps\n  HostName 5.6.7.8\n";
        let dups = check_internal_duplicates(text);
        assert_eq!(dups.len(), 1);
        assert_eq!(dups[0], "my-vps");
    }

    #[test]
    fn test_internal_duplicates_none_for_unique() {
        let text = "Host a\n  HostName 1.2.3.4\nHost b\n  HostName 5.6.7.8\n";
        let dups = check_internal_duplicates(text);
        assert!(dups.is_empty());
    }
}
