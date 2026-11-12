//! Lazy single-flight + debounce materializer for automatic bundle updates.

use std::sync::{Arc, LazyLock};
use std::time::Duration;

use clash_verge_logging::{Type, logging};
use parking_lot::Mutex;

use super::finalize_bundle_refresh;
use crate::cmd::CmdResult;

const BUNDLE_MATERIALIZE_DEBOUNCE: Duration = Duration::from_millis(750);
/// Bounded retries for a transient finalize failure (e.g. validator busy).
const BUNDLE_MATERIALIZE_MAX_ATTEMPTS: u32 = 3;

/// Single-flight + debounce state for automatic bundle materialization.
#[derive(Debug, Default)]
struct BundleMaterializerState {
    /// Owner token of the worker allowed to run; `None` means no worker.
    running: Option<u64>,
    dirty: bool,
    /// Increments on every externally requested mark; manual finalizers use it
    /// to clear only marks that were already covered by their run.
    generation: u64,
    /// Monotonic token source for workers.
    next_owner: u64,
}

impl BundleMaterializerState {
    /// Mark dirty; returns the owner token when the caller must start a worker.
    const fn mark(&mut self) -> Option<u64> {
        self.dirty = true;
        self.generation = self.generation.wrapping_add(1);
        if self.running.is_some() {
            return None;
        }
        self.next_owner = self.next_owner.wrapping_add(1);
        self.running = Some(self.next_owner);
        Some(self.next_owner)
    }

    /// Consume the dirty mark for one materialization run.
    const fn take(&mut self) -> bool {
        let dirty = self.dirty;
        self.dirty = false;
        dirty
    }

    /// Stop only when no mark arrived during the run and this owner is still the
    /// current worker; otherwise keep coalescing.
    fn stop_if_clean(&mut self, owner: u64) -> bool {
        if self.dirty || self.running != Some(owner) {
            return false;
        }
        self.running = None;
        true
    }

    /// Release a worker token only if it still belongs to `owner`; a newer worker
    /// must never be untracked by an older guard.
    fn release_owner(&mut self, owner: u64, panicked: bool) {
        if self.running != Some(owner) {
            return;
        }
        self.running = None;
        if panicked {
            // Keep a recoverable dirty mark; the next mark restarts the worker.
            self.dirty = true;
        }
    }

    const fn current_generation(&self) -> u64 {
        self.generation
    }

    /// A successful manual materialization covers pending marks up to the
    /// generation captured before it ran; newer marks are left to the worker.
    const fn clear_dirty_if_generation(&mut self, generation: u64) -> bool {
        if self.generation != generation {
            return false;
        }
        self.dirty = false;
        true
    }
}

static BUNDLE_MATERIALIZER: LazyLock<Arc<Mutex<BundleMaterializerState>>> =
    LazyLock::new(|| Arc::new(Mutex::new(BundleMaterializerState::default())));

/// Mark the bundle dirty after an automatic member update and lazily start the
/// coalescing worker. Manual updates/full refresh bypass this and finalize
/// immediately.
/// Generation snapshot for a manual finalizer; pair with `end_manual_materialize`.
pub(crate) fn begin_manual_materialize() -> u64 {
    BUNDLE_MATERIALIZER.lock().current_generation()
}

/// Clear a pending automatic mark when a manual finalizer covered it and no
/// newer member update arrived while it ran.
pub(crate) fn end_manual_materialize(generation: u64, succeeded: bool) {
    if !succeeded {
        return;
    }
    BUNDLE_MATERIALIZER.lock().clear_dirty_if_generation(generation);
}

pub(crate) fn mark_bundle_dirty() {
    let owner = {
        let mut state = BUNDLE_MATERIALIZER.lock();
        state.mark()
    };
    if let Some(owner) = owner {
        let state = Arc::clone(&BUNDLE_MATERIALIZER);
        crate::process::AsyncHandler::spawn(move || async move {
            run_bundle_materializer(state, owner, BUNDLE_MATERIALIZE_DEBOUNCE, finalize_bundle_refresh).await;
        });
    }
}

/// RAII guard: even a panic/abort inside the worker must not leave the state
/// wedged with `running = true`.
struct MaterializerRunningGuard {
    state: Arc<Mutex<BundleMaterializerState>>,
    owner: u64,
}

impl Drop for MaterializerRunningGuard {
    fn drop(&mut self) {
        let panicked = std::thread::panicking();
        self.state.lock().release_owner(self.owner, panicked);
    }
}

async fn run_bundle_materializer<F, Fut>(
    state: Arc<Mutex<BundleMaterializerState>>,
    owner: u64,
    debounce: Duration,
    mut finalize: F,
) where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = CmdResult<bool>>,
{
    let _running_guard = MaterializerRunningGuard {
        state: Arc::clone(&state),
        owner,
    };
    let mut attempts: u32 = 0;
    loop {
        tokio::time::sleep(debounce).await;
        let dirty = {
            let mut state = state.lock();
            state.take()
        };
        if !dirty {
            let done = {
                let mut state = state.lock();
                state.stop_if_clean(owner)
            };
            if done {
                break;
            }
            continue;
        }
        match finalize().await {
            Ok(regenerated) => {
                attempts = 0;
                logging!(
                    debug,
                    Type::Cmd,
                    "coalesced bundle materialization finished (regenerated={regenerated})"
                );
            }
            Err(error) => {
                attempts += 1;
                if attempts < BUNDLE_MATERIALIZE_MAX_ATTEMPTS {
                    // Bounded retry: keep dirty, the next debounce tick retries.
                    state.lock().dirty = true;
                    logging!(
                        warn,
                        Type::Cmd,
                        "coalesced bundle materialization failed (attempt {attempts}/{BUNDLE_MATERIALIZE_MAX_ATTEMPTS}), retrying: {error}"
                    );
                } else {
                    attempts = 0;
                    logging!(
                        error,
                        Type::Cmd,
                        "coalesced bundle materialization failed after {BUNDLE_MATERIALIZE_MAX_ATTEMPTS} attempts; waiting for the next update: {error}"
                    );
                }
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn start(state: &mut BundleMaterializerState) -> u64 {
        state.mark().expect("worker starts")
    }

    #[test]
    fn materializer_state_is_single_flight_and_coalesces_marks() {
        let mut state = BundleMaterializerState::default();
        let owner = start(&mut state);
        assert!(
            state.mark().is_none(),
            "marks while running must not start another worker"
        );
        assert!(state.take(), "the pending mark runs once");
        assert!(!state.take());

        // A mark that arrives during a run is coalesced into the next run.
        assert!(state.mark().is_none());
        assert!(!state.stop_if_clean(owner));
        assert!(state.take());
        assert!(state.stop_if_clean(owner));
        assert!(state.mark().is_some(), "a clean stop allows a later mark to restart");
    }

    #[test]
    fn old_guard_does_not_clear_a_new_owner() {
        let state = Arc::new(Mutex::new(BundleMaterializerState::default()));
        let old = {
            let mut guard = state.lock();
            start(&mut guard)
        };
        let old_guard = MaterializerRunningGuard {
            state: Arc::clone(&state),
            owner: old,
        };
        assert!(state.lock().take());
        assert!(state.lock().stop_if_clean(old));
        let new = {
            let mut guard = state.lock();
            start(&mut guard)
        };
        assert_ne!(old, new);

        // The old worker finishing after a new one started must not untrack it.
        drop(old_guard);
        assert_eq!(
            state.lock().running,
            Some(new),
            "an old guard must only clear its own owner token"
        );
    }

    #[tokio::test]
    async fn debounce_worker_coalesces_multiple_marks_into_one_finalize() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };

        let state = Arc::new(Mutex::new(BundleMaterializerState::default()));
        let mut owner = None;
        for _ in 0..5 {
            let mut guard = state.lock();
            if let Some(id) = guard.mark() {
                owner = Some(id);
            }
        }
        let owner = owner.expect("the first mark starts a worker");
        let runs = Arc::new(AtomicUsize::new(0));
        let run_counter = Arc::clone(&runs);
        let worker_state = Arc::clone(&state);
        let handle = tokio::spawn(run_bundle_materializer(
            worker_state,
            owner,
            Duration::from_millis(20),
            move || {
                let run_counter = Arc::clone(&run_counter);
                async move {
                    run_counter.fetch_add(1, Ordering::SeqCst);
                    Ok(true)
                }
            },
        ));

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let running = state.lock().running;
                if running.is_none() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("materializer should stop");
        handle.await.unwrap();
        assert_eq!(
            runs.load(Ordering::SeqCst),
            1,
            "five dirty marks must coalesce into one finalization"
        );
    }

    #[tokio::test]
    async fn mark_during_finalize_schedules_one_more_run() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };

        let state = Arc::new(Mutex::new(BundleMaterializerState::default()));
        let owner = {
            let mut guard = state.lock();
            start(&mut guard)
        };
        let runs = Arc::new(AtomicUsize::new(0));
        let run_counter = Arc::clone(&runs);
        let finalize_state = Arc::clone(&state);
        let worker_state = Arc::clone(&state);
        let handle = tokio::spawn(run_bundle_materializer(
            worker_state,
            owner,
            Duration::from_millis(20),
            move || {
                let run_counter = Arc::clone(&run_counter);
                let finalize_state = Arc::clone(&finalize_state);
                async move {
                    let run = run_counter.fetch_add(1, Ordering::SeqCst);
                    if run == 0 {
                        finalize_state.lock().mark();
                    }
                    Ok(true)
                }
            },
        ));

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let running = state.lock().running;
                if running.is_none() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("materializer should stop");
        handle.await.unwrap();
        assert_eq!(
            runs.load(Ordering::SeqCst),
            2,
            "a mark arriving during finalize must schedule exactly one more run"
        );
    }

    #[test]
    fn manual_finalize_clears_only_marks_covered_by_its_generation() {
        let mut state = BundleMaterializerState::default();
        assert!(state.mark().is_some());
        let covered = state.current_generation();

        // A newer mark must survive the older manual finalizer.
        assert!(state.mark().is_none());
        assert!(!state.clear_dirty_if_generation(covered));
        assert!(state.dirty);

        let latest = state.current_generation();
        assert!(state.clear_dirty_if_generation(latest));
        assert!(!state.dirty);
    }

    #[tokio::test]
    async fn panic_in_finalize_resets_running_and_keeps_a_recoverable_dirty_mark() {
        let state = Arc::new(Mutex::new(BundleMaterializerState::default()));
        let owner = {
            let mut guard = state.lock();
            start(&mut guard)
        };
        let worker_state = Arc::clone(&state);
        let handle = tokio::spawn(run_bundle_materializer(
            worker_state,
            owner,
            Duration::from_millis(5),
            move || async move {
                panic!("finalize panic");
            },
        ));
        assert!(handle.await.is_err(), "the panic must reach the join handle");
        let guard = state.lock();
        assert!(guard.running.is_none(), "RAII guard must reset running on panic");
        assert!(guard.dirty, "a panicked run leaves a recoverable dirty mark");
        drop(guard);
    }

    #[tokio::test]
    async fn finalize_errors_retry_with_a_bound() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };

        let state = Arc::new(Mutex::new(BundleMaterializerState::default()));
        let owner = {
            let mut guard = state.lock();
            start(&mut guard)
        };
        let runs = Arc::new(AtomicUsize::new(0));
        let run_counter = Arc::clone(&runs);
        let worker_state = Arc::clone(&state);
        let handle = tokio::spawn(run_bundle_materializer(
            worker_state,
            owner,
            Duration::from_millis(5),
            move || {
                let run_counter = Arc::clone(&run_counter);
                async move {
                    run_counter.fetch_add(1, Ordering::SeqCst);
                    Err(crate::cmd::coded_error("TEST", "transient failure"))
                }
            },
        ));

        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let running = state.lock().running;
                if running.is_none() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("materializer must stop after the bounded retries");
        handle.await.unwrap();
        assert_eq!(
            runs.load(Ordering::SeqCst),
            BUNDLE_MATERIALIZE_MAX_ATTEMPTS as usize,
            "retries are bounded"
        );
        let guard = state.lock();
        assert!(guard.running.is_none());
        assert!(
            !guard.dirty,
            "after giving up, the next external mark starts a fresh worker"
        );
        drop(guard);
    }
}
