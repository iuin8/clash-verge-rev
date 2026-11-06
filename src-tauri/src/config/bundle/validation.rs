//! Validation gate for generated bundle artifacts.

use anyhow::{Result, bail};
use clash_verge_logging::{Type, logging};

use super::{GeneratedBundle, write};

/// Result of asking the app's core validator about a generated bundle profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BundleValidation {
    Valid,
    Invalid,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitDecision {
    Write,
    Reject,
    KeepStale,
}

/// Invalid output is never written. If the validator cannot run, only callers
/// that immediately apply (and can roll back) the runtime may write it.
pub const fn commit_decision(validation: BundleValidation, allow_unvalidated: bool) -> CommitDecision {
    match validation {
        BundleValidation::Valid => CommitDecision::Write,
        BundleValidation::Invalid => CommitDecision::Reject,
        BundleValidation::Unavailable if allow_unvalidated => CommitDecision::Write,
        BundleValidation::Unavailable => CommitDecision::KeepStale,
    }
}

/// The validator needs the app handle and a core sidecar; background refreshes
/// must not write unvalidated YAML when it is unavailable.
pub async fn validate_generated(yaml: &str) -> BundleValidation {
    if crate::APP_HANDLE.get().is_none() || crate::core::handle::Handle::global().is_exiting() {
        return BundleValidation::Unavailable;
    }
    match crate::core::validate::CoreConfigValidator::global()
        .validate_config_outcome_with(yaml)
        .await
    {
        Ok(outcome) if outcome.is_valid() => BundleValidation::Valid,
        Ok(
            crate::core::validate::ValidationOutcome::Busy | crate::core::validate::ValidationOutcome::Skipped { .. },
        ) => {
            logging!(
                info,
                Type::Config,
                "core validator is busy or exiting; bundle write deferred"
            );
            BundleValidation::Unavailable
        }
        Ok(outcome) => {
            logging!(
                warn,
                Type::Config,
                "generated bundle profile rejected by the core validator: {outcome}"
            );
            BundleValidation::Invalid
        }
        Err(error) => {
            logging!(
                warn,
                Type::Config,
                "core validator unavailable for the bundle profile: {error:#}"
            );
            BundleValidation::Unavailable
        }
    }
}

/// Validate (when possible) then write generated + metadata atomically each.
/// Artifacts are always written unvalidated: only a successful runtime apply may
/// call `mark_validated()`. This keeps a crash between write and apply detectable
/// (`is_stale` stays true) even when the core validator accepted the bytes.
pub async fn commit_generated(generated: &mut GeneratedBundle, allow_unvalidated: bool) -> Result<()> {
    let validation = validate_generated(&generated.yaml).await;
    match commit_decision(validation, allow_unvalidated) {
        CommitDecision::Write => {
            generated.meta.validated = false;
            write(generated).await
        }
        CommitDecision::Reject => bail!("generated bundle profile is invalid"),
        CommitDecision::KeepStale => {
            bail!("core validator is unavailable; keeping the previous bundle artifacts")
        }
    }
}
