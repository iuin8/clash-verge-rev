//! FORK: Profile Bundle startup initialization.
//!
//! Kept out of `config/config.rs` so the upstream startup path only carries a
//! single call into fork code.

use clash_verge_logging::{Type, logging};

use crate::config::Config;

/// Migrate the removed merged.json store and regenerate a stale bundle before
/// the runtime config is generated for this launch. Failures are logged, never
/// fatal: the generated profile stays usable and the next start retries.
pub(crate) async fn init_bundle_state() {
    let profiles = Config::profiles().await;

    match crate::config::bundle::migrate_legacy(&profiles, false).await {
        Ok(outcome) if outcome.migrated => {
            logging!(
                info,
                Type::Config,
                "legacy merged profiles migrated into a profile bundle"
            );
        }
        Ok(_) => {}
        Err(error) => {
            logging!(error, Type::Config, "profile bundle migration failed: {error:#}");
        }
    }

    let snapshot = profiles.latest_arc();
    if let Err(error) = crate::config::bundle::cleanup_orphans(snapshot.as_ref()).await {
        logging!(
            warn,
            Type::Config,
            "failed to clean up orphaned profile bundle artifacts: {error:#}"
        );
    }
    match crate::config::bundle::refresh_if_stale(snapshot.as_ref(), false).await {
        Ok(Some(outcome)) if outcome.regenerated => {
            logging!(info, Type::Config, "regenerated the stale profile bundle at startup");
        }
        Ok(_) => {}
        Err(error) => {
            logging!(
                warn,
                Type::Config,
                "failed to refresh the profile bundle at startup: {error:#}"
            );
        }
    }
}
