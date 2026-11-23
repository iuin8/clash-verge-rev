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
    #[serde(default)]
    pub route_count: u32,
    #[serde(default)]
    pub profile_uid: Option<std::string::String>,
    #[serde(default)]
    pub profile_status: OpenP2PProfileStatus,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum OpenP2PProfileStatus {
    Ready,
    NeedsToken,
    NeedsKey,
    Error,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct OpenP2PSource {
    pub kind: std::string::String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub applied_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<std::string::String>,
    pub contains_mihomo: bool,
    pub self_contained: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct OpenP2PView {
    pub config_revision: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<OpenP2PSource>,
    pub accounts: Vec<OpenP2PAccount>,
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

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct OpenP2PApplyOutcome {
    pub applied: bool,
    pub state: OpenP2PView,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation: Option<ValidationOutcome>,
}
