use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use regex::Regex;
use tokio::io::{AsyncReadExt as _, AsyncSeekExt as _};

use crate::core::openp2p::schema::{AppProtocol, AppSpec};
use crate::core::openp2p::state::{OpenP2PAppCheck, OpenP2PCheck, OpenP2PCheckState, OpenP2PChecks};

pub const LOGIN_TIMEOUT: Duration = Duration::from_secs(60);
pub const DEGRADED_RESTART_AFTER: Duration = Duration::from_secs(60);
pub const PROBE_TIMEOUT: Duration = Duration::from_millis(500);
const MAX_ERROR_LEN: usize = 240;

static TOKEN_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(token|private[_-]?key|passphrase)\b\s*[=:]\s*\S+").expect("token redaction regex")
});
static ANSI_PATTERN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\x1b\[[0-9;]*m").expect("ANSI escape regex"));

pub fn sanitize(raw: &str) -> std::string::String {
    let clean = ANSI_PATTERN.replace_all(raw.trim(), "");
    let redacted = TOKEN_PATTERN.replace_all(&clean, "$1=***");
    let mut text: std::string::String = redacted.chars().take(MAX_ERROR_LEN).collect();
    if redacted.chars().count() > MAX_ERROR_LEN {
        text.push('…');
    }
    text
}

#[derive(Debug, Default)]
pub struct LogTail {
    path: PathBuf,
    offset: u64,
    partial: Vec<u8>,
    read_rotated: bool,
}

impl LogTail {
    pub const fn new(path: PathBuf) -> Self {
        Self {
            path,
            offset: 0,
            partial: Vec::new(),
            read_rotated: false,
        }
    }

    pub async fn poll(&mut self) -> Vec<std::string::String> {
        let bytes = match self.read_from(&self.path.clone(), self.offset).await {
            Some((bytes, new_offset)) => {
                self.offset = new_offset;
                bytes
            }
            None if self.offset == 0 && !self.read_rotated => {
                let rotated = rotated_path(&self.path);
                self.read_rotated = true;
                self.read_from(&rotated, 0)
                    .await
                    .map(|(bytes, _)| bytes)
                    .unwrap_or_default()
            }
            None => Vec::new(),
        };
        if bytes.is_empty() {
            return Vec::new();
        }
        self.partial.extend_from_slice(&bytes);
        let mut lines = Vec::new();
        while let Some(position) = self.partial.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.partial.drain(..=position).collect();
            if let Ok(text) = std::string::String::from_utf8(line[..line.len().saturating_sub(1)].to_vec())
                && !text.trim().is_empty()
            {
                lines.push(text);
            }
        }
        if self.partial.len() > 64 * 1024 {
            self.partial.clear();
        }
        lines
    }
    async fn read_from(&self, path: &Path, offset: u64) -> Option<(Vec<u8>, u64)> {
        let metadata = tokio::fs::metadata(path).await.ok()?;
        if metadata.is_dir() {
            return None;
        }
        let mut file = tokio::fs::File::open(path).await.ok()?;
        let start = if metadata.len() < offset {
            // Truncated or rotated in place: restart at the beginning.
            file.seek(std::io::SeekFrom::Start(0)).await.ok()?;
            0
        } else {
            if offset > 0 && file.seek(std::io::SeekFrom::Start(offset)).await.is_err() {
                return None;
            }
            offset
        };
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer).await.ok()?;
        let new_offset = start.saturating_add(buffer.len() as u64);
        Some((buffer, new_offset))
    }
}

fn rotated_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".0");
    PathBuf::from(name)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthFailure {
    Auth,
    Config,
    LoginTimeout,
    Degraded,
}

#[derive(Debug, Clone)]
pub struct AppHealth {
    pub name: std::string::String,
    pub protocol: AppProtocol,
    pub src_port: u16,
    pub listen: bool,
    pub reachable: Option<bool>,
    pub banner: Option<std::string::String>,
    fail_streak: u8,
}

#[derive(Debug)]
pub struct HealthMonitor {
    apps: Vec<AppHealth>,
    login: OpenP2PCheck,
    pub saw_login_ok: bool,
    pub login_errors: u8,
    pub auth_failed: bool,
    pub config_failed: bool,
    pub network_login_error: bool,
    pub last_error: Option<std::string::String>,
    started: Instant,
    online_since: Option<Instant>,
    degraded_since: Option<Instant>,
    last_probe: Option<Instant>,
}

impl HealthMonitor {
    pub fn new(apps: &[AppSpec]) -> Self {
        Self {
            apps: apps
                .iter()
                .filter(|app| app.enabled.is_enabled())
                .map(|app| AppHealth {
                    name: app.name.clone(),
                    protocol: app.protocol(),
                    src_port: app.src_port,
                    listen: false,
                    reachable: None,
                    banner: None,
                    fail_streak: 0,
                })
                .collect(),
            login: OpenP2PCheck::default(),
            saw_login_ok: false,
            login_errors: 0,
            auth_failed: false,
            config_failed: false,
            network_login_error: false,
            last_error: None,
            started: Instant::now(),
            online_since: None,
            degraded_since: None,
            last_probe: None,
        }
    }

    pub fn observe_lines(&mut self, lines: &[std::string::String]) {
        for line in lines {
            self.observe_line(line);
        }
    }

    fn observe_line(&mut self, line: &str) {
        if line.contains("login ok") {
            if !self.saw_login_ok {
                self.online_since = Some(Instant::now());
            }
            self.saw_login_ok = true;
            self.login = OpenP2PCheck {
                state: OpenP2PCheckState::Ok,
                detail: None,
            };
            self.login_errors = 0;
            self.network_login_error = false;
        } else if line.contains("login error:") {
            self.login_errors = self.login_errors.saturating_add(1);
            self.login = OpenP2PCheck {
                state: OpenP2PCheckState::Failed,
                detail: Some(sanitize(line)),
            };
            self.last_error = Some(sanitize(line));
            if self.login_errors >= 2 {
                self.auth_failed = true;
            }
        } else if line.contains("P2PNetwork login error") {
            self.network_login_error = true;
            self.last_error = Some(sanitize(line));
        }
        if let Some((protocol, port)) = parse_listen(line) {
            for app in &mut self.apps {
                if app.src_port == port && app.protocol.as_str() == protocol {
                    app.listen = true;
                }
            }
        }
        if line.contains("Access Denied") {
            self.auth_failed = true;
            self.last_error = Some(sanitize(line));
        }
        let lower = line.to_ascii_lowercase();
        if (lower.contains("configure network interface")
            || lower.contains("invalid network interface")
            || (lower.contains("workdir") && lower.contains("error")))
            && (lower.contains("error") || lower.contains("failed") || lower.contains("invalid"))
        {
            self.config_failed = true;
            self.last_error = Some(sanitize(line));
        }
        if lower.contains("error") || lower.contains("failed") || lower.contains("denied") {
            let sanitized = sanitize(line);
            if !sanitized.is_empty() {
                self.last_error = Some(sanitized);
            }
        }
    }

    pub fn check(&self) -> Option<HealthFailure> {
        if self.auth_failed {
            return Some(HealthFailure::Auth);
        }
        if self.config_failed {
            return Some(HealthFailure::Config);
        }
        if !self.saw_login_ok && self.started.elapsed() >= LOGIN_TIMEOUT {
            return Some(HealthFailure::LoginTimeout);
        }
        if self
            .degraded_since
            .is_some_and(|since| since.elapsed() >= DEGRADED_RESTART_AFTER)
        {
            return Some(HealthFailure::Degraded);
        }
        None
    }

    pub fn online_for(&self) -> Option<Duration> {
        self.online_since.map(|since| since.elapsed())
    }

    pub fn should_probe(&self) -> bool {
        let ready = self.saw_login_ok && self.apps.iter().any(|app| app.protocol == AppProtocol::Tcp);
        ready
            && self
                .last_probe
                .is_none_or(|last| last.elapsed() >= Duration::from_secs(4))
    }

    pub async fn probe(&mut self) {
        self.last_probe = Some(Instant::now());
        let mut any_failure = false;
        let mut any_recovered = false;
        for app in &mut self.apps {
            if app.protocol != AppProtocol::Tcp || !app.listen {
                continue;
            }
            let address = format!("127.0.0.1:{}", app.src_port);
            match tokio::time::timeout(PROBE_TIMEOUT, tokio::net::TcpStream::connect(&address)).await {
                Ok(Ok(mut stream)) => {
                    app.reachable = Some(true);
                    app.fail_streak = 0;
                    any_recovered = true;
                    if app.banner.is_none() {
                        let mut banner = [0u8; 64];
                        if let Ok(Ok(read)) =
                            tokio::time::timeout(Duration::from_millis(250), stream.read(&mut banner)).await
                        {
                            let text = std::string::String::from_utf8_lossy(&banner[..read]).to_string();
                            if text.starts_with("SSH-2.0-") {
                                app.banner = Some(sanitize(text.trim()));
                            }
                        }
                    }
                }
                _ => {
                    app.reachable = Some(false);
                    app.fail_streak = app.fail_streak.saturating_add(1);
                    any_failure = true;
                }
            }
        }
        if any_failure && self.degraded_since.is_none() {
            if self.apps.iter().any(|app| app.fail_streak >= 3) {
                self.degraded_since = Some(Instant::now());
                self.last_error = Some("forwarded app is not accepting loopback connections".into());
            }
        } else if any_recovered && !any_failure {
            self.degraded_since = None;
        }
    }

    pub fn checks(&self) -> OpenP2PChecks {
        OpenP2PChecks {
            login: self.login.clone(),
            apps: self
                .apps
                .iter()
                .map(|app| OpenP2PAppCheck {
                    name: app.name.clone(),
                    protocol: app.protocol.as_str().into(),
                    src_port: app.src_port,
                    listen: app.listen,
                    tcp_reachable: if app.protocol == AppProtocol::Tcp {
                        app.reachable
                    } else {
                        None
                    },
                    banner: app.banner.clone(),
                })
                .collect(),
        }
    }

    pub const fn is_degraded(&self) -> bool {
        self.degraded_since.is_some()
    }
}

fn parse_listen(line: &str) -> Option<(&str, u16)> {
    let marker = "LISTEN ON PORT ";
    let start = line.find(marker)? + marker.len();
    let mut parts = line.get(start..)?.split_whitespace();
    let token = parts.next()?;
    if parts.next()? != "START" {
        return None;
    }
    let (protocol, port) = token.split_once(':')?;
    let port: u16 = port.parse().ok()?;
    match protocol {
        "tcp" => Some(("tcp", port)),
        "udp" => Some(("udp", port)),
        _ => None,
    }
}

/// Classifies the sidecar log after a child exits, before deciding whether a retry can help.
pub fn classify_exit(log: &str) -> HealthFailureKind {
    let lower = log.to_ascii_lowercase();
    if lower.contains("access denied") || lower.matches("login error:").count() >= 2 {
        HealthFailureKind::Auth
    } else if lower.contains("configure network interface")
        || lower.contains("invalid network interface")
        || (lower.contains("workdir") && lower.contains("error"))
    {
        HealthFailureKind::Config
    } else {
        HealthFailureKind::Transient
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthFailureKind {
    Auth,
    Config,
    Transient,
}

pub const fn backoff(attempt: usize) -> Duration {
    match attempt {
        0 => Duration::from_secs(2),
        1 => Duration::from_secs(5),
        2 => Duration::from_secs(15),
        3 => Duration::from_secs(60),
        _ => Duration::from_secs(300),
    }
}

pub fn read_error_tail(path: &Path, limit: usize) -> std::string::String {
    let Ok(raw) = std::fs::read(path) else {
        return std::string::String::new();
    };
    let text = std::string::String::from_utf8_lossy(&raw);
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(limit);
    lines[start..].join("\n")
}
#[cfg(test)]
mod tests {
    use super::*;

    fn apps() -> Vec<AppSpec> {
        crate::core::openp2p::schema::fixtures::sample_config()
            .accounts
            .remove(0)
            .apps
    }

    #[test]
    fn login_listen_and_forward_checks_are_parsed_from_the_log() {
        let mut monitor = HealthMonitor::new(&apps());
        monitor.observe_lines(&[
            "2026/01/01 00:00:00 INFO login ok. user=u, node=n".into(),
            "2026/01/01 00:00:01 INFO appid:1 LISTEN ON PORT tcp:21001 START".into(),
        ]);
        assert!(monitor.saw_login_ok);
        let checks = monitor.checks();
        assert_eq!(checks.login.state, OpenP2PCheckState::Ok);
        assert!(checks.apps[0].listen);
        assert_eq!(checks.apps[0].protocol, "tcp");
    }

    #[test]
    fn log_lines_never_expose_a_token() {
        let raw = "Access Denied: token=123456789 node=abc";
        let sanitized = sanitize(raw);
        assert!(!sanitized.contains("123456789"), "{sanitized}");
        assert!(sanitized.contains("token=***"), "{sanitized}");
    }

    #[test]
    fn two_login_errors_trip_auth_failure() {
        let mut monitor = HealthMonitor::new(&apps());
        monitor.observe_lines(&["login error:1, detail:nope".into()]);
        assert!(!monitor.auth_failed);
        monitor.observe_lines(&["login error:2, detail:nope".into()]);
        assert_eq!(monitor.check(), Some(HealthFailure::Auth));
    }

    #[test]
    fn a_slow_login_is_bounded() {
        let mut monitor = HealthMonitor::new(&apps());
        monitor.started = Instant::now() - LOGIN_TIMEOUT - Duration::from_secs(1);
        assert_eq!(monitor.check(), Some(HealthFailure::LoginTimeout));
    }

    #[test]
    fn a_long_degraded_period_requests_a_restart() {
        let mut monitor = HealthMonitor::new(&apps());
        monitor.saw_login_ok = true;
        monitor.degraded_since = Some(Instant::now() - DEGRADED_RESTART_AFTER - Duration::from_secs(1));
        assert_eq!(monitor.check(), Some(HealthFailure::Degraded));
    }

    #[test]
    fn log_tail_handles_append_truncate_and_rotation() {
        let root = std::env::temp_dir().join(format!(
            "cvr-openp2p-tail-{}-{}",
            std::process::id(),
            chrono::Local::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&root).expect("scratch");
        let path = root.join("openp2p.log");
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        runtime.block_on(async {
            let mut tail = LogTail::new(path.clone());
            std::fs::write(&path, b"one\n").expect("write");
            assert_eq!(tail.poll().await, vec!["one".to_string()]);
            std::fs::write(&path, b"one\ntwo\n").expect("append");
            assert_eq!(tail.poll().await, vec!["two".to_string()]);
            std::fs::write(&path, b"reset\n").expect("truncate");
            assert_eq!(tail.poll().await, vec!["reset".to_string()]);
            std::fs::write(&path, b"reset\nnext\n").expect("append after truncate");
            assert_eq!(tail.poll().await, vec!["next".to_string()]);
        });
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn exit_classification_separates_auth_config_and_transient() {
        assert_eq!(classify_exit("Access Denied: token mismatch"), HealthFailureKind::Auth);
        assert_eq!(
            classify_exit("configure network interface \"utun9\": no such device"),
            HealthFailureKind::Config
        );
        assert_eq!(classify_exit("P2PNetwork login error"), HealthFailureKind::Transient);
    }
}
