use anyhow::Result;
use chrono::{Datelike, Local, NaiveTime};
use common::models::EnforceAction;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::db::Db;

pub async fn evaluate_enforcement(
    uid: u32,
    db: &Arc<Mutex<Db>>,
    online: bool,
) -> Result<EnforceAction> {
    let db = db.lock().await;

    if online {
        if let Some(remaining) = db.get_server_remaining(uid)? {
            return Ok(match remaining.enforce.as_str() {
                "warn" => EnforceAction::Warn,
                "lock" => EnforceAction::Lock,
                _ => EnforceAction::Allow,
            });
        }
        // Server hasn't sent remaining yet — allow (benefit of the doubt on startup).
        return Ok(EnforceAction::Allow);
    }

    // Offline: calculate locally.
    offline_evaluate(uid, &db)
}

fn offline_evaluate(uid: u32, db: &Db) -> Result<EnforceAction> {
    let now = Local::now();
    let today = now.date_naive();
    let weekday = today.weekday().num_days_from_monday() as u8;

    // 1. Check schedule windows.
    let schedules = db.get_cached_schedules(uid)?;
    let (in_window, window_remaining_minutes) = if schedules.is_empty() {
        (true, i32::MAX)
    } else {
        let current_time = now.time();
        let matching = schedules.iter().find(|s| {
            s.day_of_week == weekday
                && parse_time(&s.start_time)
                    .zip(parse_time(&s.end_time))
                    .map(|(start, end)| current_time >= start && current_time <= end)
                    .unwrap_or(false)
        });
        match matching {
            None => (false, 0),
            Some(s) => {
                let end = parse_time(&s.end_time).unwrap_or(NaiveTime::from_hms_opt(23, 59, 0).unwrap());
                let secs = (end - current_time).num_seconds().max(0);
                (true, (secs / 60) as i32)
            }
        }
    };

    if !in_window {
        return Ok(EnforceAction::Lock);
    }

    // 2. Check daily limit.
    let limits = db.get_cached_daily_limits(uid)?;
    let today_str = today.to_string();
    let time_remaining_minutes = if let Some(limit) = limits.iter().find(|l| l.day_of_week == weekday) {
        let used_secs = db.get_usage_seconds(uid, &today_str)?;
        let adjustment = db.get_cached_adjustment(uid, &today_str)?;
        let used_minutes = (used_secs / 60) as i32;
        limit.allowed_minutes as i32 + adjustment - used_minutes
    } else {
        i32::MAX // no limit for this day
    };

    // 3. Effective remaining.
    let remaining = time_remaining_minutes.min(window_remaining_minutes);

    if remaining <= 0 {
        return Ok(EnforceAction::Lock);
    }

    // 4. Warning thresholds — warn if remaining is at or below any configured threshold.
    let enforcement = db.get_cached_enforcement(uid)?;
    if enforcement.warning_thresholds.iter().any(|&t| remaining <= t as i32) {
        return Ok(EnforceAction::Warn);
    }

    Ok(EnforceAction::Allow)
}

fn parse_time(s: &str) -> Option<NaiveTime> {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() < 2 {
        return None;
    }
    let h: u32 = parts[0].parse().ok()?;
    let m: u32 = parts[1].parse().ok()?;
    NaiveTime::from_hms_opt(h, m, 0)
}

/// Execute a lock for a UID: DBus lock → wait grace period → DBus terminate if still active.
pub async fn execute_lock(uid: u32, db: &Arc<Mutex<Db>>) -> Result<()> {
    let (session_ids, grace_minutes, language) = {
        let db = db.lock().await;
        let sessions = db.get_all_session_ids(uid)?;
        let enforcement = db.get_cached_enforcement(uid)?;
        (sessions, enforcement.lockout_grace_minutes, enforcement.language)
    };

    if session_ids.is_empty() {
        return Ok(());
    }

    // Final warning before the screen locks.
    let _ = crate::dbus::send_desktop_notification(
        uid,
        crate::i18n::notif_lock_title(&language),
        crate::i18n::notif_lock_body(&language),
    ).await;
    tokio::time::sleep(std::time::Duration::from_secs(4)).await;

    tracing::info!("Locking sessions for uid={uid}: {:?}", session_ids);
    crate::dbus::lock_sessions(&session_ids).await?;

    // Wait grace period, then terminate any still-active sessions.
    tokio::time::sleep(std::time::Duration::from_secs(grace_minutes as u64 * 60)).await;

    let still_active = {
        let db = db.lock().await;
        db.get_all_session_ids(uid)?
    };

    if !still_active.is_empty() {
        tracing::warn!("Sessions still active after grace period for uid={uid}, terminating");
        crate::dbus::terminate_sessions(&still_active).await?;
    }

    Ok(())
}

/// Midnight handler: called when the calendar date changes.
/// Resets daily usage and locks any sessions that fall outside the new day's schedule.
pub async fn handle_midnight(db: &Arc<Mutex<Db>>) -> Result<()> {
    let yesterday = (Local::now() - chrono::Duration::days(1)).date_naive();
    let uids = {
        let db = db.lock().await;
        db.get_managed_uids()?
    };

    for uid in &uids {
        let db_guard = db.lock().await;
        let _ = db_guard.reset_usage_for_date(*uid, &yesterday);
    }
    tracing::info!("Midnight: reset daily usage counters for {} users", uids.len());

    // Lock any sessions that fall outside the new day's allowed schedule window.
    for uid in uids {
        let action = evaluate_enforcement(uid, db, false).await?;
        if action == EnforceAction::Lock {
            tracing::info!("Midnight: locking uid={uid} (outside schedule for new day)");
            let db = db.clone();
            tokio::spawn(async move {
                if let Err(e) = execute_lock(uid, &db).await {
                    tracing::error!("Midnight lock failed for uid={uid}: {e}");
                }
            });
        }
    }

    Ok(())
}
