use crate::models::{LocalUser, RemainingEntry, UserConfig, UsageEntry};
use serde::{Deserialize, Serialize};

// ── message type constants ────────────────────────────────────────────────────

pub const MSG_AGENT_HELLO: &str = "agent_hello";
pub const MSG_USER_LIST_UPDATE: &str = "user_list_update";
pub const MSG_HEARTBEAT: &str = "heartbeat";
pub const MSG_USAGE_SYNC: &str = "usage_sync";
pub const MSG_PAIRING_REQUEST: &str = "pairing_request";

pub const MSG_CONFIG_PUSH: &str = "config_push";
pub const MSG_REMAINING_UPDATE: &str = "remaining_update";
pub const MSG_PAIRING_ACCEPTED: &str = "pairing_accepted";
pub const MSG_LOCK_NOW: &str = "lock_now";
pub const MSG_CONFIG_RELOAD: &str = "config_reload";

// ── Agent → Server ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentHello {
    pub machine_id: String,
    pub hostname: String,
    pub timezone: String,
    pub agent_version: String,
    pub last_config_version: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserListUpdate {
    pub users: Vec<LocalUser>,
    pub removed_uids: Vec<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeartbeatUser {
    pub local_uid: u32,
    pub active_seconds_since_last: u32,
    pub idle: bool,
    pub session_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Heartbeat {
    pub users: Vec<HeartbeatUser>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageSync {
    pub usage: Vec<UsageEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairingRequest {
    pub machine_id: String,
    pub hostname: String,
    pub pairing_code: String,
}

// ── Server → Agent ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigPush {
    pub config_version: i64,
    pub users: Vec<UserConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemainingUpdate {
    pub users: Vec<RemainingEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairingAccepted {
    pub agent_id: String,
    pub auth_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockNow {
    pub local_uid: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigReload {}

// ── Typed inbound envelope (server → agent) ───────────────────────────────────

#[derive(Debug, Clone)]
pub enum ServerMessage {
    ConfigPush(ConfigPush),
    RemainingUpdate(RemainingUpdate),
    PairingAccepted(PairingAccepted),
    LockNow(LockNow),
    ConfigReload,
    Unknown(String),
}
