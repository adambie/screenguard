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

// ── user management (owner-only) ────────────────────────────────────────────
//
// Every admin account has full access to the rest of ScreenGuard's API —
// this module is the one exception: creating, listing, deleting, and
// resetting the password of OTHER accounts is gated to whichever account
// has `is_owner = true` (today: exactly the original account from setup).

/// Loads the caller's own admin row from `claims.sub` and rejects the
/// request with 403 unless it is the owner. Looked up fresh from the DB on
/// every call, not cached in the JWT — a revoked owner flag then takes
/// effect immediately rather than only once the (potentially long-lived)
/// token expires.
async fn require_owner(
    state: &AppState,
    claims: &Claims,
) -> Result<db::AdminUser, (StatusCode, Json<serde_json::Value>)> {
    let id: Uuid = claims
        .sub
        .parse()
        .map_err(|_| (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": "Invalid token" }))))?;
    let admin = db::get_admin_user_by_id(&state.db, id)
        .await
        .map_err(internal)?
        .ok_or_else(not_found)?;
    if !admin.is_owner {
        return Err((
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "Owner privileges required" })),
        ));
    }
    Ok(admin)
}

fn admin_json(a: &db::AdminUser) -> serde_json::Value {
    serde_json::json!({
        "id": a.id,
        "username": a.username,
        "is_owner": a.is_owner,
        "created_at": a.created_at,
    })
}

pub async fn list_users(
    State(state): State<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    require_owner(&state, &claims).await?;
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
    Extension(claims): Extension<Claims>,
    Json(body): Json<CreateUserBody>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<serde_json::Value>)> {
    require_owner(&state, &claims).await?;

    if body.username.trim().is_empty() || body.password.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Username and password are required" })),
        ));
    }
    if db::get_admin_by_username(&state.db, &body.username)
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
    // is_owner is always false here — never taken from the request body, so
    // a caller can't smuggle owner status through this endpoint even if they
    // somehow reached it without actually being an owner.
    let id = db::create_admin(&state.db, &body.username, &hash, false)
        .await
        .map_err(internal)?;
    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({ "id": id, "username": body.username, "is_owner": false })),
    ))
}

pub async fn delete_user(
    State(state): State<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let caller = require_owner(&state, &claims).await?;

    // The owner is the only account allowed to delete anything, so "delete my
    // own account" only ever means the owner deleting themselves. With no
    // ownership-transfer mechanism in this version, that would silently zero
    // out every `is_owner=true` row while other accounts remain — locking
    // user management for good, not just for the deleter. Block it outright
    // rather than relying on the (separate) last-admin-overall check below,
    // which wouldn't catch this since other, non-owner accounts can still
    // exist.
    if id == caller.id {
        return Err((
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "Cannot delete your own account",
                "code": "self_delete",
            })),
        ));
    }

    if db::get_admin_user_by_id(&state.db, id).await.map_err(internal)?.is_none() {
        return Err(not_found());
    }
    let count = db::admin_count(&state.db).await.map_err(internal)?;
    if count <= 1 {
        return Err((
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "Cannot delete the last remaining admin account",
                "code": "last_admin",
            })),
        ));
    }

    db::delete_admin(&state.db, id).await.map_err(internal)?;
    Ok(Json(serde_json::json!({ "message": "User deleted" })))
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
    require_owner(&state, &claims).await?;

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
    db::update_admin_password(&state.db, id, &hash).await.map_err(internal)?;
    Ok(Json(serde_json::json!({ "message": "Password reset" })))
}
