//! Apps, app groups, group limits and new-app approval (#27). Everything is per
//! profile: each child has their own apps (seen on their computers) and groups.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use common::apps::GroupRule;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use crate::api::auth::{internal, not_found};
use crate::api::profiles::bump_and_propagate;
use crate::app_groups;
use crate::app_usage;
use crate::state::AppState;

type ApiError = (StatusCode, Json<serde_json::Value>);
type ApiResult = Result<Json<serde_json::Value>, ApiError>;
type Created = Result<(StatusCode, Json<serde_json::Value>), ApiError>;

fn bad(e: impl std::fmt::Display) -> ApiError {
    (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e.to_string() })))
}

async fn profile(state: &AppState, id: Uuid) -> Result<(), ApiError> {
    crate::db::get_profile(&state.db, id).await.map_err(internal)?.ok_or_else(not_found)?;
    Ok(())
}

async fn group_of_profile(state: &AppState, id: Uuid, group_id: &str) -> Result<(), ApiError> {
    if !app_groups::group_exists(&state.db, id, group_id).await.map_err(internal)? {
        return Err(not_found());
    }
    Ok(())
}

/// `GET /profiles/{id}/apps` — the child's apps board: every app the child used
/// (with today's time and its group) and the child's groups (rules, limits,
/// today's extra time and today's group time).
pub async fn board(State(state): State<Arc<AppState>>, Path(id): Path<Uuid>) -> ApiResult {
    profile(&state, id).await?;
    let db = &state.db;
    let today = chrono::Local::now().date_naive();
    let apps = app_usage::profile_catalog(db, id).await.map_err(internal)?;
    let groups = app_groups::list_groups(db, id).await.map_err(internal)?;
    let limits = app_groups::limits(db, &id.to_string()).await.map_err(internal)?;
    let adjustments = app_groups::adjustments(db, &id.to_string(), &today.to_string()).await.map_err(internal)?;
    let today_apps = app_usage::totals(db, id, Some(today), Some(today)).await.map_err(internal)?;
    let today_groups = app_usage::group_totals(db, id, Some(today), Some(today)).await.map_err(internal)?;
    let app_secs = |a: &str| today_apps.iter().find(|t| t.app_id == a).map_or(0, |t| t.seconds);

    let apps_out: Vec<serde_json::Value> = apps
        .iter()
        .map(|a| {
            serde_json::json!({
                "app_id": a.app_id, "name": a.name, "icon": a.icon, "categories": a.categories,
                "group": app_groups::group_of(&groups, &a.app_id, &a.categories),
                "today_seconds": app_secs(&a.app_id),
            })
        })
        .collect();
    let groups_out: Vec<serde_json::Value> = groups
        .iter()
        .map(|g| {
            // group time as the agents counted it (once while any app ran); for
            // agents that don't report groups yet, the sum of the group's apps
            let today_seconds = today_groups.get(&g.id).copied().unwrap_or_else(|| {
                apps.iter()
                    .filter(|a| app_groups::group_of(&groups, &a.app_id, &a.categories) == Some(g.id.as_str()))
                    .map(|a| app_secs(&a.app_id))
                    .sum()
            });
            let l: Vec<serde_json::Value> = limits
                .get(&g.id)
                .into_iter()
                .flatten()
                .map(|(d, m)| serde_json::json!({ "day_of_week": d, "allowed_minutes": m }))
                .collect();
            serde_json::json!({
                "id": g.id, "name": g.name, "builtin": g.builtin, "rules": g.rules, "limits": l,
                "adjustment_today": adjustments.get(&g.id).copied().unwrap_or(0),
                "today_seconds": today_seconds,
            })
        })
        .collect();
    Ok(Json(serde_json::json!({
        "today": today, "weekday": chrono::Datelike::weekday(&today).num_days_from_monday(),
        "apps": apps_out, "groups": groups_out,
    })))
}

#[derive(Deserialize)]
pub struct GroupBody {
    pub name: String,
}

pub async fn create_group(State(state): State<Arc<AppState>>, Path(id): Path<Uuid>, Json(body): Json<GroupBody>) -> Created {
    profile(&state, id).await?;
    let gid = app_groups::create_group(&state.db, id, &body.name).await.map_err(bad)?;
    bump_and_propagate(&state, id).await.map_err(internal)?;
    Ok((StatusCode::CREATED, Json(serde_json::json!({ "id": gid }))))
}

pub async fn rename_group(
    State(state): State<Arc<AppState>>,
    Path((id, gid)): Path<(Uuid, String)>,
    Json(body): Json<GroupBody>,
) -> ApiResult {
    if !app_groups::rename_group(&state.db, id, &gid, &body.name).await.map_err(bad)? {
        return Err(not_found());
    }
    bump_and_propagate(&state, id).await.map_err(internal)?;
    Ok(Json(serde_json::json!({ "message": "Group renamed" })))
}

pub async fn delete_group(State(state): State<Arc<AppState>>, Path((id, gid)): Path<(Uuid, String)>) -> ApiResult {
    if !app_groups::delete_group(&state.db, id, &gid).await.map_err(internal)? {
        return Err(not_found());
    }
    bump_and_propagate(&state, id).await.map_err(internal)?;
    Ok(Json(serde_json::json!({ "message": "Group deleted" })))
}

/// `POST /profiles/{id}/app-groups/{gid}/rules` — add a category / prefix / app rule.
pub async fn add_rule(State(state): State<Arc<AppState>>, Path((id, gid)): Path<(Uuid, String)>, Json(rule): Json<GroupRule>) -> Created {
    group_of_profile(&state, id, &gid).await?;
    let rule_id = app_groups::add_rule(&state.db, &gid, &rule).await.map_err(bad)?;
    bump_and_propagate(&state, id).await.map_err(internal)?;
    Ok((StatusCode::CREATED, Json(serde_json::json!({ "id": rule_id }))))
}

pub async fn delete_rule(State(state): State<Arc<AppState>>, Path((id, gid, rule_id)): Path<(Uuid, String, String)>) -> ApiResult {
    group_of_profile(&state, id, &gid).await?;
    if !app_groups::delete_rule(&state.db, &gid, &rule_id).await.map_err(internal)? {
        return Err(not_found());
    }
    bump_and_propagate(&state, id).await.map_err(internal)?;
    Ok(Json(serde_json::json!({ "message": "Rule deleted" })))
}

#[derive(Deserialize)]
pub struct MembershipBody {
    pub app_id: String,
    pub member: bool,
}

/// `POST /profiles/{id}/app-groups/{gid}/membership` — put an app into the group
/// (moving it from any other) or take it out.
pub async fn set_membership(
    State(state): State<Arc<AppState>>,
    Path((id, gid)): Path<(Uuid, String)>,
    Json(body): Json<MembershipBody>,
) -> ApiResult {
    let categories = app_usage::app_categories(&state.db, &body.app_id).await.map_err(internal)?;
    app_groups::set_membership(&state.db, id, &gid, &body.app_id, &categories, body.member)
        .await
        .map_err(bad)?
        .ok_or_else(not_found)?;
    bump_and_propagate(&state, id).await.map_err(internal)?;
    Ok(Json(serde_json::json!({ "message": "Membership updated" })))
}

#[derive(Deserialize)]
pub struct LimitEntry {
    pub day_of_week: u8,
    pub allowed_minutes: i32,
}

#[derive(Deserialize)]
pub struct LimitsBody {
    pub limits: Vec<LimitEntry>,
}

/// `PUT /profiles/{id}/app-group-limits/{group_id}` — replace (empty = no limit).
pub async fn put_group_limits(
    State(state): State<Arc<AppState>>,
    Path((id, gid)): Path<(Uuid, String)>,
    Json(body): Json<LimitsBody>,
) -> ApiResult {
    profile(&state, id).await?;
    group_of_profile(&state, id, &gid).await?;
    let entries: Vec<(u8, i32)> = body.limits.iter().map(|l| (l.day_of_week, l.allowed_minutes)).collect();
    app_groups::replace_limits(&state.db, &id.to_string(), &gid, &entries).await.map_err(bad)?;
    let version = bump_and_propagate(&state, id).await.map_err(internal)?;
    Ok(Json(serde_json::json!({ "message": "Group limits updated", "config_version": version })))
}

#[derive(Deserialize)]
pub struct AdjustmentBody {
    pub group_id: String,
    pub minutes: i32,
    /// YYYY-MM-DD, default today (server's local date).
    pub date: Option<String>,
}

/// `POST /profiles/{id}/app-group-adjustments` — extra time for a group today.
pub async fn add_group_adjustment(State(state): State<Arc<AppState>>, Path(id): Path<Uuid>, Json(body): Json<AdjustmentBody>) -> Created {
    profile(&state, id).await?;
    group_of_profile(&state, id, &body.group_id).await?;
    let date = match body.date {
        Some(d) => d.parse::<chrono::NaiveDate>().map_err(|_| bad("date must be YYYY-MM-DD"))?.to_string(),
        None => chrono::Local::now().date_naive().to_string(),
    };
    app_groups::add_adjustment(&state.db, &id.to_string(), &body.group_id, &date, body.minutes).await.map_err(bad)?;
    let version = bump_and_propagate(&state, id).await.map_err(internal)?;
    Ok((StatusCode::CREATED, Json(serde_json::json!({ "message": "Adjustment added", "config_version": version }))))
}

/// `GET /profiles/{id}/app-approvals` — the mode and every request/decision.
pub async fn get_approvals(State(state): State<Arc<AppState>>, Path(id): Path<Uuid>) -> ApiResult {
    profile(&state, id).await?;
    let mode = crate::app_approvals::mode(&state.db, id).await.map_err(internal)?;
    let apps = crate::app_approvals::list(&state.db, id).await.map_err(internal)?;
    Ok(Json(serde_json::json!({ "enabled": mode, "apps": apps })))
}

#[derive(Deserialize)]
pub struct ModeBody {
    pub enabled: bool,
}

/// `PUT /profiles/{id}/app-approvals/mode` — turning it on approves the apps already used.
pub async fn set_approval_mode(State(state): State<Arc<AppState>>, Path(id): Path<Uuid>, Json(body): Json<ModeBody>) -> ApiResult {
    profile(&state, id).await?;
    crate::app_approvals::set_mode(&state.db, id, body.enabled).await.map_err(internal)?;
    let version = bump_and_propagate(&state, id).await.map_err(internal)?;
    Ok(Json(serde_json::json!({ "enabled": body.enabled, "config_version": version })))
}

#[derive(Deserialize)]
pub struct DecisionBody {
    pub app_id: String,
    pub approve: bool,
    /// Optionally also put an approved app into one of the child's groups.
    pub group_id: Option<String>,
}

/// `POST /profiles/{id}/app-approvals/decision`
pub async fn decide_approval(State(state): State<Arc<AppState>>, Path(id): Path<Uuid>, Json(body): Json<DecisionBody>) -> ApiResult {
    profile(&state, id).await?;
    crate::app_approvals::decide(&state.db, id, &body.app_id, body.approve).await.map_err(bad)?;
    if let (true, Some(g)) = (body.approve, body.group_id.as_deref().filter(|g| !g.is_empty())) {
        let categories = app_usage::app_categories(&state.db, &body.app_id).await.map_err(internal)?;
        app_groups::set_membership(&state.db, id, g, &body.app_id, &categories, true).await.map_err(bad)?.ok_or_else(not_found)?;
    }
    bump_and_propagate(&state, id).await.map_err(internal)?;
    Ok(Json(serde_json::json!({ "message": "Decision saved" })))
}

/// `GET /app-requests` — pending requests of every profile (dashboard).
pub async fn pending_requests(State(state): State<Arc<AppState>>) -> ApiResult {
    let all = crate::app_approvals::pending_all(&state.db).await.map_err(internal)?;
    let out: Vec<serde_json::Value> = all
        .into_iter()
        .map(|(pid, pname, a)| serde_json::json!({ "profile_id": pid, "profile_name": pname, "app_id": a.app_id, "name": a.name, "requested_at": a.requested_at }))
        .collect();
    Ok(Json(serde_json::json!({ "requests": out })))
}
