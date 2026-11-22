use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt as _;
use tokio::process::{Child, Command};
use tokio::task::JoinHandle;

use crate::core::openp2p::account::{self, AccountPaths};
use crate::core::openp2p::schema::AccountSpec;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChildState {
    Starting,
    Running,
    Stopping,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChildRecord {
    pub account_id: std::string::String,
    pub state: ChildState,
    pub pid: u32,
    pub pgid: i32,
    pub start_time: u64,
    pub uid: u32,
    pub spawned_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessIdentity {
    pub start_time: u64,
    pub uid: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reconcile {
    Missing,
    Killed,
    Stale,
    /// A crash landed between the ledger write and the spawn update; never kill what cannot be verified.
    Indeterminate,
}

pub fn sidecar_binary() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("failed to resolve the current executable")?;
    let dir = exe
        .parent()
        .ok_or_else(|| anyhow::anyhow!("the current executable has no parent directory"))?;
    // cargo test places the binary in target/debug/deps; Tauri copies sidecars next to target/debug.
    let dir = if dir.ends_with("deps") {
        dir.parent().unwrap_or(dir)
    } else {
        dir
    };
    let name = if cfg!(windows) { "openp2p.exe" } else { "openp2p" };
    Ok(dir.join(name))
}

pub fn account_args(account: &AccountSpec, workdir: &Path) -> Vec<std::string::String> {
    let mut args: Vec<std::string::String> = vec![
        "-workdir".into(),
        workdir.to_string_lossy().into_owned(),
        "-node".into(),
        account.node.clone(),
    ];
    if let Some(gateway) = account.gateway.as_ref() {
        args.push("-serverhost".into());
        args.push(gateway.host.clone());
        args.push("-serverport".into());
        args.push(gateway.port.to_string());
    }
    args.push("-publicipport".into());
    args.push(account.public_ip_port.to_string());
    args.push("-interface".into());
    args.push(account.interface().into());
    if !account.dns.is_empty() {
        args.push("-dns".into());
        args.push(account.dns.join(","));
    }
    args.extend(["-nv", "-loglevel", "1", "-noupdate"].map(std::string::String::from));
    args
}

pub struct SpawnSpec {
    pub account: AccountSpec,
    pub args: Vec<std::string::String>,
    pub workdir: PathBuf,
    pub child_path: PathBuf,
    pub binary: PathBuf,
}

impl SpawnSpec {
    pub fn for_account(account: AccountSpec) -> Result<Self> {
        let paths: AccountPaths = account::account_paths(&account.id)?;
        let args = account_args(&account, &paths.workdir);
        let binary = sidecar_binary()?;
        if !binary.is_file() {
            bail!("the packaged openp2p sidecar binary is missing at {}", binary.display());
        }
        Ok(Self {
            account,
            args,
            workdir: paths.workdir,
            child_path: paths.child_path,
            binary,
        })
    }
}

pub struct RunningChild {
    pub pid: u32,
    child: Child,
    stdout_drain: Option<JoinHandle<()>>,
    stderr_drain: Option<JoinHandle<()>>,
    ledger_path: PathBuf,
}

impl RunningChild {
    pub const fn pgid(&self) -> i32 {
        self.pid as i32
    }

    pub async fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        self.child.wait().await
    }

    /// Kills the whole process group, reaps it, and waits for the pipe readers.
    pub async fn terminate(&mut self) {
        update_ledger_state(&self.ledger_path, ChildState::Stopping);
        kill_group(self.pgid());
        let _ = tokio::time::timeout(Duration::from_secs(3), self.child.wait()).await;
        let _ = self.child.start_kill();
        let _ = tokio::time::timeout(Duration::from_secs(1), self.child.wait()).await;
        self.finish_drains().await;
        let _ = std::fs::remove_file(&self.ledger_path);
    }

    pub async fn finish_after_exit(&mut self) {
        self.finish_drains().await;
        let _ = std::fs::remove_file(&self.ledger_path);
    }

    async fn finish_drains(&mut self) {
        if let Some(drain) = self.stdout_drain.take() {
            let _ = tokio::time::timeout(Duration::from_millis(500), drain).await;
        }
        if let Some(drain) = self.stderr_drain.take() {
            let _ = tokio::time::timeout(Duration::from_millis(500), drain).await;
        }
    }
}

fn free_tcp_port(address: std::net::SocketAddr) -> Result<()> {
    use socket2::{Domain, Protocol, Socket, Type};
    let socket = Socket::new(Domain::for_address(address), Type::STREAM, Some(Protocol::TCP))?;
    socket.set_reuse_address(true)?;
    socket.bind(&address.into())?;
    socket.listen(1)?;
    Ok(())
}

fn free_udp_port(address: std::net::SocketAddr) -> Result<()> {
    use socket2::{Domain, Protocol, Socket, Type};
    let socket = Socket::new(Domain::for_address(address), Type::DGRAM, Some(Protocol::UDP))?;
    socket.set_reuse_address(true)?;
    socket.bind(&address.into())?;
    Ok(())
}

fn preflight_ports(account: &AccountSpec) -> Result<()> {
    let public = account.public_ip_port;
    free_tcp_port(([0, 0, 0, 0], public).into())
        .map_err(|error| anyhow::anyhow!("public TCP port {public} is unavailable: {error:#}"))?;
    free_udp_port(([0, 0, 0, 0], public).into())
        .map_err(|error| anyhow::anyhow!("public UDP port {public} is unavailable: {error:#}"))?;
    for app in account.apps.iter().filter(|app| app.enabled.is_enabled()) {
        let address = ([127, 0, 0, 1], app.src_port).into();
        match app.protocol() {
            crate::core::openp2p::schema::AppProtocol::Tcp => free_tcp_port(address)
                .map_err(|error| anyhow::anyhow!("forwarded TCP port {} is unavailable: {error:#}", app.src_port))?,
            crate::core::openp2p::schema::AppProtocol::Udp => free_udp_port(address)
                .map_err(|error| anyhow::anyhow!("forwarded UDP port {} is unavailable: {error:#}", app.src_port))?,
        }
    }
    Ok(())
}

pub async fn spawn(spec: SpawnSpec) -> Result<RunningChild> {
    preflight_ports(&spec.account)?;
    spec.workdir.parent().map(account::create_private_dir).transpose()?;
    account::create_private_dir(&spec.workdir)?;
    let starting = ChildRecord {
        account_id: spec.account.id.clone(),
        state: ChildState::Starting,
        pid: 0,
        pgid: 0,
        start_time: 0,
        uid: current_uid(),
        spawned_at: chrono::Local::now().timestamp(),
    };
    write_ledger(&spec.child_path, &starting)?;

    validate_workdir_ownership(&AccountPaths {
        workdir: spec.workdir.clone(),
        config_path: spec.workdir.join("config.json"),
        child_path: spec.child_path.clone(),
        log_dir: spec.workdir.join("log"),
        log_path: spec.workdir.join("log/openp2p.log"),
    })?;
    let mut command = Command::new(&spec.binary);
    command
        .args(&spec.args)
        .current_dir(&spec.workdir)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in child_environment() {
        command.env(key, value);
    }
    #[cfg(unix)]
    command.process_group(0);

    // openp2p creates config.json and its log with mode 0644; the inherited umask is the only
    // portable way to make those 0600/0700 without patching the sidecar.
    #[cfg(unix)]
    let previous_mask = unsafe { libc::umask(0o077) };
    let spawned = command.spawn();
    #[cfg(unix)]
    unsafe {
        libc::umask(previous_mask)
    };
    let mut child = match spawned {
        Ok(child) => child,
        Err(error) => {
            let _ = std::fs::remove_file(&spec.child_path);
            return Err(
                anyhow::Error::new(error).context(format!("failed to start openp2p sidecar {}", spec.binary.display()))
            );
        }
    };
    let pid = child
        .id()
        .ok_or_else(|| anyhow::anyhow!("the openp2p sidecar did not report a pid"))?;
    let identity = match process_identity(pid) {
        Some(identity) => identity,
        None => {
            let status = child.try_wait().ok().flatten().and_then(|status| status.code());
            let _ = child.start_kill();
            let _ = child.wait().await;
            let _ = std::fs::remove_file(&spec.child_path);
            return Err(anyhow::anyhow!(
                "the openp2p sidecar exited before it could be tracked (exit code {status:?})"
            ));
        }
    };
    let running = ChildRecord {
        account_id: spec.account.id.clone(),
        state: ChildState::Running,
        pid,
        pgid: pid as i32,
        start_time: identity.start_time,
        uid: identity.uid,
        spawned_at: starting.spawned_at,
    };
    write_ledger(&spec.child_path, &running)?;

    let stdout_drain = child.stdout.take().map(|stdout| tokio::spawn(drain(stdout)));
    let stderr_drain = child.stderr.take().map(|stderr| tokio::spawn(drain(stderr)));
    Ok(RunningChild {
        pid,
        child,
        stdout_drain,
        stderr_drain,
        ledger_path: spec.child_path,
    })
}

/// The complete child environment: nothing inherited from CVR, no token in env.
pub fn child_environment() -> Vec<(&'static str, std::ffi::OsString)> {
    let mut environment = Vec::new();
    for key in ["HOME", "PATH", "TMPDIR", "LANG"] {
        if let Some(value) = std::env::var_os(key) {
            environment.push((key, value));
        }
    }
    if std::env::var_os("PATH").is_none() {
        environment.push(("PATH", std::ffi::OsString::from("/usr/bin:/bin:/usr/sbin:/sbin")));
    }
    environment
}

async fn drain<R>(mut reader: R)
where
    R: tokio::io::AsyncRead + Unpin,
{
    // The sidecar writes its own file (with -nv); keeping the pipes moving is what matters here.
    let mut buffer = [0u8; 8192];
    loop {
        match reader.read(&mut buffer).await {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
    }
}

fn write_ledger(path: &Path, record: &ChildRecord) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(record)?;
    account::atomic_write(path, &bytes, 0o600)
}

fn update_ledger_state(path: &Path, state: ChildState) {
    let Ok(raw) = std::fs::read(path) else {
        return;
    };
    let Ok(mut record) = serde_json::from_slice::<ChildRecord>(&raw) else {
        return;
    };
    record.state = state;
    let _ = write_ledger(path, &record);
}

fn read_ledger(path: &Path) -> Option<ChildRecord> {
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub fn kill_group(pgid: i32) {
    if pgid <= 1 {
        return;
    }
    #[cfg(unix)]
    unsafe {
        let result = libc::killpg(pgid, libc::SIGKILL);
        if result != 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                log::warn!("failed to kill openp2p process group: {error}");
            }
        }
    }
}

pub fn reconcile(account_paths: &AccountPaths) -> Reconcile {
    let Some(record) = read_ledger(&account_paths.child_path) else {
        return Reconcile::Missing;
    };
    if record.state == ChildState::Starting && record.pid == 0 {
        return Reconcile::Indeterminate;
    }
    let Some(identity) = process_identity(record.pid) else {
        let _ = std::fs::remove_file(&account_paths.child_path);
        return Reconcile::Stale;
    };
    if identity.start_time != record.start_time || identity.uid != record.uid {
        let _ = std::fs::remove_file(&account_paths.child_path);
        return Reconcile::Stale;
    }
    kill_group(record.pgid);
    let deadline = Instant::now() + Duration::from_secs(2);
    while process_identity(record.pid).is_some() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = std::fs::remove_file(&account_paths.child_path);
    Reconcile::Killed
}

/// Last-resort cleanup for RunEvent::Exit when async shutdown could not run.
pub fn kill_tracked_children() -> usize {
    let Ok(root) = account::accounts_root() else {
        return 0;
    };
    let Ok(entries) = std::fs::read_dir(root) else {
        return 0;
    };
    let mut killed = 0;
    for entry in entries.flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        let paths = AccountPaths {
            workdir: entry.path(),
            config_path: entry.path().join("config.json"),
            child_path: entry.path().join("child.json"),
            log_dir: entry.path().join("log"),
            log_path: entry.path().join("log/openp2p.log"),
        };
        if reconcile(&paths) == Reconcile::Killed {
            killed += 1;
        }
    }
    killed
}

pub fn current_uid() -> u32 {
    #[cfg(unix)]
    unsafe {
        libc::geteuid()
    }
    #[cfg(not(unix))]
    {
        0
    }
}

#[cfg(target_os = "macos")]
pub fn process_identity(pid: u32) -> Option<ProcessIdentity> {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>();
    let read = unsafe {
        libc::proc_pidinfo(
            pid as i32,
            libc::PROC_PIDTBSDINFO,
            0,
            std::ptr::from_mut(&mut info).cast(),
            size as i32,
        )
    };
    if read != size as i32 {
        return None;
    }
    Some(ProcessIdentity {
        start_time: info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec,
        uid: info.pbi_uid,
    })
}

#[cfg(target_os = "linux")]
pub fn process_identity(pid: u32) -> Option<ProcessIdentity> {
    use std::os::unix::fs::MetadataExt as _;
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let tail = &stat[stat.rfind(')')? + 1..];
    let start_time = tail.split_whitespace().nth(19)?.parse().ok()?;
    let uid = std::fs::metadata(format!("/proc/{pid}")).ok()?.uid();
    Some(ProcessIdentity { start_time, uid })
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn process_identity(pid: u32) -> Option<ProcessIdentity> {
    let _ = pid;
    None
}

pub fn validate_workdir_ownership(paths: &AccountPaths) -> Result<()> {
    let metadata = std::fs::symlink_metadata(&paths.workdir)
        .with_context(|| format!("account directory is unavailable: {}", paths.workdir.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!("account directory must be a real directory, not a symlink");
    }
    Ok(())
}
#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::core::openp2p::health::LogTail;
    use crate::core::openp2p::schema::fixtures::sample_config;

    fn scratch(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "cvr-openp2p-process-{name}-{}-{}",
            std::process::id(),
            chrono::Local::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("scratch");
        path
    }

    fn stub_account(id: &str) -> AccountSpec {
        let mut account = sample_config().accounts.remove(0);
        account.id = id.into();
        account.node = "stub-node-01".into();
        // Let the OS choose probe ports so parallel tests never collide.
        account.public_ip_port = 0;
        account.apps[0].src_port = 0;
        account
    }

    #[test]
    fn argv_never_carries_a_token_secret_or_daemon_flag() {
        let account = sample_config().accounts.remove(0);
        let args = account_args(&account, Path::new("/tmp/openp2p-account"));
        assert!(args.iter().any(|arg| arg == "-workdir"));
        assert!(args.iter().any(|arg| arg == "-interface"));
        assert!(args.iter().any(|arg| arg == "-dns"));
        assert!(args.ends_with(&["-nv".into(), "-loglevel".into(), "1".into(), "-noupdate".into()]));
        assert!(!args.iter().any(|arg| arg == "-d" || arg == "-token"));
        assert!(!args.iter().any(|arg| arg.contains("PRIVATE KEY")));
        assert!(!args.iter().any(|arg| arg.contains("123456789")));
    }

    #[test]
    fn child_environment_is_limited_to_the_allowlist() {
        for (key, _) in child_environment() {
            assert!(
                matches!(key, "HOME" | "PATH" | "TMPDIR" | "LANG"),
                "unexpected child variable {key}"
            );
        }
    }

    #[tokio::test]
    async fn a_stub_sidecar_log_is_tailed_and_the_group_is_killed() {
        let root = scratch("stub");
        let script = root.join("stub.sh");
        std::fs::write(
            &script,
            "#!/bin/sh\nmkdir -p log\nprintf '2026/01/01 00:00:00 INFO login ok. user=x, node=y\n2026/01/01 00:00:01 INFO appid:1 LISTEN ON PORT tcp:21001 START\n' > log/openp2p.log\nsleep 30 &\necho $! > sleeper.pid\nsleep 30\n",
        )
        .expect("script");
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("chmod");

        let workdir = root.join("account");
        let account = stub_account("stubacct");
        let spec = SpawnSpec {
            account,
            args: vec![script.to_string_lossy().into_owned()],
            workdir: workdir.clone(),
            child_path: workdir.join("child.json"),
            binary: PathBuf::from("/bin/sh"),
        };
        let mut child = spawn(spec).await.expect("spawn");
        let pid = child.pid;
        let log_path = workdir.join("log/openp2p.log");
        let mut tail = LogTail::new(log_path);
        let mut lines = Vec::new();
        for _ in 0..40 {
            lines.extend(tail.poll().await);
            if lines.len() >= 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(lines.iter().any(|line| line.contains("login ok")), "{lines:?}");
        assert!(lines.iter().any(|line| line.contains("LISTEN ON PORT")), "{lines:?}");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(workdir.join("child.json"))
                .expect("ledger")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
            assert_eq!(
                std::fs::metadata(&workdir).expect("workdir").permissions().mode() & 0o777,
                0o700
            );
        }

        let sleeper_path = workdir.join("sleeper.pid");
        let sleeper_deadline = Instant::now() + Duration::from_secs(3);
        while !sleeper_path.exists() && Instant::now() < sleeper_deadline {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let sleeper: i32 = std::fs::read_to_string(&sleeper_path)
            .expect("sleeper pid")
            .trim()
            .parse()
            .expect("pid");
        child.terminate().await;
        assert!(process_identity(pid).is_none(), "sidecar must be reaped");
        assert!(!workdir.join("child.json").exists(), "ledger must be cleared");
        let deadline = Instant::now() + Duration::from_secs(3);
        while unsafe { libc::kill(sleeper, 0) } == 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_ne!(
            unsafe { libc::kill(sleeper, 0) },
            0,
            "the child process group must not leave a sleeper behind"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    fn spawn_sleeper() -> std::process::Child {
        use std::os::unix::process::CommandExt as _;
        std::process::Command::new("/bin/sh")
            .args(["-c", "sleep 30"])
            .process_group(0)
            .spawn()
            .expect("sleeper")
    }

    fn wait_until_gone(pid: u32) -> bool {
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if process_identity(pid).is_none() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }

    #[test]
    fn reconciliation_kills_only_a_start_time_match() {
        let root = scratch("reconcile");
        let paths = AccountPaths {
            workdir: root.clone(),
            config_path: root.join("config.json"),
            child_path: root.join("child.json"),
            log_dir: root.join("log"),
            log_path: root.join("log/openp2p.log"),
        };
        let mut matching = spawn_sleeper();
        let identity = process_identity(matching.id()).expect("identity");
        write_ledger(
            &paths.child_path,
            &ChildRecord {
                account_id: "match".into(),
                state: ChildState::Running,
                pid: matching.id(),
                pgid: matching.id() as i32,
                start_time: identity.start_time,
                uid: identity.uid,
                spawned_at: 0,
            },
        )
        .expect("ledger");
        assert_eq!(reconcile(&paths), Reconcile::Killed);
        assert!(wait_until_gone(matching.id()));
        let _ = matching.wait();
        assert!(!paths.child_path.exists());

        let mut stale = spawn_sleeper();
        let identity = process_identity(stale.id()).expect("identity");
        write_ledger(
            &paths.child_path,
            &ChildRecord {
                account_id: "stale".into(),
                state: ChildState::Running,
                pid: stale.id(),
                pgid: stale.id() as i32,
                start_time: identity.start_time.wrapping_add(1),
                uid: identity.uid,
                spawned_at: 0,
            },
        )
        .expect("ledger");
        assert_eq!(reconcile(&paths), Reconcile::Stale);
        assert!(
            process_identity(stale.id()).is_some(),
            "a mismatched record must never kill"
        );
        let _ = stale.kill();
        let _ = stale.wait();
        assert!(!paths.child_path.exists());

        write_ledger(
            &paths.child_path,
            &ChildRecord {
                account_id: "pending".into(),
                state: ChildState::Starting,
                pid: 0,
                pgid: 0,
                start_time: 0,
                uid: current_uid(),
                spawned_at: 0,
            },
        )
        .expect("ledger");
        assert_eq!(reconcile(&paths), Reconcile::Indeterminate);
        assert!(paths.child_path.exists(), "an unverifiable record is kept");
        let _ = std::fs::remove_dir_all(root);
    }
}
