use axum::{extract::State, http::StatusCode, Json};
use chrono::Utc;
use jsonwebtoken::{encode, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::db;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct AuthRequest {
    pub username: String,
    pub password: String,
}

#[derive(Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub expires_at: String,
}

#[derive(Serialize, Deserialize)]
pub struct Claims {
    pub sub: String, // admin user id
    pub exp: usize,
}

pub async fn status(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let count = db::admin_count(&state.db).map_err(internal)?;
    Ok(Json(serde_json::json!({ "setup_needed": count == 0 })))
}

pub async fn setup(
    State(state): State<Arc<AppState>>,
    Json(body): Json<AuthRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<serde_json::Value>)> {
    let count = db::admin_count(&state.db).map_err(internal)?;
    if count > 0 {
        return Err((
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "error": "Admin account already exists" })),
        ));
    }
    if body.username.is_empty() || body.password.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Username and password are required" })),
        ));
    }

    let hash = hash_password(&body.password).map_err(internal)?;
    db::create_admin(&state.db, &body.username, &hash).map_err(internal)?;

    Ok((StatusCode::CREATED, Json(serde_json::json!({ "message": "Admin account created" }))))
}

pub async fn login(
    State(state): State<Arc<AppState>>,
    Json(body): Json<AuthRequest>,
) -> Result<Json<LoginResponse>, (StatusCode, Json<serde_json::Value>)> {
    let admin = db::get_admin_by_username(&state.db, &body.username)
        .map_err(internal)?
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": "Invalid credentials" }))))?;

    let valid = verify_password(&body.password, &admin.password_hash).map_err(internal)?;
    if !valid {
        return Err((StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": "Invalid credentials" }))));
    }

    let expiry_secs = state.jwt_expiry_hours * 3600;
    let exp = (Utc::now().timestamp() as usize) + expiry_secs as usize;
    let expires_at = chrono::DateTime::<Utc>::from_timestamp(exp as i64, 0)
        .unwrap_or_default()
        .to_rfc3339();

    let claims = Claims { sub: admin.id.to_string(), exp };
    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(state.jwt_secret.as_bytes()),
    ).map_err(internal)?;

    Ok(Json(LoginResponse { token, expires_at }))
}

pub fn hash_password(password: &str) -> anyhow::Result<String> {
    use argon2::{password_hash::{rand_core::OsRng, PasswordHasher, SaltString}, Argon2};
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("Hash error: {e}"))?
        .to_string();
    Ok(hash)
}

pub fn verify_password(password: &str, hash: &str) -> anyhow::Result<bool> {
    use argon2::{password_hash::{PasswordHash, PasswordVerifier}, Argon2};
    let parsed = PasswordHash::new(hash).map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok())
}

pub fn internal<E: std::fmt::Display>(e: E) -> (StatusCode, Json<serde_json::Value>) {
    tracing::error!("Internal error: {e}");
    (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": "Internal server error" })))
}

pub fn not_found() -> (StatusCode, Json<serde_json::Value>) {
    (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "Not found" })))
}
