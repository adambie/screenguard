use anyhow::Result;
use chrono::{Local, NaiveDate, NaiveTime, TimeZone, Timelike};
use common::messages::ConfigPush;
use common::models::{
    DailyLimit as ModelDailyLimit, EnforceAction, RemainingEntry, Schedule as ModelSchedule,
    UserConfig, UserStatus,
};
use uuid::Uuid;

use crate::db::{self, DbPool};

/// Calculate remaining time for every managed agent_user linked to this agent,
/// update daily_usage with the new active seconds, and return a list of
/// RemainingEntry values to send back in a remaining_update message.
pub fn calculate_remaining_for_agent(
    pool: &DbPool,
    agent_id: Uuid,
    agent_timezone: &str,
    admin_timezone: &str,
    heartbeat_users: &[(u32, u32)], // (local_uid, active_seconds_since_last)
) -> Result<Vec<RemainingEntry>> {
    let today = today_in_timezone(agent_timezone);
    let today_str = today.to_string();
    let now_time = current_time_in_timezone(agent_timezone);

    let mut entries = Vec::new();

    for (local_uid, active_secs) in heartbeat_users {
        let Some(au) = db::get_agent_user(pool, agent_id, *local_uid)? else {
            continue;
        };
        let Some(profile_id) = au.profile_id else {
            // Unmanaged user — always allow.
            entries.push(RemainingEntry {
                local_uid: *local_uid,
                remaining_minutes: 1440,
                limit_today_minutes: None,
                used_today_minutes: 0,
                adjustments_today_minutes: 0,
                current_window_ends_at: None,
                next_window_starts_at: None,
                enforce: EnforceAction::Allow,
            });
            continue;
        };

        // 1. Persist new active seconds.
        if *active_secs > 0 {
            db::add_usage_seconds(pool, au.id, &today_str, *active_secs as i64)?;
        }

        // 2. Sum all usage for this profile today (across all agents).
        let used_secs = db::get_used_seconds_for_profile_today(pool, profile_id, &today_str)?;
        let used_minutes = (used_secs / 60) as i32;

        // 3. Daily limit for today's weekday.
        let weekday = db::weekday_for_date(&today_str);
        let limits = db::get_daily_limits(pool, profile_id)?;
        let (limit_minutes, limit_today) = limits
            .iter()
            .find(|l| l.day_of_week == weekday)
            .map(|l| (l.allowed_minutes, Some(l.allowed_minutes as u32)))
            .unwrap_or((1440, None));

        // 4. Adjustments today.
        let adjustments = db::sum_adjustments_for_date(pool, profile_id, &today_str)?;

        // 5. Remaining from limit.
        let mut remaining = (limit_minutes + adjustments - used_minutes).max(0);

        // 6. Schedule window check (times converted from admin_tz to agent_tz).
        let schedules = db::get_schedules(pool, profile_id)?;
        let converted_schedules: Vec<crate::db::Schedule> = schedules.iter().map(|s| {
            crate::db::Schedule {
                id: s.id,
                profile_id: s.profile_id,
                day_of_week: s.day_of_week,
                start_time: {
                    let t = db::parse_time(&s.start_time).unwrap_or(NaiveTime::from_hms_opt(0, 0, 0).unwrap());
                    let converted = schedule_time_in_agent_tz(t, admin_timezone, agent_timezone);
                    format!("{:02}:{:02}", converted.hour(), converted.minute())
                },
                end_time: {
                    let t = db::parse_time(&s.end_time).unwrap_or(NaiveTime::from_hms_opt(23, 59, 0).unwrap());
                    let converted = schedule_time_in_agent_tz(t, admin_timezone, agent_timezone);
                    format!("{:02}:{:02}", converted.hour(), converted.minute())
                },
            }
        }).collect();
        let (window_ends_at, next_window) = check_schedule_windows(&converted_schedules, weekday, now_time);

        if window_ends_at.is_none() && !converted_schedules.is_empty() {
            // Outside all windows — lock regardless of time remaining.
            entries.push(RemainingEntry {
                local_uid: *local_uid,
                remaining_minutes: 0,
                limit_today_minutes: limit_today,
                used_today_minutes: used_minutes as u32,
                adjustments_today_minutes: adjustments,
                current_window_ends_at: None,
                next_window_starts_at: next_window,
                enforce: EnforceAction::Lock,
            });
            continue;
        }

        // Cap remaining by time until window ends.
        if let Some(end) = window_ends_at {
            let secs_to_end = (end - now_time).num_seconds().max(0);
            let mins_to_end = (secs_to_end / 60) as i32;
            remaining = remaining.min(mins_to_end);
        }

        // 7. Determine enforce action.
        let enforcement = db::get_enforcement_settings(pool, profile_id)?;
        let enforce = if remaining <= 0 {
            EnforceAction::Lock
        } else if enforcement.warning_thresholds.iter().any(|&t| remaining <= t) {
            EnforceAction::Warn
        } else {
            EnforceAction::Allow
        };

        entries.push(RemainingEntry {
            local_uid: *local_uid,
            remaining_minutes: remaining,
            limit_today_minutes: limit_today,
            used_today_minutes: used_minutes as u32,
            adjustments_today_minutes: adjustments,
            current_window_ends_at: window_ends_at,
            next_window_starts_at: next_window,
            enforce,
        });
    }

    Ok(entries)
}

/// Build a ConfigPush for a specific agent from the DB.
pub fn build_config_push(pool: &DbPool, agent_id: Uuid, config_version: i64) -> Result<ConfigPush> {
    let agent_users = db::list_agent_users(pool, agent_id)?;
    let mut user_configs = Vec::new();

    for au in &agent_users {
        if au.status != "managed" {
            continue;
        }
        let Some(profile_id) = au.profile_id else { continue; };

        let schedules = db::get_schedules(pool, profile_id)?
            .into_iter()
            .map(|s| ModelSchedule {
                day_of_week: s.day_of_week,
                start_time: db::parse_time(&s.start_time)
                    .unwrap_or(NaiveTime::from_hms_opt(0, 0, 0).unwrap()),
                end_time: db::parse_time(&s.end_time)
                    .unwrap_or(NaiveTime::from_hms_opt(23, 59, 0).unwrap()),
            })
            .collect();

        let daily_limits = db::get_daily_limits(pool, profile_id)?
            .into_iter()
            .map(|l| ModelDailyLimit {
                day_of_week: l.day_of_week,
                allowed_minutes: l.allowed_minutes as u32,
            })
            .collect();

        let enforcement = db::get_enforcement_settings(pool, profile_id)?;
        let today = Local::now().date_naive().to_string();
        let today_adj = db::sum_adjustments_for_date(pool, profile_id, &today)?;
        let adjustment_message = db::latest_adjustment_reason_for_date(pool, profile_id, &today)?;
        let language = db::get_profile(pool, profile_id)?
            .map(|p| p.language)
            .unwrap_or_else(|| "en".to_string());

        user_configs.push(UserConfig {
            local_uid: au.local_uid as u32,
            profile_id,
            status: UserStatus::Managed,
            schedules,
            daily_limits,
            adjustments_today: today_adj,
            adjustment_message,
            lockout_grace_minutes: enforcement.lockout_grace_minutes as u32,
            warning_thresholds_minutes: enforcement.warning_thresholds.iter().map(|&t| t as u32).collect(),
            language,
        });
    }

    Ok(ConfigPush { config_version, users: user_configs })
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// Convert a naive schedule time from admin_tz to agent_tz, using today's UTC
/// date for DST-aware conversion.
fn schedule_time_in_agent_tz(naive: NaiveTime, admin_tz: &str, agent_tz: &str) -> NaiveTime {
    if admin_tz == agent_tz {
        return naive;
    }
    let a_tz: chrono_tz::Tz = admin_tz.parse().unwrap_or(chrono_tz::UTC);
    let b_tz: chrono_tz::Tz = agent_tz.parse().unwrap_or(chrono_tz::UTC);
    let today = chrono::Utc::now().date_naive();
    let dt_in_admin = a_tz.from_local_datetime(&today.and_time(naive)).single();
    match dt_in_admin {
        Some(dt) => dt.with_timezone(&b_tz).time(),
        None => naive, // ambiguous time (DST gap) — use as-is
    }
}

fn today_in_timezone(tz: &str) -> NaiveDate {
    let tz: chrono_tz::Tz = tz.parse().unwrap_or(chrono_tz::UTC);
    chrono::Utc::now().with_timezone(&tz).date_naive()
}

fn current_time_in_timezone(tz: &str) -> NaiveTime {
    let tz: chrono_tz::Tz = tz.parse().unwrap_or(chrono_tz::UTC);
    chrono::Utc::now().with_timezone(&tz).time()
}

/// Returns (current_window_end, next_window_start) given schedules and current time.
fn check_schedule_windows(
    schedules: &[crate::db::Schedule],
    weekday: u8,
    now: NaiveTime,
) -> (Option<NaiveTime>, Option<NaiveTime>) {
    // Find an active window.
    let active_end = schedules.iter()
        .filter(|s| s.day_of_week == weekday)
        .filter_map(|s| {
            let start = db::parse_time(&s.start_time)?;
            let end = db::parse_time(&s.end_time)?;
            if now >= start && now < end { Some(end) } else { None }
        })
        .min();

    // Find the next upcoming window start (today or wrapping to tomorrow).
    let next_start = schedules.iter()
        .filter_map(|s| {
            let start = db::parse_time(&s.start_time)?;
            if s.day_of_week == weekday && start > now {
                Some(start)
            } else {
                None
            }
        })
        .min();

    (active_end, next_start)
}
