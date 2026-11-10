//! Lazy single-flight + debounce materializer for automatic bundle updates.

use std::sync::{Arc, LazyLock};
use std::time::Duration;

use clash_verge_logging::{Type, logging};
use parking_lot::Mutex;

use super::finalize_bundle_refresh;
use crate::cmd::CmdResult;

const BUNDLE_MATERIALIZE_DEBOUNCE: Duration = Duration::from_millis(750);

/// Single-flight + debounce state for automatic bundle materialization.
#[derive(Debug, Default)]
struct BundleMaterializerState {
    running: bool,
    dirty: bool,
}

impl BundleMaterializerState {
    /// Mark dirty; returns true when the caller must start the worker.
    const fn mark(&mut self) -> bool {
        self.dirty = true;
        if self.running {
            return false;
        }
        self.running = true;
        true
    }

    /// Consume the dirty mark for one materialization run.
    const fn take(&mut self) -> bool {
        let dirty = self.dirty;
        self.dirty = false;
        dirty
    }

    /// Stop only when no mark arrived during the run; otherwise keep coalescing.
    const fn stop_if_clean(&mut self) -> bool {
        if self.dirty {
            return false;
        }
        self.running = false;
        true
    }
}

static BUNDLE_MATERIALIZER: LazyLock<Arc<Mutex<BundleMaterializerState>>> =
    LazyLock::new(|| Arc::new(Mutex::new(BundleMaterializerState::default())));

/// Mark the bundle dirty after an automatic member update and lazily start the
/// coalescing worker. Manual updates/full refresh bypass this and finalize
/// immediately.
pub(crate) fn mark_bundle_dirty() {
    let start = {
        let mut state = BUNDLE_MATERIALIZER.lock();
        state.mark()
    };
    if start {
        let state = Arc::clone(&BUNDLE_MATERIALIZER);
        crate::process::AsyncHandler::spawn(move || async move {
            run_bundle_materializer(state, BUNDLE_MATERIALIZE_DEBOUNCE, finalize_bundle_refresh).await;
        });
    }
}

async fn run_bundle_materializer<F, Fut>(
    state: Arc<Mutex<BundleMaterializerState>>,
    debounce: Duration,
    mut finalize: F,
) where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = CmdResult<bool>>,
{
    loop {
        tokio::time::sleep(debounce).await;
        let dirty = {
            let mut state = state.lock();
            state.take()
        };
        if !dirty {
            let done = {
                let mut state = state.lock();
                state.stop_if_clean()
            };
            if done {
                break;
            }
            continue;
        }
        match finalize().await {
            Ok(regenerated) => {
                logging!(
                    debug,
                    Type::Cmd,
                    "coalesced bundle materialization finished (regenerated={regenerated})"
                );
            }
            Err(error) => {
                logging!(warn, Type::Cmd, "coalesced bundle materialization failed: {error}");
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn materializer_state_is_single_flight_and_coalesces_marks() {
        let mut state = BundleMaterializerState::default();
        assert!(state.mark(), "first mark starts the worker");
        assert!(!state.mark(), "marks while running must not start another worker");
        assert!(state.take(), "the pending mark runs once");
        assert!(!state.take());

        // A mark that arrives during a run is coalesced into the next run.
        assert!(!state.mark(), "a mark during a run must not start another worker");
        assert!(!state.stop_if_clean());
        assert!(state.take());
        assert!(state.stop_if_clean());
        assert!(state.mark(), "a clean stop allows a later mark to restart");
    }

    #[tokio::test]
    async fn debounce_worker_coalesces_multiple_marks_into_one_finalize() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };

        let state = Arc::new(Mutex::new(BundleMaterializerState::default()));
        for _ in 0..5 {
            state.lock().mark();
        }
        let runs = Arc::new(AtomicUsize::new(0));
        let run_counter = Arc::clone(&runs);
        let worker_state = Arc::clone(&state);
        let handle = tokio::spawn(run_bundle_materializer(
            worker_state,
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
                if !running {
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
        state.lock().mark();
        let runs = Arc::new(AtomicUsize::new(0));
        let run_counter = Arc::clone(&runs);
        let finalize_state = Arc::clone(&state);
        let worker_state = Arc::clone(&state);
        let handle = tokio::spawn(run_bundle_materializer(
            worker_state,
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
                if !running {
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
}
