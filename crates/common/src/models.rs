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
    #[serde(default)]
    pub preserve_tasks_on_lock: bool,
    pub warning_thresholds_minutes: Vec<u32>,
    #[serde(default = "default_lang")]
    pub language: String,
    #[serde(default)]
    pub blocked_domains: Vec<String>,
}

fn default_lang() -> String { "en".to_string() }

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

#[cfg(test)]
mod tests {
    use super::UserConfig;
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct LegacyUserConfig {
        local_uid: u32,
    }

    #[test]
    fn user_config_missing_preserve_tasks_defaults_to_false() {
        let json = r#"{
            "local_uid": 1000,
            "profile_id": "00000000-0000-0000-0000-000000000001",
            "status": "managed",
            "schedules": [],
            "daily_limits": [],
            "adjustments_today": 0,
            "adjustment_message": null,
            "lockout_grace_minutes": 5,
            "warning_thresholds_minutes": [15, 5, 1]
        }"#;

        let config: UserConfig = serde_json::from_str(json).unwrap();

        assert!(!config.preserve_tasks_on_lock);
    }

    #[test]
    fn user_config_serializes_preserve_tasks_setting() {
        let mut value = serde_json::json!({
            "local_uid": 1000,
            "profile_id": "00000000-0000-0000-0000-000000000001",
            "status": "managed",
            "schedules": [],
            "daily_limits": [],
            "adjustments_today": 0,
            "adjustment_message": null,
            "lockout_grace_minutes": 5,
            "warning_thresholds_minutes": [15, 5, 1]
        });
        let config: UserConfig = serde_json::from_value(value.clone()).unwrap();
        value["preserve_tasks_on_lock"] = serde_json::Value::Bool(false);
        value["language"] = serde_json::Value::String("en".to_string());
        value["blocked_domains"] = serde_json::json!([]);

        assert_eq!(serde_json::to_value(config).unwrap(), value);
    }

    #[test]
    fn legacy_agent_ignores_preserve_tasks_field() {
        let config: LegacyUserConfig = serde_json::from_value(serde_json::json!({
            "local_uid": 1000,
            "preserve_tasks_on_lock": true
        })).unwrap();

        assert_eq!(config.local_uid, 1000);
    }
}
