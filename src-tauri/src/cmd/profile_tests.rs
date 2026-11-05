use super::{commit_current_profile, run_profile_config_update_transition};
use crate::config::{IProfiles, PrfItem};
use crate::core::validate::ValidationOutcome;
use clash_verge_draft::Draft;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::Poll,
    time::Duration,
};
use tokio::sync::Barrier;

struct CancellationProbe {
    cancelled: Arc<AtomicBool>,
    completed: Arc<AtomicBool>,
}

impl Drop for CancellationProbe {
    fn drop(&mut self) {
        if !self.completed.load(Ordering::Acquire) {
            self.cancelled.store(true, Ordering::Release);
        }
    }
}

fn profile(uid: &str) -> PrfItem {
    PrfItem {
        uid: Some(uid.into()),
        ..PrfItem::default()
    }
}

#[tokio::test]
async fn committing_profile_switch_preserves_profiles_added_after_draft_creation() -> anyhow::Result<()> {
    let profiles = Draft::new(IProfiles {
        current: Some("a".into()),
        items: Some(vec![profile("a"), profile("b")]),
    });
    profiles.edit_draft(|draft| {
        draft.patch_config(&IProfiles {
            current: Some("b".into()),
            items: None,
        });
    });
    profiles
        .with_data_modify(|mut committed| async move {
            committed.items.get_or_insert_with(Vec::new).push(profile("new"));
            Ok((committed, ()))
        })
        .await?;

    commit_current_profile(&profiles, Some("b".into())).await?;

    let committed = profiles.data_arc();
    assert_eq!(committed.current.as_deref(), Some("b"));
    assert!(committed.get_item("new").is_ok());
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn profile_config_update_runs_past_former_deadline_without_cancellation() -> anyhow::Result<()> {
    let update_started = Arc::new(Barrier::new(2));
    let release_update = Arc::new(Barrier::new(2));
    let update_cancelled = Arc::new(AtomicBool::new(false));
    let update_completed = Arc::new(AtomicBool::new(false));

    let mut update = Box::pin(run_profile_config_update_transition({
        let update_started = Arc::clone(&update_started);
        let release_update = Arc::clone(&release_update);
        let update_cancelled = Arc::clone(&update_cancelled);
        let update_completed = Arc::clone(&update_completed);
        move || async move {
            let _probe = CancellationProbe {
                cancelled: update_cancelled,
                completed: Arc::clone(&update_completed),
            };
            update_started.wait().await;
            release_update.wait().await;
            update_completed.store(true, Ordering::Release);
            Ok(ValidationOutcome::Valid)
        }
    }));

    assert!(matches!(futures::poll!(update.as_mut()), Poll::Pending));
    update_started.wait().await;
    tokio::time::advance(Duration::from_secs(31)).await;

    assert!(matches!(futures::poll!(update.as_mut()), Poll::Pending));
    assert!(!update_cancelled.load(Ordering::Acquire));

    release_update.wait().await;
    assert!(update.await?.is_valid());
    assert!(update_completed.load(Ordering::Acquire));
    assert!(!update_cancelled.load(Ordering::Acquire));
    Ok(())
}
