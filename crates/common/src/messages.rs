use crate::models::{LocalUser, RemainingEntry, UserConfig, UsageEntry};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

// ── message type constants ────────────────────────────────────────────────────

pub const MSG_AGENT_HELLO: &str = "agent_hello";
pub const MSG_USER_LIST_UPDATE: &str = "user_list_update";
pub const MSG_HEARTBEAT: &str = "heartbeat";
pub const MSG_USAGE_SYNC: &str = "usage_sync";
pub const MSG_PAIRING_REQUEST: &str = "pairing_request";
/// Sent only to servers that understand it; see `AppUsageSync`.
pub const MSG_APP_USAGE_SYNC: &str = "app_usage_sync";
/// A child started an app that needs a parent's approval.
pub const MSG_APP_REQUEST: &str = "app_request";

pub const MSG_CONFIG_PUSH: &str = "config_push";
pub const MSG_REMAINING_UPDATE: &str = "remaining_update";
pub const MSG_PAIRING_ACCEPTED: &str = "pairing_accepted";
pub const MSG_LOCK_NOW: &str = "lock_now";
pub const MSG_UNLOCK_NOW: &str = "unlock_now";
pub const MSG_CONFIG_RELOAD: &str = "config_reload";
pub const MSG_NOTIFY_USER: &str = "notify_user";
pub const MSG_UNPAIR: &str = "unpair";

pub const MSG_FETCH_LOGS: &str = "fetch_logs";
pub const MSG_LOG_RESPONSE: &str = "log_response";
pub const MSG_UPDATE_AGENT: &str = "update_agent";

// ── Agent → Server ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentHello {
    pub machine_id: String,
    pub hostname: String,
    pub timezone: String,
    pub agent_version: String,
    pub last_config_version: i64,
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Cloud tenant binding (email). Only present when the agent was started in
    /// cloud mode (`--cloud-account`). Absent on the wire for self-hosted agents,
    /// so an old server never sees the field. **Experimental.**
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloud_account: Option<String>,
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

/// Per-app usage totals (docs/feature-app-usage-limits.md). Each entry carries the
/// day's running total, not a delta, so the server can upsert with MAX and a
/// resend after a reconnect never double-counts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppUsageSync {
    pub entries: Vec<AppUsageEntry>,
    /// Day totals per app group (time while any member app ran). Absent from
    /// agents before app groups.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<GroupUsageEntry>,
}

/// Request to allow an app (new-app approval). Sent once per app per connection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppRequest {
    pub local_uid: u32,
    pub app_id: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroupUsageEntry {
    pub local_uid: u32,
    pub date: NaiveDate,
    pub group_id: String,
    pub seconds_total: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppUsageEntry {
    pub local_uid: u32,
    /// Local date on the agent.
    pub date: NaiveDate,
    /// `common::apps::AppId` as a string; the server drops ids it can't parse.
    pub app_id: String,
    /// `Name=` from the `.desktop` file, or a fallback derived from the id.
    pub display_name: String,
    /// Icon theme name from `Icon=` (never image data).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// freedesktop categories (`Game`, `WebBrowser`, …); absent from older agents.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<String>,
    pub seconds_total: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairingRequest {
    pub machine_id: String,
    pub hostname: String,
    pub pairing_code: String,
    /// Cloud tenant binding (email). Only present when the agent was started in
    /// cloud mode (`--cloud-account`). Absent on the wire for self-hosted agents.
    /// **Experimental.**
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloud_account: Option<String>,
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
pub struct UnlockNow {
    pub local_uid: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigReload {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotifyUser {
    pub local_uid: u32,
    pub summary: String,
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Unpair {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchLogs {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogResponse {
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateAgent {}

// ── Typed inbound envelope (server → agent) ───────────────────────────────────

#[derive(Debug, Clone)]
pub enum ServerMessage {
    ConfigPush(ConfigPush),
    RemainingUpdate(RemainingUpdate),
    PairingAccepted(PairingAccepted),
    LockNow(LockNow),
    UnlockNow(UnlockNow),
    NotifyUser(NotifyUser),
    ConfigReload,
    Unpair,
    FetchLogs,
    UpdateAgent,
    Unknown(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::WssMessage;

    #[test]
    fn app_usage_sync_round_trip() {
        let sync = AppUsageSync {
            entries: vec![
                AppUsageEntry {
                    local_uid: 1001,
                    date: NaiveDate::from_ymd_opt(2026, 10, 20).unwrap(),
                    app_id: "desktop:org.kde.dolphin".into(),
                    display_name: "Dolphin".into(),
                    icon: Some("system-file-manager".into()),
                    categories: vec!["System".into(), "FileManager".into()],
                    seconds_total: 3600,
                },
                AppUsageEntry {
                    local_uid: 1001,
                    date: NaiveDate::from_ymd_opt(2026, 10, 20).unwrap(),
                    app_id: "steam:570".into(),
                    display_name: "Steam game 570".into(),
                    icon: None,
                    categories: vec![],
                    seconds_total: 5,
                },
            ],
            groups: vec![GroupUsageEntry {
                local_uid: 1001,
                date: NaiveDate::from_ymd_opt(2026, 10, 20).unwrap(),
                group_id: "g1".into(),
                seconds_total: 7,
            }],
        };
        let msg = WssMessage::new(MSG_APP_USAGE_SYNC, &sync).unwrap();
        let json = msg.to_json().unwrap();
        assert!(json.contains(r#""date":"2026-10-20""#));
        assert!(!json.contains(r#""icon":null"#));
        assert_eq!(json.matches("categories").count(), 1); // empty list omitted
        let back: AppUsageSync = WssMessage::from_json(&json).unwrap().parse_payload().unwrap();
        assert_eq!(back, sync);
    }
}
