//! FORK: Profile Bundle side effects for the profile file save path.
//!
//! `cmd/save_profile.rs` keeps the upstream save/validate/apply flow and only
//! calls into this module for generated-bundle rejection, member detection,
//! artifact snapshot/refresh/restore and the validated mark.

use super::{CmdResult, coded_error};
use crate::config::Config;
use clash_verge_logging::{Type, logging};
use smartstring::alias::String;

/// Only an active bundle may be refreshed with unvalidated output, have refresh
/// failures treated as fatal, or be marked validated after apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MemberSaveBundlePolicy {
    allow_unvalidated: bool,
    fatal_refresh_error: bool,
    mark_after_apply: bool,
}

const fn member_save_bundle_policy(bundle_active: bool) -> MemberSaveBundlePolicy {
    MemberSaveBundlePolicy {
        allow_unvalidated: bundle_active,
        fatal_refresh_error: bundle_active,
        mark_after_apply: bundle_active,
    }
}

pub(crate) struct SavedBundle {
    member: bool,
    active: bool,
    artifacts: Option<crate::config::bundle::BundleArtifacts>,
}

/// Editing the generated bundle file is rejected: members own its content.
pub(crate) fn ensure_writable(index: &String) -> CmdResult<()> {
    if crate::config::bundle::is_bundle_uid(index) {
        return Err(coded_error(
            "BUNDLE_GENERATED_READ_ONLY",
            "the bundle profile is generated from its members; edit the bundle members instead",
        ));
    }
    Ok(())
}

impl SavedBundle {
    pub(crate) async fn prepare(index: &String) -> anyhow::Result<Self> {
        let member = match crate::config::bundle::is_member(index).await {
            Ok(is_member) => is_member,
            Err(error) => {
                logging!(
                    warn,
                    Type::Config,
                    "failed to read bundle metadata while saving profile '{index}': {error:#}"
                );
                false
            }
        };
        let profiles = Config::profiles().await;
        let snapshot = profiles.latest_arc();
        let active = snapshot.current.as_deref() == Some(crate::config::bundle::BUNDLE_UID);
        let artifacts = if member {
            Some(crate::config::bundle::snapshot_artifacts().await?)
        } else {
            None
        };
        Ok(Self {
            member,
            active,
            artifacts,
        })
    }

    /// A saved member affects the active bundle runtime as well.
    pub(crate) const fn affects_runtime(&self, affects: bool) -> bool {
        affects || (self.member && self.active)
    }

    /// Refresh the bundle after a member file save. Active bundles must roll back
    /// on failure; inactive bundles keep the edit and stay stale until activation
    /// or an explicit refresh.
    pub(crate) async fn refresh(&self) -> std::result::Result<(), std::string::String> {
        if !self.member {
            return Ok(());
        }
        let profiles = Config::profiles().await;
        let snapshot = profiles.latest_arc();
        let _profile_write = crate::config::profiles::PROFILE_WRITE_LOCK.lock().await;
        let policy = member_save_bundle_policy(self.active);
        match crate::config::bundle::refresh(snapshot.as_ref(), true, policy.allow_unvalidated).await {
            Ok(_) => Ok(()),
            Err(error) if policy.fatal_refresh_error => {
                let message = error.to_string();
                logging!(
                    error,
                    Type::Config,
                    "[cmd配置save] bundle regeneration failed: {message}"
                );
                Err(message)
            }
            Err(error) => {
                logging!(
                    warn,
                    Type::Config,
                    "[cmd配置save] bundle stays stale until it is activated or refreshed: {error:#}"
                );
                self.restore().await;
                Ok(())
            }
        }
    }

    pub(crate) async fn restore(&self) {
        if let Some(artifacts) = self.artifacts.as_ref()
            && let Err(error) = crate::config::bundle::restore_artifacts(artifacts).await
        {
            logging!(
                warn,
                Type::Config,
                "[cmd配置save] failed to restore bundle artifacts: {error:#}"
            );
        }
    }

    pub(crate) async fn mark_validated(&self) {
        if self.member
            && member_save_bundle_policy(self.active).mark_after_apply
            && let Err(error) = crate::config::bundle::mark_validated_if_current().await
        {
            logging!(
                warn,
                Type::Config,
                "[cmd配置save] failed to mark the applied bundle validated: {error:#}"
            );
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::config::bundle::BUNDLE_UID;

    fn saved(member: bool, active: bool) -> SavedBundle {
        SavedBundle {
            member,
            active,
            artifacts: None,
        }
    }

    #[test]
    fn generated_bundle_is_read_only() {
        assert!(ensure_writable(&BUNDLE_UID.into()).is_err());
        assert!(ensure_writable(&"normal-profile".into()).is_ok());
    }

    #[test]
    fn active_member_save_affects_bundle_runtime() {
        assert!(saved(true, true).affects_runtime(false));
        assert!(saved(true, true).affects_runtime(true));
        // Inactive bundle: the saved member only affects runtime if it already did.
        assert!(!saved(true, false).affects_runtime(false));
        assert!(saved(true, false).affects_runtime(true));
        assert!(!saved(false, true).affects_runtime(false));
    }

    #[test]
    fn inactive_bundle_never_uses_member_runtime_state_for_bundle_validation() {
        // Regression: saving the CURRENT member while the bundle is inactive makes
        // `affects_runtime` true, but must not allow unvalidated bundle writes,
        // treat refresh failures as fatal, or mark the bundle validated.
        assert_eq!(
            member_save_bundle_policy(false),
            MemberSaveBundlePolicy {
                allow_unvalidated: false,
                fatal_refresh_error: false,
                mark_after_apply: false,
            }
        );
        assert_eq!(
            member_save_bundle_policy(true),
            MemberSaveBundlePolicy {
                allow_unvalidated: true,
                fatal_refresh_error: true,
                mark_after_apply: true,
            }
        );
    }
}
