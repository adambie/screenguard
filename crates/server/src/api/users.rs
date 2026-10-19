use axum::{
    extract::{Extension, Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use crate::api::auth::{self, internal, not_found, Claims};
use crate::db;
use crate::state::AppState;

// ── user management ──────────────────────────────────────────────────────────
//
// All admin accounts are equal: any admin can add, remove, or reset the
// password of the others. Passwords set here are chosen by someone other than
// the account holder, so they are flagged `must_change_password` and the
// holder must replace them on first login (enforced in `require_auth`).

fn caller_id(claims: &Claims) -> Result<Uuid, (StatusCode, Json<serde_json::Value>)> {
    claims
        .sub
        .parse()
        .map_err(|_| (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": "Invalid token" }))))
}

fn admin_json(a: &db::AdminUser) -> serde_json::Value {
    serde_json::json!({
        "id": a.id,
        "username": a.username,
        "must_change_password": a.must_change_password,
        "created_at": a.created_at,
    })
}

pub async fn list_users(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let admins = db::list_admins(&state.db).await.map_err(internal)?;
    Ok(Json(serde_json::json!({
        "users": admins.iter().map(admin_json).collect::<Vec<_>>(),
    })))
}

#[derive(Deserialize)]
pub struct CreateUserBody {
    pub username: String,
    pub password: String,
}

pub async fn create_user(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateUserBody>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<serde_json::Value>)> {
    let username = body.username.trim();
    if username.is_empty() || body.password.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Username and password are required" })),
        ));
    }
    if db::get_admin_by_username(&state.db, username)
        .await
        .map_err(internal)?
        .is_some()
    {
        return Err((
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "error": "Username already exists" })),
        ));
    }

    let hash = auth::hash_password(&body.password).map_err(internal)?;
    let id = db::create_admin(&state.db, username, &hash, true)
        .await
        .map_err(internal)?;
    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({ "id": id, "username": username })),
    ))
}

pub async fn delete_user(
    State(state): State<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    // Deleting yourself is almost always a mistake, and as the last admin it
    // would reopen /auth/setup to anyone on the network.
    if id == caller_id(&claims)? {
        return Err((
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "Cannot delete your own account",
                "code": "self_delete",
            })),
        ));
    }

    match db::delete_admin(&state.db, id).await.map_err(internal)? {
        db::DeleteAdmin::Deleted => Ok(Json(serde_json::json!({ "message": "User deleted" }))),
        db::DeleteAdmin::NotFound => Err(not_found()),
        db::DeleteAdmin::LastAdmin => Err((
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "Cannot delete the last remaining admin account",
                "code": "last_admin",
            })),
        )),
    }
}

#[derive(Deserialize)]
pub struct ResetPasswordBody {
    pub password: String,
}

pub async fn reset_password(
    State(state): State<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
    Json(body): Json<ResetPasswordBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    // Your own password goes through POST /auth/password (which checks the
    // current one); resetting it here would also lock you behind the
    // must-change flag.
    if id == caller_id(&claims)? {
        return Err((
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "Use change password for your own account",
                "code": "self_reset",
            })),
        ));
    }
    if body.password.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Password is required" })),
        ));
    }
    if db::get_admin_user_by_id(&state.db, id).await.map_err(internal)?.is_none() {
        return Err(not_found());
    }

    let hash = auth::hash_password(&body.password).map_err(internal)?;
    db::update_admin_password(&state.db, id, &hash, true).await.map_err(internal)?;
    Ok(Json(serde_json::json!({ "message": "Password reset" })))
}
