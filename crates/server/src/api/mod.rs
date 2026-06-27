use axum::{
    extract::{Request, State},
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::Response,
    routing::{get, patch, post},
    Json, Router,
};
use jsonwebtoken::{decode, DecodingKey, Validation};
use std::sync::Arc;

use crate::state::AppState;

pub mod auth;
pub mod agents;
pub mod profiles;
pub mod usage;

pub fn router(state: Arc<AppState>) -> Router {
    let public = Router::new()
        .route("/auth/status", get(auth::status))
        .route("/auth/setup", post(auth::setup))
        .route("/auth/login", post(auth::login));

    let protected = Router::new()
        // Auth (me)
        .route("/auth/me", get(auth::get_me).patch(auth::patch_me))
        // Agents
        .route("/agents", get(agents::list_agents))
        .route("/agents/{id}", get(agents::get_agent).patch(agents::patch_agent).delete(agents::delete_agent))
        .route("/agents/{id}/accept", post(agents::accept_agent))
        .route("/agents/{id}/undo-delete", post(agents::undo_delete_agent))
        .route("/agents/{id}/force-delete", post(agents::force_delete_agent))
        .route("/agents/{id}/users", get(agents::list_agent_users))
        .route("/agents/{id}/logs", get(agents::fetch_agent_logs))
        // Agent users
        .route("/agent-users/{id}", patch(profiles::patch_agent_user))
        // Profiles
        .route("/profiles", get(profiles::list_profiles).post(profiles::create_profile))
        .route("/profiles/{id}", get(profiles::get_profile).patch(profiles::patch_profile).delete(profiles::delete_profile))
        // Schedules
        .route("/profiles/{id}/schedules", get(profiles::get_schedules).put(profiles::replace_schedules))
        // Limits
        .route("/profiles/{id}/daily-limits", get(profiles::get_daily_limits).put(profiles::replace_daily_limits))
        // Adjustments
        .route("/profiles/{id}/adjustments", get(profiles::list_adjustments).post(profiles::create_adjustment))
        .route("/profiles/{id}/lock-now", post(profiles::lock_now))
        .route("/profiles/{id}/notify", post(profiles::notify_profile))
        // Usage & dashboard
        .route("/profiles/{id}/usage", get(usage::get_usage))
        .route("/profiles/{id}/status", get(usage::get_status))
        .route("/dashboard", get(usage::dashboard))
        .layer(middleware::from_fn_with_state(state.clone(), require_auth));

    Router::new()
        .nest("/api/v1", public.merge(protected))
        .with_state(state)
}

async fn require_auth(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    mut req: Request,
    next: Next,
) -> Result<Response, (StatusCode, Json<serde_json::Value>)> {
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| {
            (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": "Missing token" })))
        })?;

    let claims = decode::<auth::Claims>(
        token,
        &DecodingKey::from_secret(state.jwt_secret.as_bytes()),
        &Validation::default(),
    )
    .map_err(|_| (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": "Invalid token" }))))?
    .claims;

    req.extensions_mut().insert(claims);
    Ok(next.run(req).await)
}
