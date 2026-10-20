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
    Ok((StatusCode::CREATED, Json(serde_json::json!({ "id": id }))))
}

pub async fn rename_group(State(state): State<Arc<AppState>>, Path(id): Path<String>, Json(body): Json<GroupBody>) -> ApiResult {
    if !app_groups::rename_group(&state.db, &id, &body.name).await.map_err(bad)? {
        return Err(not_found());
    }
    Ok(Json(serde_json::json!({ "message": "Group renamed" })))
}

pub async fn delete_group(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> ApiResult {
    if !app_groups::delete_group(&state.db, &id).await.map_err(internal)? {
        return Err(not_found());
    }
    Ok(Json(serde_json::json!({ "message": "Group deleted" })))
}

/// `POST /app-groups/{id}/rules` — add a category / prefix / app rule.
pub async fn add_rule(State(state): State<Arc<AppState>>, Path(id): Path<String>, Json(rule): Json<GroupRule>) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<serde_json::Value>)> {
    if !app_groups::group_exists(&state.db, &id).await.map_err(internal)? {
        return Err(not_found());
    }
    let rule_id = app_groups::add_rule(&state.db, &id, &rule).await.map_err(bad)?;
    Ok((StatusCode::CREATED, Json(serde_json::json!({ "id": rule_id }))))
}

pub async fn delete_rule(State(state): State<Arc<AppState>>, Path((id, rule_id)): Path<(String, String)>) -> ApiResult {
    if !app_groups::delete_rule(&state.db, &id, &rule_id).await.map_err(internal)? {
        return Err(not_found());
    }
    Ok(Json(serde_json::json!({ "message": "Rule deleted" })))
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
    Ok(Json(serde_json::json!({ "message": "Membership updated" })))
}
