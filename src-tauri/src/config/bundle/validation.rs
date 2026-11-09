//! Validation gate for generated bundle artifacts.

use anyhow::{Result, bail};
use clash_verge_logging::{Type, logging};
use tokio::fs;

use super::{BUNDLE_UID, GeneratedBundle, generated_path, load_meta, metadata_trusted, save_meta, sha256_hex, write};

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

/// Mark the written artifacts as validated after a successful runtime apply.
pub async fn mark_validated() -> Result<bool> {
    let Some(mut meta) = load_meta().await? else {
        return Ok(false);
    };
    if meta.members_sha256.is_empty() {
        // Legacy metadata cannot be trusted as applied: refresh must rewrite it
        // with a member fingerprint first.
        bail!("bundle metadata is missing a member fingerprint; refresh the bundle before marking it validated");
    }
    if !metadata_trusted(&meta) {
        bail!("bundle metadata member/order fingerprint mismatch; refusing to mark it validated");
    }
    if meta.generated_sha256.is_empty() {
        bail!("bundle metadata is missing a generated hash; refresh the bundle before marking it validated");
    }
    let Ok(generated_bytes) = fs::read(generated_path()?).await else {
        bail!("generated bundle profile is missing; refresh the bundle before marking it validated");
    };
    if sha256_hex(&generated_bytes).as_str() != meta.generated_sha256.as_str() {
        bail!("generated bundle profile changed since generation; refresh before marking it validated");
    }
    if meta.validated {
        return Ok(false);
    }
    meta.validated = true;
    save_meta(&meta).await?;
    Ok(true)
}

/// Startup/apply helper: only the current bundle can be considered validated by
/// a runtime apply.
pub async fn mark_validated_if_current() -> Result<bool> {
    let profiles = crate::config::Config::profiles().await.latest_arc();
    if profiles.current.as_deref() != Some(BUNDLE_UID) {
        return Ok(false);
    }
    mark_validated().await
}
