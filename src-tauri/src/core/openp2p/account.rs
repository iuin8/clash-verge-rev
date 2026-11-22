use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};

use crate::utils::dirs;

pub fn openp2p_root() -> Result<PathBuf> {
    Ok(dirs::app_home_dir()?.join("openp2p"))
}

pub fn accounts_root() -> Result<PathBuf> {
    Ok(openp2p_root()?.join("accounts"))
}

pub fn tokens_root() -> Result<PathBuf> {
    Ok(openp2p_root()?.join("tokens"))
}

pub fn keys_root() -> Result<PathBuf> {
    Ok(openp2p_root()?.join("keys"))
}

pub fn artifacts_manifest_path() -> Result<PathBuf> {
    Ok(openp2p_root()?.join("artifacts.yaml"))
}

pub fn account_paths(account_id: &str) -> Result<AccountPaths> {
    if !crate::core::openp2p::schema::valid_slug(account_id, 32) {
        bail!("invalid openp2p account id");
    }
    let workdir = accounts_root()?.join(account_id);
    let log_dir = workdir.join("log");
    Ok(AccountPaths {
        workdir: workdir.clone(),
        config_path: workdir.join("config.json"),
        child_path: workdir.join("child.json"),
        log_dir: log_dir.clone(),
        log_path: log_dir.join("openp2p.log"),
    })
}

#[derive(Debug, Clone)]
pub struct AccountPaths {
    pub workdir: PathBuf,
    pub config_path: PathBuf,
    pub child_path: PathBuf,
    pub log_dir: PathBuf,
    pub log_path: PathBuf,
}

impl AccountPaths {
    pub fn ensure(&self) -> Result<()> {
        create_private_dir(&self.workdir)?;
        create_private_dir(&self.log_dir)?;
        Ok(())
    }
}

pub fn token_path(token_ref: &str) -> Result<PathBuf> {
    ref_path(&tokens_root()?, token_ref, "token_ref")
}

pub fn key_path(key_ref: &str) -> Result<PathBuf> {
    ref_path(&keys_root()?, key_ref, "private_key_ref")
}

pub fn relative_key_path(key_ref: &str) -> Result<std::string::String> {
    // mihomo resolves private-key against the core data directory and runtime_bundle copies
    // files below it, so the generated node must carry this relative path.
    let _ = key_path(key_ref)?;
    Ok(format!("openp2p/keys/{key_ref}"))
}

fn ref_path(root: &Path, reference: &str, field: &str) -> Result<PathBuf> {
    if !crate::core::openp2p::schema::valid_ref(reference) {
        bail!("invalid {field}");
    }
    Ok(root.join(reference))
}

pub fn create_private_dir(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path).with_context(|| format!("failed to create {}", path.display()))?;
    let metadata = std::fs::symlink_metadata(path).with_context(|| format!("failed to inspect {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!("{} must be a real directory, not a symlink or file", path.display());
    }
    set_mode(path, 0o700)?;
    // Nested refs (o2p/home) create ancestors; keep every openp2p-owned ancestor private.
    let Ok(root) = openp2p_root() else {
        return Ok(());
    };
    if !path.starts_with(&root) {
        return Ok(());
    }
    let mut current = path.parent();
    while let Some(parent) = current {
        if !parent.starts_with(&root) {
            break;
        }
        let metadata =
            std::fs::symlink_metadata(parent).with_context(|| format!("failed to inspect {}", parent.display()))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            bail!("{} must be a real directory, not a symlink or file", parent.display());
        }
        set_mode(parent, 0o700)?;
        if parent == root {
            break;
        }
        current = parent.parent();
    }
    if root.is_dir() {
        set_mode(&root, 0o700)?;
    }
    Ok(())
}

pub fn ensure_private_file(path: &Path) -> Result<()> {
    if path.exists() {
        set_mode(path, 0o600)?;
    }
    Ok(())
}

pub fn set_mode(path: &Path, mode: u32) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let current = std::fs::metadata(path)
            .with_context(|| format!("failed to inspect {}", path.display()))?
            .permissions()
            .mode()
            & 0o777;
        if current != mode {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
                .with_context(|| format!("failed to chmod {}", path.display()))?;
        }
    }
    #[cfg(not(unix))]
    let _ = (path, mode);
    Ok(())
}

pub fn token_present(token_ref: &str) -> bool {
    token_path(token_ref)
        .ok()
        .is_some_and(|path| path.is_file() && !std::fs::read(&path).unwrap_or_default().is_empty())
}

pub fn key_present(key_ref: &str) -> bool {
    let Some(path) = key_path(key_ref).ok().filter(|path| path.is_file()) else {
        return false;
    };
    let _ = ensure_private_file(&path);
    true
}

pub fn account_key_present(config: &crate::core::openp2p::schema::OpenP2PConfig, account_id: &str) -> bool {
    let proxies: Vec<&crate::core::openp2p::schema::ProxySpec> = config.proxies_for(account_id).collect();
    proxies.is_empty() || proxies.iter().all(|proxy| key_present(&proxy.private_key_ref))
}

pub fn enforce_permissions(paths: &AccountPaths) {
    let _ = create_private_dir(&paths.workdir);
    let _ = create_private_dir(&paths.log_dir);
    let _ = ensure_private_file(&paths.config_path);
    let _ = ensure_private_file(&paths.child_path);
    let _ = ensure_private_file(&paths.log_path);
    if let Ok(entries) = std::fs::read_dir(&paths.log_dir) {
        for entry in entries.flatten() {
            if entry.path().is_file() {
                let _ = ensure_private_file(&entry.path());
            }
        }
    }
}

pub fn atomic_write(path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("refusing to write a path without a parent"))?;
    create_private_dir(parent)?;
    replace_file(path, bytes, mode)
}

/// Atomic replace for profile files: never changes the permission of the profiles directory.
pub fn atomic_write_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("refusing to write a path without a parent"))?;
    std::fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    const DEFAULT_PROFILE_MODE: u32 = 0o644;
    let mode = std::fs::metadata(path)
        .map(|metadata| {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                metadata.permissions().mode() & 0o777
            }
            #[cfg(not(unix))]
            {
                let _ = metadata;
                DEFAULT_PROFILE_MODE
            }
        })
        .unwrap_or(DEFAULT_PROFILE_MODE);
    replace_file(path, bytes, mode)
}

fn replace_file(path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("refusing to write a path without a parent"))?;
    let temporary = parent.join(format!(
        ".{}.tmp-{}-{}",
        path.file_name().and_then(|name| name.to_str()).unwrap_or("openp2p"),
        std::process::id(),
        chrono::Local::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    let result = (|| -> Result<()> {
        {
            use std::io::Write as _;
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt as _;
                options.mode(mode);
            }
            let mut file = options
                .open(&temporary)
                .with_context(|| format!("failed to create {}", temporary.display()))?;
            file.write_all(bytes)
                .with_context(|| format!("failed to write {}", temporary.display()))?;
            file.sync_all()
                .with_context(|| format!("failed to sync {}", temporary.display()))?;
        }
        set_mode(&temporary, mode)?;
        std::fs::rename(&temporary, path).with_context(|| format!("failed to replace {}", path.display()))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}
