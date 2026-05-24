use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use chrono::{Datelike, Local};
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use crate::api::auth::{internal, not_found};
use crate::db;
use crate::remaining;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct UsageQuery {
    pub from: Option<String>,
    pub to: Option<String>,
}

pub async fn get_usage(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Query(q): Query<UsageQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    db::get_profile(&state.db, id).map_err(internal)?.ok_or_else(not_found)?;

    let today = Local::now().date_naive();
    let to = q.to.unwrap_or_else(|| today.to_string());
    let from = q.from.unwrap_or_else(|| {
        (today - chrono::Duration::days(30)).to_string()
    });

    let daily = db::get_daily_usage_for_profile(&state.db, id, &from, &to).map_err(internal)?;
    let by_agent_raw = db::get_usage_by_agent_for_profile(&state.db, id, &from, &to).map_err(internal)?;
    let limits = db::get_daily_limits(&state.db, id).map_err(internal)?;
    let adj = db::get_adjustments(&state.db, id, Some(&from), Some(&to)).map_err(internal)?;

    let usage: Vec<serde_json::Value> = daily.iter().map(|(date, secs)| {
        let dow = db::weekday_for_date(date);
        let limit_min = limits.iter().find(|l| l.day_of_week == dow).map(|l| l.allowed_minutes);
        let adj_min: i32 = adj.iter().filter(|a| a.target_date == *date).map(|a| a.adjustment_minutes).sum();
        serde_json::json!({
            "date": date,
            "used_minutes": secs / 60,
            "limit_minutes": limit_min,
            "adjustments_minutes": adj_min,
        })
    }).collect();

    // Group by agent.
    let agents = db::list_agents(&state.db).map_err(internal)?;
    let mut by_agent: std::collections::HashMap<Uuid, Vec<serde_json::Value>> = std::collections::HashMap::new();
    for (agent_id, _agent_user_id, date, secs) in &by_agent_raw {
        by_agent.entry(*agent_id).or_default().push(serde_json::json!({
            "date": date,
            "used_minutes": secs / 60,
        }));
    }
    let by_agent_out: Vec<serde_json::Value> = by_agent.iter().map(|(aid, daily)| {
        let name = agents.iter().find(|a| a.id == *aid).map(|a| a.display_name.as_str()).unwrap_or("unknown");
        serde_json::json!({ "agent_name": name, "daily": daily })
    }).collect();

    Ok(Json(serde_json::json!({ "usage": usage, "by_agent": by_agent_out })))
}

pub async fn get_status(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let profile = db::get_profile(&state.db, id).map_err(internal)?.ok_or_else(not_found)?;

    let today = Local::now().date_naive();
    let today_str = today.to_string();
    let dow = today.weekday().num_days_from_monday() as u8;
    let used_secs = db::get_used_seconds_for_profile_today(&state.db, id, &today_str).map_err(internal)?;
    let limits = db::get_daily_limits(&state.db, id).map_err(internal)?;
    let adj = db::sum_adjustments_for_date(&state.db, id, &today_str).map_err(internal)?;
    let limit_min = limits.iter().find(|l| l.day_of_week == dow).map(|l| l.allowed_minutes);
    let used_min = (used_secs / 60) as i32;
    // Use 1440 as the base when no explicit limit is set — matches enforcement logic.
    let effective_limit = limit_min.unwrap_or(1440);
    let remaining = (effective_limit + adj - used_min).max(0);

    // Per-agent breakdown.
    let agent_users = db::get_agent_users_for_profile(&state.db, id).map_err(internal)?;
    let online = state.online.read().await;
    let mut agents_out = Vec::new();
    for au in &agent_users {
        let agent = db::get_agent_by_id(&state.db, au.agent_id).map_err(internal)?;
        let Some(agent) = agent else { continue; };
        let agent_used: i64 = db::get_usage_by_agent_for_profile(&state.db, id, &today_str, &today_str)
            .map_err(internal)?
            .iter()
            .filter(|(aid, _, _, _)| *aid == au.agent_id)
            .map(|(_, _, _, s)| s)
            .sum();
        agents_out.push(serde_json::json!({
            "agent_id": au.agent_id,
            "agent_name": agent.display_name,
            "local_username": au.local_username,
            "online": online.values().any(|h| h.agent_id == au.agent_id),
            "last_seen_at": agent.last_seen_at,
            "used_today_minutes": agent_used / 60,
        }));
    }

    let enforce = if remaining <= 0 { "lock" } else { "allow" };

    Ok(Json(serde_json::json!({
        "profile": {
            "id": profile.id,
            "display_name": profile.display_name,
            "today": {
                "date": today_str,
                "day_of_week": dow,
                "limit_minutes": limit_min,
                "used_minutes": used_min,
                "adjustments_minutes": adj,
                "remaining_minutes": remaining,
                "enforce": enforce,
            },
            "agents": agents_out,
        }
    })))
}

pub async fn dashboard(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let profiles = db::list_profiles(&state.db).map_err(internal)?;
    let today = Local::now().date_naive();
    let today_str = today.to_string();
    let dow = today.weekday().num_days_from_monday() as u8;
    let pending = db::pending_agent_count(&state.db).map_err(internal)?;
    let online = state.online.read().await;

    let mut out = Vec::new();
    for p in &profiles {
        let used_secs = db::get_used_seconds_for_profile_today(&state.db, p.id, &today_str)
            .map_err(internal)?;
        let limits = db::get_daily_limits(&state.db, p.id).map_err(internal)?;
        let adj = db::sum_adjustments_for_date(&state.db, p.id, &today_str).map_err(internal)?;
        let limit_min = limits.iter().find(|l| l.day_of_week == dow).map(|l| l.allowed_minutes);
        let used_min = (used_secs / 60) as i32;
        let remaining = limit_min.map(|l| (l + adj - used_min).max(0));

        let agent_users = db::get_agent_users_for_profile(&state.db, p.id).map_err(internal)?;
        let agents_total = agent_users.iter().map(|u| u.agent_id).collect::<std::collections::HashSet<_>>().len();
        let agents_online = agent_users.iter()
            .map(|u| u.agent_id)
            .collect::<std::collections::HashSet<_>>()
            .iter()
            .filter(|aid| online.values().any(|h| h.agent_id == **aid))
            .count();

        out.push(serde_json::json!({
            "id": p.id,
            "display_name": p.display_name,
            "remaining_minutes": remaining,
            "limit_minutes": limit_min,
            "used_minutes": used_min,
            "enforce": if remaining.map(|r| r <= 0).unwrap_or(false) { "lock" } else { "allow" },
            "agents_online": agents_online,
            "agents_total": agents_total,
        }));
    }

    // Suppress unused warning.
    let _ = remaining::build_config_push;

    Ok(Json(serde_json::json!({ "profiles": out, "pending_agents": pending })))
}
