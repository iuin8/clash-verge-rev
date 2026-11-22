use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use tokio::sync::watch;

use super::{AccountSlot, ONLINE_RESET, SHUTTING_DOWN};
use crate::core::openp2p::account::{self, AccountPaths};
use crate::core::openp2p::config as o2p_config;
use crate::core::openp2p::health::{self, HealthFailure, HealthMonitor, LogTail};
use crate::core::openp2p::process::{self, Reconcile, SpawnSpec};
use crate::core::openp2p::schema::AccountSpec;
use crate::core::openp2p::state::{OpenP2PChecks, OpenP2PPhase};
use clash_verge_logging::{Type, logging};

pub(super) async fn reconcile_account(paths: AccountPaths) -> Reconcile {
    tokio::task::spawn_blocking(move || process::reconcile(&paths))
        .await
        .unwrap_or(Reconcile::Stale)
}

pub(super) fn force_stop_slot(slot: &Arc<AccountSlot>) {
    let id = slot.spec().id;
    if let Ok(paths) = account::account_paths(&id) {
        let _ = process::reconcile(&paths);
    }
    slot.set_stopped();
}

#[allow(
    clippy::cognitive_complexity,
    reason = "the supervisor is one explicit child lifecycle state machine"
)]
pub(super) async fn supervise(slot: Arc<AccountSlot>, spec: AccountSpec, mut stop: watch::Receiver<bool>) {
    let Ok(paths) = account::account_paths(&spec.id) else {
        slot.set_terminal(OpenP2PPhase::Failed, "invalid account directory".into());
        return;
    };
    if let Err(error) = paths.ensure() {
        slot.set_terminal(OpenP2PPhase::Failed, health::sanitize(&format!("{error:#}")));
        return;
    }
    match reconcile_account(paths.clone()).await {
        Reconcile::Indeterminate => {
            slot.set_terminal(
                OpenP2PPhase::Failed,
                "an unverified previous child record exists; inspect the account child.json before starting".into(),
            );
            return;
        }
        Reconcile::Killed | Reconcile::Stale | Reconcile::Missing => {}
    }

    let mut attempt = 0usize;
    let mut consecutive_auth = 0u32;
    loop {
        if SHUTTING_DOWN.load(Ordering::Acquire) || *stop.borrow() {
            slot.set_stopped();
            return;
        }
        let token = match o2p_config::read_token(&spec.token_ref) {
            Ok(token) => token,
            Err(error) => {
                slot.set_terminal(OpenP2PPhase::TokenMissing, health::sanitize(&format!("{error:#}")));
                return;
            }
        };
        if let Err(error) = o2p_config::write_rendered_config(&spec, token) {
            slot.set_terminal(OpenP2PPhase::Failed, health::sanitize(&format!("{error:#}")));
            return;
        }
        let spawn_spec = match SpawnSpec::for_account(spec.clone()) {
            Ok(spawn) => spawn,
            Err(error) => {
                slot.set_terminal(OpenP2PPhase::ConfigFailed, health::sanitize(&format!("{error:#}")));
                return;
            }
        };
        slot.set_phase(OpenP2PPhase::Starting, None);
        let mut child = match process::spawn(spawn_spec).await {
            Ok(child) => child,
            Err(error) => {
                let log_tail = health::read_error_tail(&paths.log_path, 40);
                match health::classify_exit(&log_tail) {
                    health::HealthFailureKind::Auth => {
                        slot.set_terminal(OpenP2PPhase::AuthFailed, health::sanitize(&format!("{error:#}")));
                        return;
                    }
                    health::HealthFailureKind::Config => {
                        slot.set_terminal(OpenP2PPhase::ConfigFailed, health::sanitize(&format!("{error:#}")));
                        return;
                    }
                    health::HealthFailureKind::Transient => {}
                }
                let delay = health::backoff(attempt);
                attempt = attempt.saturating_add(1);
                slot.set_backoff(
                    attempt as u32,
                    delay.as_secs(),
                    Some(health::sanitize(&format!("{error:#}"))),
                );
                if wait_or_stop(&mut stop, delay).await || SHUTTING_DOWN.load(Ordering::Acquire) {
                    slot.set_stopped();
                    return;
                }
                continue;
            }
        };
        let pid = child.pid;
        slot.set_running(pid, OpenP2PPhase::LoginPending, OpenP2PChecks::default(), None);
        let mut tail = LogTail::new(paths.log_path.clone());
        let mut monitor = HealthMonitor::new(&spec.apps);
        let mut reconciled = false;
        let mut ticker = tokio::time::interval(Duration::from_millis(500));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        enum Exit {
            Stop,
            Exited(std::io::Result<std::process::ExitStatus>),
            Failure(HealthFailure),
        }

        let exit = loop {
            tokio::select! {
                biased;
                changed = stop.changed() => {
                    match changed {
                        Ok(()) if *stop.borrow() => break Exit::Stop,
                        Err(_) => break Exit::Stop,
                        Ok(()) => {}
                    }
                }
                status = child.wait() => break Exit::Exited(status),
                _ = ticker.tick() => {
                    let lines = tail.poll().await;
                    monitor.observe_lines(&lines);
                    if monitor.saw_login_ok && !reconciled {
                        reconciled = true;
                        consecutive_auth = 0;
                        match o2p_config::reconcile_after_login(&spec, token) {
                            Ok(o2p_config::TokenReconcile::Match) => {}
                            Ok(o2p_config::TokenReconcile::Repaired) => logging!(
                                info,
                                Type::Network,
                                "openp2p account {} config.json was rendered again from the token file",
                                spec.id
                            ),
                            Ok(o2p_config::TokenReconcile::Mismatch) => {
                                logging!(
                                    warn,
                                    Type::Network,
                                    "openp2p account {}: the server-reported token differs from the token file; keeping the file token",
                                    spec.id
                                );
                                slot.set_warning("server-reported token differs from the saved token".into());
                            }
                            Err(error) => logging!(
                                warn,
                                Type::Network,
                                "openp2p account {} token reconcile failed: {error:#}",
                                spec.id
                            ),
                        }
                    }
                    if monitor.should_probe() {
                        monitor.probe().await;
                    }
                    account::enforce_permissions(&paths);
                    let phase = if monitor.is_degraded() {
                        OpenP2PPhase::Degraded
                    } else if monitor.saw_login_ok {
                        OpenP2PPhase::Online
                    } else {
                        OpenP2PPhase::LoginPending
                    };
                    let error = monitor.last_error.clone().or_else(|| slot.last_error());
                    slot.set_running(pid, phase, monitor.checks(), error);
                    if let Some(failure) = monitor.check() {
                        break Exit::Failure(failure);
                    }
                }
            }
        };

        match exit {
            Exit::Stop => {
                slot.set_phase(OpenP2PPhase::Stopping, None);
                child.terminate().await;
                slot.set_stopped();
                return;
            }
            Exit::Exited(status) => {
                let code = status.ok().and_then(|status| status.code());
                let log_tail = health::read_error_tail(&paths.log_path, 80);
                child.finish_after_exit().await;
                if monitor.online_for().is_some_and(|online| online >= ONLINE_RESET) {
                    attempt = 0;
                }
                if log_tail.contains("Access Denied") {
                    slot.set_terminal(OpenP2PPhase::AuthFailed, "openp2p rejected the token".into());
                    return;
                }
                let errors = log_tail.matches("login error:").count() as u32;
                if errors > 0 {
                    consecutive_auth = consecutive_auth.saturating_add(errors.min(2));
                } else if log_tail.contains("login ok") {
                    consecutive_auth = 0;
                }
                if consecutive_auth >= 2 {
                    slot.set_terminal(
                        OpenP2PPhase::AuthFailed,
                        "login failed twice; automatic restart stopped".into(),
                    );
                    return;
                }
                match health::classify_exit(&log_tail) {
                    health::HealthFailureKind::Auth => {
                        slot.set_terminal(OpenP2PPhase::AuthFailed, "authentication failed".into());
                        return;
                    }
                    health::HealthFailureKind::Config => {
                        slot.set_terminal(
                            OpenP2PPhase::ConfigFailed,
                            "the sidecar rejected its network binding configuration".into(),
                        );
                        return;
                    }
                    health::HealthFailureKind::Transient => {}
                }
                let delay = health::backoff(attempt);
                attempt = attempt.saturating_add(1);
                let error = health::sanitize(&format!("openp2p exited with code {code:?}"));
                slot.set_backoff(attempt as u32, delay.as_secs(), Some(error));
                if wait_or_stop(&mut stop, delay).await || SHUTTING_DOWN.load(Ordering::Acquire) {
                    slot.set_stopped();
                    return;
                }
            }
            Exit::Failure(failure) => {
                child.terminate().await;
                match failure {
                    HealthFailure::Auth => {
                        slot.set_terminal(OpenP2PPhase::AuthFailed, "authentication failed".into());
                        return;
                    }
                    HealthFailure::Config => {
                        slot.set_terminal(
                            OpenP2PPhase::ConfigFailed,
                            "the sidecar rejected its network binding configuration".into(),
                        );
                        return;
                    }
                    HealthFailure::LoginTimeout | HealthFailure::Degraded => {
                        let delay = health::backoff(attempt);
                        attempt = attempt.saturating_add(1);
                        let message = if failure == HealthFailure::LoginTimeout {
                            "login did not complete in time"
                        } else {
                            "forwarded app stayed unreachable"
                        };
                        slot.set_backoff(attempt as u32, delay.as_secs(), Some(message.into()));
                        if wait_or_stop(&mut stop, delay).await {
                            slot.set_stopped();
                            return;
                        }
                    }
                }
            }
        }
    }
}

async fn wait_or_stop(stop: &mut watch::Receiver<bool>, delay: Duration) -> bool {
    tokio::select! {
        biased;
        changed = stop.changed() => changed.is_err() || *stop.borrow(),
        () = tokio::time::sleep(delay) => false,
    }
}
