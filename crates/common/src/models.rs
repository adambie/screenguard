use chrono::{NaiveDate, NaiveTime};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schedule {
    pub day_of_week: u8,
    pub start_time: NaiveTime,
    pub end_time: NaiveTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyLimit {
    pub day_of_week: u8,
    pub allowed_minutes: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserConfig {
    pub local_uid: u32,
    pub profile_id: Uuid,
    pub status: UserStatus,
    pub schedules: Vec<Schedule>,
    pub daily_limits: Vec<DailyLimit>,
    pub adjustments_today: i32,
    pub adjustment_message: Option<String>,
    pub lockout_grace_minutes: u32,
    pub warning_thresholds_minutes: Vec<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalUser {
    pub local_uid: u32,
    pub username: String,
    pub display_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeartbeatEntry {
    pub local_uid: u32,
    pub active_seconds_since_last: u32,
    pub idle: bool,
    pub session_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageEntry {
    pub local_uid: u32,
    pub date: NaiveDate,
    pub used_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemainingEntry {
    pub local_uid: u32,
    pub remaining_minutes: i32,
    pub limit_today_minutes: Option<u32>,
    pub used_today_minutes: u32,
    pub adjustments_today_minutes: i32,
    pub current_window_ends_at: Option<NaiveTime>,
    pub next_window_starts_at: Option<NaiveTime>,
    pub enforce: EnforceAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EnforceAction {
    Allow,
    Warn,
    Lock,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UserStatus {
    Managed,
    Unmanaged,
}
