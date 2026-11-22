use serde::{Deserialize, Serialize};

use crate::core::validate::ValidationOutcome;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum OpenP2PPhase {
    Stopped,
    Starting,
    LoginPending,
    Online,
    Degraded,
    Backoff,
    AuthFailed,
    ConfigFailed,
    TokenMissing,
    Stopping,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum OpenP2PCheckState {
    Unknown,
    Pending,
    Ok,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct OpenP2PCheck {
    pub state: OpenP2PCheckState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<std::string::String>,
}

impl Default for OpenP2PCheck {
    fn default() -> Self {
        Self {
            state: OpenP2PCheckState::Unknown,
            detail: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct OpenP2PAppCheck {
    pub name: std::string::String,
    pub protocol: std::string::String,
    pub src_port: u16,
    /// The openp2p log reported LISTEN ON PORT for this app.
    pub listen: bool,
    /// Loopback TCP connect result. UDP apps are not probed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tcp_reachable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub banner: Option<std::string::String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct OpenP2PChecks {
    pub login: OpenP2PCheck,
    pub apps: Vec<OpenP2PAppCheck>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct OpenP2PGateway {
    pub host: std::string::String,
    pub port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct OpenP2PAccount {
    pub id: std::string::String,
    pub label: std::string::String,
    pub enabled: bool,
    pub gateway: OpenP2PGateway,
    pub node: std::string::String,
    pub public_ip_port: u16,
    pub interface: std::string::String,
    pub phase: OpenP2PPhase,
    pub token_present: bool,
    pub key_present: bool,
    pub key_refs: Vec<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    pub checks: OpenP2PChecks,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<std::string::String>,
    pub log_dir: std::string::String,
    pub restart_count: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct OpenP2PArtifacts {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_uid: Option<std::string::String>,
    pub proxies: Vec<std::string::String>,
    pub groups: Vec<std::string::String>,
    pub enhanced: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<std::string::String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct OpenP2PView {
    pub config_revision: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_profile_uid: Option<std::string::String>,
    pub accounts: Vec<OpenP2PAccount>,
    pub artifacts: OpenP2PArtifacts,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_error: Option<std::string::String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum OpenP2PPreviewAction {
    Add,
    Update,
    Unchanged,
    Remove,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct OpenP2PAccountPreview {
    pub id: std::string::String,
    pub label: std::string::String,
    pub action: OpenP2PPreviewAction,
    pub warnings: Vec<std::string::String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct OpenP2PPreview {
    pub can_apply: bool,
    pub issues: Vec<crate::core::openp2p::schema::OpenP2PIssue>,
    pub accounts: Vec<OpenP2PAccountPreview>,
    pub proxies: Vec<std::string::String>,
    pub groups: Vec<std::string::String>,
    pub rules: Vec<std::string::String>,
    pub dns_policy: Vec<std::string::String>,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct OpenP2PApplyOutcome {
    pub applied: bool,
    pub state: OpenP2PView,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation: Option<ValidationOutcome>,
}
