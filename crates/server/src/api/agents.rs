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
use crate::release_check;
use crate::state::{AppState, DEFAULT_TENANT, PairingDecision};

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
    /// The agent is connected and waiting for approval with its pairing code.
    /// The code itself is never returned: the admin reads it from the agent.
    pub pairing_requested: bool,
    pub upgradeable: bool,
    pub web_filter_available: Option<bool>,
}

pub async fn list_agents(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let agents = db::list_agents(&state.db).await.map_err(internal)?;
    let online = state.online.read().await;
    let pending = state.pending.read().await;
    let latest = state.latest_agent_release.read().await;
    let mut result = Vec::new();
    for a in agents {
        let user_count = db::list_agent_users(&state.db, a.id)
            .await
            .map(|u| u.len())
            .unwrap_or(0);
        let pairing_requested = pending.contains_key(&(DEFAULT_TENANT.to_string(), a.machine_id.clone()));
        let upgradeable = a.agent_version.as_deref()
            .zip(latest.as_deref())
            .map_or(false, |(agent_v, latest_v)| release_check::is_older(agent_v, latest_v));
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
            pairing_requested,
            upgradeable,
            web_filter_available: a.web_filter_available,
        });
    }
    Ok(Json(serde_json::json!({ "agents": result })))
}

pub async fn get_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let a = db::get_agent_by_id(&state.db, id).await.map_err(internal)?.ok_or_else(not_found)?;
    let online = state.online.read().await;
    let pending = state.pending.read().await;
    let latest = state.latest_agent_release.read().await;
    let user_count = db::list_agent_users(&state.db, a.id).await.map(|u| u.len()).unwrap_or(0);
    let pairing_requested = pending.contains_key(&(DEFAULT_TENANT.to_string(), a.machine_id.clone()));
    let upgradeable = a.agent_version.as_deref()
        .zip(latest.as_deref())
        .map_or(false, |(agent_v, latest_v)| release_check::is_older(agent_v, latest_v));
    Ok(Json(serde_json::json!(AgentResponse {
        online: online.values().any(|h| h.agent_id == a.id),
        id: a.id, machine_id: a.machine_id, display_name: a.display_name,
        hostname: a.hostname, timezone: a.timezone, status: a.status,
        last_seen_at: a.last_seen_at, agent_version: a.agent_version,
        user_count, pairing_requested, upgradeable,
        web_filter_available: a.web_filter_available,
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
    db::get_agent_by_id(&state.db, id).await.map_err(internal)?.ok_or_else(not_found)?;
    db::update_agent_fields(&state.db, id, body.display_name.as_deref(), body.status.as_deref())
        .await.map_err(internal)?;
    Ok(Json(serde_json::json!({ "message": "Agent updated" })))
}

/// Wrong codes allowed per pairing request before it is dropped (the agent then
/// starts over with a fresh code).
const MAX_PAIRING_ATTEMPTS: u32 = 5;

#[derive(Deserialize)]
pub struct AcceptAgentBody {
    pub pairing_code: String,
}

/// Uppercase and drop separators, so "abc-def" or "ABC DEF" match "ABCDEF".
fn normalize_code(code: &str) -> String {
    code.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

fn codes_match(entered: &str, expected: &str) -> bool {
    let (a, b) = (normalize_code(entered), normalize_code(expected));
    // Constant-time over the bytes so response timing doesn't leak a prefix.
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn conflict(code: &str, error: &str) -> (StatusCode, Json<serde_json::Value>) {
    (StatusCode::CONFLICT, Json(serde_json::json!({ "error": error, "code": code })))
}

pub async fn accept_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(body): Json<AcceptAgentBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let agent = db::get_agent_by_id(&state.db, id).await.map_err(internal)?.ok_or_else(not_found)?;
    let key = (DEFAULT_TENANT.to_string(), agent.machine_id.clone());

    let ph = {
        let mut pending = state.pending.write().await;
        let Some(ph) = pending.get_mut(&key) else {
            return Err(conflict("not_waiting", "Agent is not currently waiting for approval"));
        };
        if !codes_match(&body.pairing_code, &ph.pairing_code) {
            ph.failed_attempts += 1;
            let attempts_left = MAX_PAIRING_ATTEMPTS.saturating_sub(ph.failed_attempts);
            if attempts_left == 0 {
                // Dropping the handle ends the agent's pairing connection; it
                // reconnects with a new code.
                pending.remove(&key);
            }
            tracing::warn!("Wrong pairing code for agent {id} ({attempts_left} attempts left)");
            return Err((StatusCode::FORBIDDEN, Json(serde_json::json!({
                "error": "Wrong pairing code",
                "code": "wrong_code",
                "attempts_left": attempts_left,
            }))));
        }
        pending.remove(&key).expect("checked above")
    };

    let token = generate_token();
    db::accept_agent(&state.db, id, &hash_token(&token)).await.map_err(internal)?;
    if ph.tx.send(PairingDecision { auth_token: token, agent_db_id: id }).is_err() {
        // The agent disconnected in the last instant; it will re-pair.
        return Err(conflict("not_waiting", "Agent disconnected before approval"));
    }
    tracing::info!("Delivered pairing_accepted to agent {id}");

    Ok(Json(serde_json::json!({ "message": "Agent accepted", "agent_id": id })))
}

pub async fn delete_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let agent = db::get_agent_by_id(&state.db, id).await.map_err(internal)?.ok_or_else(not_found)?;
    db::mark_agent_pending_delete(&state.db, id).await.map_err(internal)?;

    let online = state.online.read().await;
    if let Some(handle) = online.values().find(|h| h.agent_id == id) {
        use common::messages::{Unpair, MSG_UNPAIR};
        use common::protocol::WssMessage;
        if let Ok(msg) = WssMessage::new(MSG_UNPAIR, &Unpair {}) {
            let _ = handle.outbound_tx.send(msg).await;
        }
    }
    drop(online);

    tracing::info!("Agent {} marked pending_delete (machine_id={})", id, agent.machine_id);
    Ok(Json(serde_json::json!({ "message": "Agent queued for deletion" })))
}

pub async fn undo_delete_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    db::get_agent_by_id(&state.db, id).await.map_err(internal)?.ok_or_else(not_found)?;
    db::restore_agent(&state.db, id).await.map_err(internal)?;
    Ok(Json(serde_json::json!({ "message": "Agent deletion cancelled" })))
}

pub async fn force_delete_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    db::get_agent_by_id(&state.db, id).await.map_err(internal)?.ok_or_else(not_found)?;
    db::delete_agent(&state.db, id).await.map_err(internal)?;
    Ok(Json(serde_json::json!({ "message": "Agent force deleted" })))
}

pub async fn list_agent_users(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    db::get_agent_by_id(&state.db, id).await.map_err(internal)?.ok_or_else(not_found)?;
    let users = db::list_agent_users(&state.db, id).await.map_err(internal)?;
    Ok(Json(serde_json::json!({ "users": users })))
}

pub async fn fetch_agent_logs(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    db::get_agent_by_id(&state.db, id).await.map_err(internal)?.ok_or_else(not_found)?;

    if !state.is_online(DEFAULT_TENANT, id).await {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": "Agent is offline" })),
        ));
    }

    let (tx, rx) = tokio::sync::oneshot::channel::<Vec<String>>();
    state.log_requests.write().await.insert((DEFAULT_TENANT.to_string(), id), tx);

    use common::messages::{FetchLogs, MSG_FETCH_LOGS};
    use common::protocol::WssMessage;
    let msg = WssMessage::new(MSG_FETCH_LOGS, &FetchLogs {}).map_err(internal)?;
    state.send_to_agent_id(DEFAULT_TENANT, id, msg).await;

    match tokio::time::timeout(std::time::Duration::from_secs(5), rx).await {
        Ok(Ok(lines)) => Ok(Json(serde_json::json!({ "lines": lines }))),
        _ => {
            state.log_requests.write().await.remove(&(DEFAULT_TENANT.to_string(), id));
            Err((
                StatusCode::GATEWAY_TIMEOUT,
                Json(serde_json::json!({ "error": "Agent did not respond in time" })),
            ))
        }
    }
}

pub async fn update_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    db::get_agent_by_id(&state.db, id).await.map_err(internal)?.ok_or_else(not_found)?;

    if !state.is_online(DEFAULT_TENANT, id).await {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": "Agent is offline" })),
        ));
    }

    use common::messages::{UpdateAgent, MSG_UPDATE_AGENT};
    use common::protocol::WssMessage;
    let msg = WssMessage::new(MSG_UPDATE_AGENT, &UpdateAgent {}).map_err(internal)?;
    state.send_to_agent_id(DEFAULT_TENANT, id, msg).await;

    tracing::info!("Remote update triggered for agent {id}");
    Ok(Json(serde_json::json!({ "message": "Update triggered" })))
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
