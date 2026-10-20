//! Apps and app groups (#27 Phase 2): the catalogue of every app seen, and
//! global groups with rules.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use common::apps::{GroupRule, group_matches};
use serde::Deserialize;
use std::sync::Arc;

use crate::api::auth::{internal, not_found};
use crate::app_groups;
use crate::app_usage;
use crate::state::AppState;

type ApiResult = Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)>;

/// Group definitions are shared by every profile: push new config to all of them.
async fn propagate_all(state: &AppState) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    for p in crate::db::list_profiles(&state.db).await.map_err(internal)? {
        crate::api::profiles::bump_and_propagate(state, p.id).await.map_err(internal)?;
    }
    Ok(())
}

fn bad(e: impl std::fmt::Display) -> (StatusCode, Json<serde_json::Value>) {
    (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e.to_string() })))
}

/// `GET /apps` — every app seen, with the groups each one is in, plus the groups.
pub async fn list_apps(State(state): State<Arc<AppState>>) -> ApiResult {
    let apps = app_usage::catalog(&state.db).await.map_err(internal)?;
    let groups = app_groups::list_groups(&state.db).await.map_err(internal)?;
    let rules: Vec<(String, Vec<GroupRule>)> = groups.iter().map(|g| (g.id.clone(), g.rules())).collect();
    let apps: Vec<serde_json::Value> = apps
        .into_iter()
        .map(|a| {
            let in_groups: Vec<&String> =
                rules.iter().filter(|(_, r)| group_matches(r, &a.app_id, &a.categories)).map(|(id, _)| id).collect();
            serde_json::json!({
                "app_id": a.app_id, "name": a.name, "icon": a.icon, "categories": a.categories,
                "last_seen": a.last_seen, "groups": in_groups,
            })
        })
        .collect();
    Ok(Json(serde_json::json!({ "apps": apps, "groups": groups })))
}

pub async fn list_groups(State(state): State<Arc<AppState>>) -> ApiResult {
    let groups = app_groups::list_groups(&state.db).await.map_err(internal)?;
    Ok(Json(serde_json::json!({ "groups": groups })))
}

#[derive(Deserialize)]
pub struct GroupBody {
    pub name: String,
}

pub async fn create_group(State(state): State<Arc<AppState>>, Json(body): Json<GroupBody>) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<serde_json::Value>)> {
    let id = app_groups::create_group(&state.db, &body.name).await.map_err(bad)?;
    propagate_all(&state).await?;
    Ok((StatusCode::CREATED, Json(serde_json::json!({ "id": id }))))
}

pub async fn rename_group(State(state): State<Arc<AppState>>, Path(id): Path<String>, Json(body): Json<GroupBody>) -> ApiResult {
    if !app_groups::rename_group(&state.db, &id, &body.name).await.map_err(bad)? {
        return Err(not_found());
    }
    propagate_all(&state).await?;
    Ok(Json(serde_json::json!({ "message": "Group renamed" })))
}

pub async fn delete_group(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> ApiResult {
    if !app_groups::delete_group(&state.db, &id).await.map_err(internal)? {
        return Err(not_found());
    }
    propagate_all(&state).await?;
    Ok(Json(serde_json::json!({ "message": "Group deleted" })))
}

/// `POST /app-groups/{id}/rules` — add a category / prefix / app rule.
pub async fn add_rule(State(state): State<Arc<AppState>>, Path(id): Path<String>, Json(rule): Json<GroupRule>) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<serde_json::Value>)> {
    if !app_groups::group_exists(&state.db, &id).await.map_err(internal)? {
        return Err(not_found());
    }
    let rule_id = app_groups::add_rule(&state.db, &id, &rule).await.map_err(bad)?;
    propagate_all(&state).await?;
    Ok((StatusCode::CREATED, Json(serde_json::json!({ "id": rule_id }))))
}

pub async fn delete_rule(State(state): State<Arc<AppState>>, Path((id, rule_id)): Path<(String, String)>) -> ApiResult {
    if !app_groups::delete_rule(&state.db, &id, &rule_id).await.map_err(internal)? {
        return Err(not_found());
    }
    propagate_all(&state).await?;
    Ok(Json(serde_json::json!({ "message": "Rule deleted" })))
}

/// `GET /profiles/{id}/app-group-limits` — every group with this profile's limits.
pub async fn get_group_limits(State(state): State<Arc<AppState>>, Path(id): Path<uuid::Uuid>) -> ApiResult {
    crate::db::get_profile(&state.db, id).await.map_err(internal)?.ok_or_else(not_found)?;
    let groups = app_groups::list_groups(&state.db).await.map_err(internal)?;
    let limits = app_groups::limits(&state.db, &id.to_string()).await.map_err(internal)?;
    let today = chrono::Local::now().date_naive().to_string();
    let adjustments = app_groups::adjustments(&state.db, &id.to_string(), &today).await.map_err(internal)?;
    let out: Vec<serde_json::Value> = groups
        .iter()
        .map(|g| {
            let l: Vec<serde_json::Value> = limits
                .get(&g.id)
                .into_iter()
                .flatten()
                .map(|(dow, mins)| serde_json::json!({ "day_of_week": dow, "allowed_minutes": mins }))
                .collect();
            let adj = adjustments.get(&g.id).copied().unwrap_or(0);
            serde_json::json!({ "id": g.id, "name": g.name, "builtin": g.builtin, "limits": l, "adjustment_today": adj })
        })
        .collect();
    Ok(Json(serde_json::json!({ "groups": out })))
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
    Path((id, group_id)): Path<(uuid::Uuid, String)>,
    Json(body): Json<LimitsBody>,
) -> ApiResult {
    crate::db::get_profile(&state.db, id).await.map_err(internal)?.ok_or_else(not_found)?;
    if !app_groups::group_exists(&state.db, &group_id).await.map_err(internal)? {
        return Err(not_found());
    }
    let entries: Vec<(u8, i32)> = body.limits.iter().map(|l| (l.day_of_week, l.allowed_minutes)).collect();
    app_groups::replace_limits(&state.db, &id.to_string(), &group_id, &entries).await.map_err(bad)?;
    let version = crate::api::profiles::bump_and_propagate(&state, id).await.map_err(internal)?;
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
pub async fn add_group_adjustment(
    State(state): State<Arc<AppState>>,
    Path(id): Path<uuid::Uuid>,
    Json(body): Json<AdjustmentBody>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<serde_json::Value>)> {
    crate::db::get_profile(&state.db, id).await.map_err(internal)?.ok_or_else(not_found)?;
    if !app_groups::group_exists(&state.db, &body.group_id).await.map_err(internal)? {
        return Err(not_found());
    }
    let date = match body.date {
        Some(d) => d.parse::<chrono::NaiveDate>().map_err(|_| bad("date must be YYYY-MM-DD"))?.to_string(),
        None => chrono::Local::now().date_naive().to_string(),
    };
    app_groups::add_adjustment(&state.db, &id.to_string(), &body.group_id, &date, body.minutes).await.map_err(bad)?;
    let version = crate::api::profiles::bump_and_propagate(&state, id).await.map_err(internal)?;
    Ok((StatusCode::CREATED, Json(serde_json::json!({ "message": "Adjustment added", "config_version": version }))))
}

/// `GET /profiles/{id}/app-approvals` — the mode and every request/decision.
pub async fn get_approvals(State(state): State<Arc<AppState>>, Path(id): Path<uuid::Uuid>) -> ApiResult {
    crate::db::get_profile(&state.db, id).await.map_err(internal)?.ok_or_else(not_found)?;
    let mode = crate::app_approvals::mode(&state.db, id).await.map_err(internal)?;
    let apps = crate::app_approvals::list(&state.db, id).await.map_err(internal)?;
    Ok(Json(serde_json::json!({ "enabled": mode, "apps": apps })))
}

#[derive(Deserialize)]
pub struct ModeBody {
    pub enabled: bool,
}

/// `PUT /profiles/{id}/app-approvals/mode` — turning it on approves the apps already used.
pub async fn set_approval_mode(State(state): State<Arc<AppState>>, Path(id): Path<uuid::Uuid>, Json(body): Json<ModeBody>) -> ApiResult {
    crate::db::get_profile(&state.db, id).await.map_err(internal)?.ok_or_else(not_found)?;
    crate::app_approvals::set_mode(&state.db, id, body.enabled).await.map_err(internal)?;
    let version = crate::api::profiles::bump_and_propagate(&state, id).await.map_err(internal)?;
    Ok(Json(serde_json::json!({ "enabled": body.enabled, "config_version": version })))
}

#[derive(Deserialize)]
pub struct DecisionBody {
    pub app_id: String,
    pub approve: bool,
    /// Optionally also put an approved app into a group.
    pub group_id: Option<String>,
}

/// `POST /profiles/{id}/app-approvals/decision`
pub async fn decide_approval(State(state): State<Arc<AppState>>, Path(id): Path<uuid::Uuid>, Json(body): Json<DecisionBody>) -> ApiResult {
    crate::db::get_profile(&state.db, id).await.map_err(internal)?.ok_or_else(not_found)?;
    crate::app_approvals::decide(&state.db, id, &body.app_id, body.approve).await.map_err(bad)?;
    if let (true, Some(g)) = (body.approve, body.group_id.as_deref().filter(|g| !g.is_empty())) {
        let categories = app_usage::app_categories(&state.db, &body.app_id).await.map_err(internal)?;
        app_groups::set_membership(&state.db, g, &body.app_id, &categories, true).await.map_err(bad)?.ok_or_else(not_found)?;
        propagate_all(&state).await?;
    } else {
        crate::api::profiles::bump_and_propagate(&state, id).await.map_err(internal)?;
    }
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

#[derive(Deserialize)]
pub struct MembershipBody {
    pub app_id: String,
    pub member: bool,
}

/// `POST /app-groups/{id}/membership` — put an app into the group or take it out.
pub async fn set_membership(State(state): State<Arc<AppState>>, Path(id): Path<String>, Json(body): Json<MembershipBody>) -> ApiResult {
    let categories = app_usage::app_categories(&state.db, &body.app_id).await.map_err(internal)?;
    app_groups::set_membership(&state.db, &id, &body.app_id, &categories, body.member)
        .await
        .map_err(bad)?
        .ok_or_else(not_found)?;
    propagate_all(&state).await?;
    Ok(Json(serde_json::json!({ "message": "Membership updated" })))
}
