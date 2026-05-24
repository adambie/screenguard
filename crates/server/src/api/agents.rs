use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::api::auth::{internal, not_found};
use crate::db;
use crate::state::{AppState, PairingDecision};

#[derive(Serialize)]
pub struct AgentResponse {
    pub id: Uuid,
    pub machine_id: String,
    pub display_name: String,
    pub hostname: String,
    pub timezone: String,
    pub status: String,
    pub online: bool,
    pub last_seen_at: Option<i64>,
    pub agent_version: Option<String>,
    pub user_count: usize,
    pub pairing_code: Option<String>,
}

pub async fn list_agents(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let agents = db::list_agents(&state.db).map_err(internal)?;
    let online = state.online.read().await;
    let pending = state.pending.read().await;
    let mut result = Vec::new();
    for a in agents {
        let user_count = db::list_agent_users(&state.db, a.id)
            .map(|u| u.len())
            .unwrap_or(0);
        let pairing_code = pending.get(&a.machine_id).map(|h| h.pairing_code.clone());
        result.push(AgentResponse {
            online: online.values().any(|h| h.agent_id == a.id),
            id: a.id,
            machine_id: a.machine_id,
            display_name: a.display_name,
            hostname: a.hostname,
            timezone: a.timezone,
            status: a.status,
            last_seen_at: a.last_seen_at,
            agent_version: a.agent_version,
            user_count,
            pairing_code,
        });
    }
    Ok(Json(serde_json::json!({ "agents": result })))
}

pub async fn get_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let a = db::get_agent_by_id(&state.db, id).map_err(internal)?.ok_or_else(not_found)?;
    let online = state.online.read().await;
    let pending = state.pending.read().await;
    let user_count = db::list_agent_users(&state.db, a.id).map(|u| u.len()).unwrap_or(0);
    let pairing_code = pending.get(&a.machine_id).map(|h| h.pairing_code.clone());
    Ok(Json(serde_json::json!(AgentResponse {
        online: online.values().any(|h| h.agent_id == a.id),
        id: a.id, machine_id: a.machine_id, display_name: a.display_name,
        hostname: a.hostname, timezone: a.timezone, status: a.status,
        last_seen_at: a.last_seen_at, agent_version: a.agent_version,
        user_count, pairing_code,
    })))
}

#[derive(Deserialize)]
pub struct PatchAgentBody {
    pub display_name: Option<String>,
    pub status: Option<String>,
}

pub async fn patch_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(body): Json<PatchAgentBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    db::get_agent_by_id(&state.db, id).map_err(internal)?.ok_or_else(not_found)?;
    db::update_agent_fields(&state.db, id, body.display_name.as_deref(), body.status.as_deref())
        .map_err(internal)?;
    Ok(Json(serde_json::json!({ "message": "Agent updated" })))
}

pub async fn accept_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let agent = db::get_agent_by_id(&state.db, id).map_err(internal)?.ok_or_else(not_found)?;

    // Generate a 256-bit auth token.
    let token = generate_token();
    let token_hash = hash_token(&token);

    db::accept_agent(&state.db, id, &token_hash).map_err(internal)?;

    // If the agent is waiting in pending map, deliver the decision via oneshot.
    let handle = state.pending.write().await.remove(&agent.machine_id);
    if let Some(ph) = handle {
        let _ = ph.tx.send(PairingDecision { auth_token: token.clone(), agent_db_id: id });
        tracing::info!("Delivered pairing_accepted to agent {id}");
    } else {
        tracing::warn!("Agent {id} accepted but no pending WS connection found");
    }

    Ok(Json(serde_json::json!({ "message": "Agent accepted", "agent_id": id })))
}

pub async fn delete_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    db::get_agent_by_id(&state.db, id).map_err(internal)?.ok_or_else(not_found)?;
    db::delete_agent(&state.db, id).map_err(internal)?;
    Ok(Json(serde_json::json!({ "message": "Agent deleted" })))
}

pub async fn list_agent_users(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    db::get_agent_by_id(&state.db, id).map_err(internal)?.ok_or_else(not_found)?;
    let users = db::list_agent_users(&state.db, id).map_err(internal)?;
    Ok(Json(serde_json::json!({ "users": users })))
}

fn generate_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

pub fn hash_token(token: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}
