use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use chrono::Local;
use common::messages::{MSG_CONFIG_PUSH, MSG_NOTIFY_USER, MSG_REMAINING_UPDATE, NotifyUser, RemainingUpdate};
use common::protocol::WssMessage;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use crate::api::auth::{internal, not_found};
use crate::db;
use crate::remaining;
use crate::state::AppState;

// ── profiles ──────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CreateProfileBody {
    pub display_name: String,
}

pub async fn list_profiles(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let profiles = db::list_profiles(&state.db).map_err(internal)?;
    Ok(Json(serde_json::json!({ "profiles": profiles })))
}

pub async fn create_profile(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateProfileBody>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<serde_json::Value>)> {
    let profile = db::create_profile(&state.db, &body.display_name).map_err(internal)?;
    Ok((StatusCode::CREATED, Json(serde_json::json!(profile))))
}

pub async fn get_profile(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let profile = db::get_profile(&state.db, id).map_err(internal)?.ok_or_else(not_found)?;
    let schedules = db::get_schedules(&state.db, id).map_err(internal)?;
    let limits = db::get_daily_limits(&state.db, id).map_err(internal)?;
    let users = db::get_agent_users_for_profile(&state.db, id).map_err(internal)?;
    Ok(Json(serde_json::json!({
        "profile": profile,
        "schedules": schedules,
        "daily_limits": limits,
        "agent_users": users,
    })))
}

#[derive(Deserialize)]
pub struct PatchProfileBody {
    pub display_name: Option<String>,
    pub language: Option<String>,
}

pub async fn patch_profile(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(body): Json<PatchProfileBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    db::get_profile(&state.db, id).map_err(internal)?.ok_or_else(not_found)?;
    if let Some(name) = &body.display_name {
        db::update_profile(&state.db, id, name).map_err(internal)?;
    }
    if let Some(lang) = &body.language {
        db::update_profile_language(&state.db, id, lang).map_err(internal)?;
        bump_and_propagate(&state, id).await.map_err(internal)?;
    }
    Ok(Json(serde_json::json!({ "message": "Profile updated" })))
}

pub async fn delete_profile(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    db::get_profile(&state.db, id).map_err(internal)?.ok_or_else(not_found)?;
    db::delete_profile(&state.db, id).map_err(internal)?;
    Ok(Json(serde_json::json!({ "message": "Profile deleted" })))
}

// ── schedules ─────────────────────────────────────────────────────────────────

pub async fn get_schedules(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    db::get_profile(&state.db, id).map_err(internal)?.ok_or_else(not_found)?;
    let schedules = db::get_schedules(&state.db, id).map_err(internal)?;
    Ok(Json(serde_json::json!({ "schedules": schedules })))
}

#[derive(Deserialize)]
pub struct ScheduleEntry {
    pub day_of_week: u8,
    pub start_time: String,
    pub end_time: String,
}

#[derive(Deserialize)]
pub struct ReplaceSchedulesBody {
    pub schedules: Vec<ScheduleEntry>,
}

pub async fn replace_schedules(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(body): Json<ReplaceSchedulesBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    db::get_profile(&state.db, id).map_err(internal)?.ok_or_else(not_found)?;

    let entries: Vec<(u8, &str, &str)> = body.schedules
        .iter()
        .map(|s| (s.day_of_week, s.start_time.as_str(), s.end_time.as_str()))
        .collect();
    db::replace_schedules(&state.db, id, &entries).map_err(internal)?;

    let version = bump_and_propagate(&state, id).await.map_err(internal)?;
    Ok(Json(serde_json::json!({ "message": "Schedules updated", "config_version": version })))
}

// ── daily limits ──────────────────────────────────────────────────────────────

pub async fn get_daily_limits(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    db::get_profile(&state.db, id).map_err(internal)?.ok_or_else(not_found)?;
    let limits = db::get_daily_limits(&state.db, id).map_err(internal)?;
    Ok(Json(serde_json::json!({ "limits": limits })))
}

#[derive(Deserialize)]
pub struct LimitEntry {
    pub day_of_week: u8,
    pub allowed_minutes: i32,
}

#[derive(Deserialize)]
pub struct ReplaceLimitsBody {
    pub limits: Vec<LimitEntry>,
}

pub async fn replace_daily_limits(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(body): Json<ReplaceLimitsBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    db::get_profile(&state.db, id).map_err(internal)?.ok_or_else(not_found)?;

    let entries: Vec<(u8, i32)> = body.limits.iter().map(|l| (l.day_of_week, l.allowed_minutes)).collect();
    db::replace_daily_limits(&state.db, id, &entries).map_err(internal)?;

    let version = bump_and_propagate(&state, id).await.map_err(internal)?;
    Ok(Json(serde_json::json!({ "message": "Daily limits updated", "config_version": version })))
}

// ── adjustments ───────────────────────────────────────────────────────────────

pub async fn list_adjustments(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    db::get_profile(&state.db, id).map_err(internal)?.ok_or_else(not_found)?;
    let adj = db::get_adjustments(&state.db, id, None, None).map_err(internal)?;
    Ok(Json(serde_json::json!({ "adjustments": adj })))
}

#[derive(Deserialize)]
pub struct CreateAdjustmentBody {
    pub target_date: String,
    pub adjustment_minutes: i32,
    pub reason: Option<String>,
}

pub async fn create_adjustment(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(body): Json<CreateAdjustmentBody>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<serde_json::Value>)> {
    db::get_profile(&state.db, id).map_err(internal)?.ok_or_else(not_found)?;

    let adj_id = db::create_adjustment(
        &state.db, id, &body.target_date, body.adjustment_minutes,
        body.reason.as_deref(), None,
    ).map_err(internal)?;

    bump_and_propagate(&state, id).await.map_err(internal)?;

    Ok((StatusCode::CREATED, Json(serde_json::json!({ "id": adj_id }))))
}

pub async fn lock_now(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    db::get_profile(&state.db, id).map_err(internal)?.ok_or_else(not_found)?;

    let today = Local::now().date_naive().to_string();
    let used_secs = db::get_used_seconds_for_profile_today(&state.db, id, &today)
        .map_err(internal)?;
    let limits = db::get_daily_limits(&state.db, id).map_err(internal)?;
    let weekday = db::weekday_for_date(&today);
    let limit = limits.iter().find(|l| l.day_of_week == weekday)
        .map(|l| l.allowed_minutes)
        .unwrap_or(1440);
    let adj = db::sum_adjustments_for_date(&state.db, id, &today).map_err(internal)?;
    let used_min = (used_secs / 60) as i32;
    let remaining = (limit + adj - used_min).max(0);
    // Insert a negative adjustment to zero out remaining time.
    let needed = -remaining - adj;
    let adj_id = db::create_adjustment(&state.db, id, &today, needed, Some("lock_now"), None)
        .map_err(internal)?;

    // Send lock_now to all online agents with users linked to this profile.
    let agent_users = db::get_agent_users_for_profile(&state.db, id).map_err(internal)?;
    let agent_ids: Vec<Uuid> = agent_users.iter().map(|u| u.agent_id).collect();

    for au in &agent_users {
        let msg = WssMessage::new(
            common::messages::MSG_LOCK_NOW,
            &common::messages::LockNow { local_uid: au.local_uid as u32 },
        ).map_err(internal)?;
        state.send_to_agent_id(au.agent_id, msg).await;
    }

    let _ = agent_ids;
    Ok(Json(serde_json::json!({ "message": "Today's allowance zeroed out", "adjustment_id": adj_id })))
}

// ── notify ────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct NotifyBody {
    pub summary: Option<String>,
    pub body: String,
}

pub async fn notify_profile(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(body): Json<NotifyBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    db::get_profile(&state.db, id).map_err(internal)?.ok_or_else(not_found)?;
    let agent_users = db::get_agent_users_for_profile(&state.db, id).map_err(internal)?;

    let summary = body.summary.as_deref().unwrap_or("Message from administrator");
    let mut sent = 0usize;

    for au in &agent_users {
        if !state.is_online(au.agent_id).await {
            continue;
        }
        let msg = WssMessage::new(MSG_NOTIFY_USER, &NotifyUser {
            local_uid: au.local_uid as u32,
            summary: summary.to_string(),
            body: body.body.clone(),
        }).map_err(internal)?;
        state.send_to_agent_id(au.agent_id, msg).await;
        sent += 1;
    }

    Ok(Json(serde_json::json!({
        "message": format!("Notification sent to {sent}/{} online agent(s)", agent_users.len()),
        "sent": sent,
        "total": agent_users.len(),
    })))
}

// ── agent-users ───────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct PatchAgentUserBody {
    pub profile_id: Option<Uuid>,
    pub status: Option<String>,
}

pub async fn patch_agent_user(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(body): Json<PatchAgentUserBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let au = db::get_agent_user_by_id(&state.db, id).map_err(internal)?.ok_or_else(not_found)?;
    db::update_agent_user(&state.db, id, body.profile_id, body.status.as_deref())
        .map_err(internal)?;

    // If a profile was assigned, propagate config to the agent.
    if let Some(profile_id) = body.profile_id {
        bump_and_propagate(&state, profile_id).await.map_err(internal)?;
    } else if let Some(profile_id) = au.profile_id {
        bump_and_propagate(&state, profile_id).await.map_err(internal)?;
    }

    Ok(Json(serde_json::json!({ "message": "User linked to profile" })))
}

// ── config propagation ────────────────────────────────────────────────────────

/// Bump config_version for a profile, then push updated config and remaining
/// to all currently online agents that have users linked to this profile.
pub async fn bump_and_propagate(state: &AppState, profile_id: Uuid) -> anyhow::Result<i64> {
    let version = db::bump_config_version(&state.db, profile_id)?;

    let agent_users = db::get_agent_users_for_profile(&state.db, profile_id)?;
    // Deduplicate by agent_id.
    let mut seen = std::collections::HashSet::new();
    for au in &agent_users {
        if !seen.insert(au.agent_id) { continue; }
        if !state.is_online(au.agent_id).await { continue; }

        let push = remaining::build_config_push(&state.db, au.agent_id, version)?;
        let msg = WssMessage::new(MSG_CONFIG_PUSH, &push)?;
        state.send_to_agent_id(au.agent_id, msg).await;

        // Also send fresh remaining_update.
        let today = Local::now().date_naive().to_string();
        let managed: Vec<(u32, u32)> = db::list_agent_users(&state.db, au.agent_id)?
            .iter()
            .filter(|u| u.profile_id == Some(profile_id) && u.status == "managed")
            .map(|u| (u.local_uid as u32, 0))
            .collect();

        if !managed.is_empty() {
            let agent = db::get_agent_by_id(&state.db, au.agent_id)?;
            let tz = agent.as_ref().map(|a| a.timezone.as_str()).unwrap_or("UTC").to_string();
            let admin_tz = db::get_admin_timezone(&state.db).unwrap_or_else(|_| "UTC".to_string());
            let entries = remaining::calculate_remaining_for_agent(&state.db, au.agent_id, &tz, &admin_tz, &managed)?;
            let msg = WssMessage::new(MSG_REMAINING_UPDATE, &RemainingUpdate { users: entries })?;
            state.send_to_agent_id(au.agent_id, msg).await;
        }
        let _ = today;
    }

    Ok(version)
}
