use anyhow::Result;
use axum::extract::ws::{Message, WebSocket};
use common::messages::{
    AgentHello, AppRequest, AppUsageSync, Heartbeat, LogResponse, PairingAccepted, PairingRequest,
    RemainingUpdate, Unpair, UsageSync, UserListUpdate, MSG_AGENT_HELLO, MSG_APP_REQUEST, MSG_APP_USAGE_SYNC,
    MSG_CONFIG_PUSH, MSG_HEARTBEAT,
    MSG_LOG_RESPONSE, MSG_PAIRING_ACCEPTED, MSG_PAIRING_REQUEST, MSG_REMAINING_UPDATE, MSG_UNPAIR,
    MSG_USAGE_SYNC, MSG_USER_LIST_UPDATE,
};
use common::protocol::WssMessage;
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

use crate::db;
use crate::remaining;
use crate::state::{AgentHandle, AppState, DEFAULT_TENANT, PairingDecision, PairingHandle};

pub async fn handle_ws(socket: WebSocket, state: Arc<AppState>, bearer: Option<String>) {
    if let Err(e) = handle_ws_inner(socket, state, bearer).await {
        tracing::warn!("WebSocket handler error: {e}");
    }
}

async fn handle_ws_inner(socket: WebSocket, state: Arc<AppState>, bearer: Option<String>) -> Result<()> {
    let (mut sink, mut stream) = socket.split();
    let (out_tx, mut out_rx) = mpsc::channel::<WssMessage>(64);

    tokio::spawn(async move {
        while let Some(msg) = out_rx.recv().await {
            if let Ok(json) = msg.to_json() {
                if sink.send(Message::Text(json.into())).await.is_err() {
                    break;
                }
            }
        }
    });

    // ── Phase 1: identify the agent ──────────────────────────────────────────
    let first_text = loop {
        match stream.next().await {
            Some(Ok(Message::Text(t))) => break t,
            Some(Ok(Message::Ping(d))) => {
                let _ = out_tx.send(WssMessage::new("pong", &serde_json::Value::Null)?).await;
                let _ = d;
            }
            Some(Ok(_)) => continue,
            _ => return Ok(()),
        }
    };

    let envelope = WssMessage::from_json(&first_text)?;
    let (machine_id, agent_db_id) = match envelope.msg_type.as_str() {
        MSG_PAIRING_REQUEST => {
            let req: PairingRequest = envelope.parse_payload()?;
            handle_pairing_request(req, &state, out_tx.clone(), &mut stream).await?;
            return Ok(());
        }
        MSG_AGENT_HELLO => {
            let hello: AgentHello = envelope.parse_payload()?;
            if let Ok(Some(agent)) = db::get_agent_by_machine_id(&state.db, &hello.machine_id).await {
                // machine-id is world-readable, so it identifies nothing on its
                // own: the agent must present the token issued at pairing.
                let token_hash = bearer
                    .as_deref()
                    .filter(|t| !t.is_empty())
                    .map(crate::api::agents::hash_token);
                match db::verify_agent_token(&state.db, agent.id, token_hash.as_deref()).await? {
                    db::TokenCheck::Valid => {}
                    db::TokenCheck::Adopted => tracing::warn!(
                        "Agent {} paired before token verification — adopted its token",
                        hello.machine_id
                    ),
                    db::TokenCheck::UnverifiedNoToken => tracing::warn!(
                        "Agent {} sent no auth token; allowed because its pairing predates \
                         token verification — update the agent",
                        hello.machine_id
                    ),
                    db::TokenCheck::Invalid => {
                        tracing::warn!("Agent {} rejected: invalid auth token", hello.machine_id);
                        return Ok(());
                    }
                }
                if agent.status == "pending_delete" {
                    tracing::info!("Agent {} is pending_delete — sending unpair", hello.machine_id);
                    let msg = WssMessage::new(MSG_UNPAIR, &Unpair {})?;
                    let _ = out_tx.send(msg).await;
                    // Drain until the agent closes the connection.
                    while let Some(Ok(msg)) = stream.next().await {
                        if matches!(msg, Message::Close(_)) { break; }
                    }
                    // Agent has acknowledged — delete the record so re-pairing starts fresh.
                    let _ = db::delete_agent(&state.db, agent.id).await;
                    tracing::info!("Agent {} deleted after unpair", hello.machine_id);
                    return Ok(());
                }
            }
            handle_agent_hello(hello, &state, out_tx.clone()).await?
        }
        other => {
            tracing::warn!("Unexpected first WS message: {other}");
            return Ok(());
        }
    };

    // ── Phase 2: normal message loop ─────────────────────────────────────────
    state.add_online(DEFAULT_TENANT, machine_id.clone(), AgentHandle {
        agent_id: agent_db_id,
        outbound_tx: out_tx,
    }).await;

    let result = message_loop(&mut stream, &state, agent_db_id, &machine_id).await;

    state.remove_online(DEFAULT_TENANT, &machine_id).await;

    // If the agent was pending_delete, it disconnected after receiving unpair — clean up now.
    if let Ok(Some(agent)) = db::get_agent_by_id(&state.db, agent_db_id).await {
        if agent.status == "pending_delete" {
            let _ = db::delete_agent(&state.db, agent_db_id).await;
            tracing::info!("Agent {} deleted after unpair (was online)", machine_id);
            return result;
        }
    }

    let _ = db::update_agent_last_seen(&state.db, agent_db_id).await;
    tracing::info!("Agent {machine_id} disconnected");

    result
}

async fn handle_pairing_request(
    req: PairingRequest,
    state: &Arc<AppState>,
    out_tx: mpsc::Sender<WssMessage>,
    stream: &mut (impl StreamExt<Item = Result<Message, axum::Error>> + Unpin),
) -> Result<()> {
    // The pairing code is deliberately never logged or returned by the API: the
    // admin must read it from the agent's own console to prove they control it.
    tracing::info!("Pairing request from '{}' (machine_id={})", req.hostname, req.machine_id);

    let agent = db::upsert_agent_pending(
        &state.db,
        &req.machine_id,
        &req.hostname,
        "UTC",
        "",
    ).await?;

    let key = (DEFAULT_TENANT.to_string(), req.machine_id.clone());
    let conn_id = Uuid::new_v4();
    let (tx, rx) = oneshot::channel::<PairingDecision>();
    state.pending.write().await.insert(key.clone(), PairingHandle {
        pairing_code: req.pairing_code.clone(),
        tx,
        conn_id,
        failed_attempts: 0,
    });

    tracing::info!("Agent '{}' ({}) waiting for approval with its pairing code", req.hostname, agent.id);

    // Wait for the admin's decision, but stop as soon as the agent goes away
    // (its own 5 min timeout, restart, ...): a request can only be accepted
    // while the agent is actually there to receive the token.
    let decision = tokio::select! {
        d = rx => d.ok(),
        _ = async { while let Some(Ok(_)) = stream.next().await {} } => None,
    };
    let Some(decision) = decision else {
        let mut pending = state.pending.write().await;
        if pending.get(&key).is_some_and(|h| h.conn_id == conn_id) {
            pending.remove(&key);
        }
        tracing::info!("Pairing request from '{}' ended without approval", req.hostname);
        return Ok(());
    };

    let accepted = PairingAccepted {
        agent_id: decision.agent_db_id.to_string(),
        auth_token: decision.auth_token,
    };
    let msg = WssMessage::new(MSG_PAIRING_ACCEPTED, &accepted)?;
    out_tx.send(msg).await?;

    // Agent closes this connection and reconnects with the auth token.
    tracing::info!("Pairing accepted for agent {} — agent will reconnect with auth token", decision.agent_db_id);
    Ok(())
}

async fn handle_agent_hello(
    hello: AgentHello,
    state: &Arc<AppState>,
    out_tx: mpsc::Sender<WssMessage>,
) -> Result<(String, Uuid)> {
    let Some(agent) = db::get_agent_by_machine_id(&state.db, &hello.machine_id).await? else {
        tracing::warn!("Unknown agent: {}", hello.machine_id);
        return Err(anyhow::anyhow!("Unknown agent"));
    };

    if agent.status != "paired" {
        tracing::warn!("Agent {} not paired (status={})", hello.machine_id, agent.status);
        return Err(anyhow::anyhow!("Agent not paired"));
    }

    db::update_agent_hello(&state.db, agent.id, &hello.hostname, &hello.timezone, &hello.agent_version).await?;
    let web_filter_available = hello.capabilities.iter().any(|c| c == "web_filter");
    db::update_agent_web_filter(&state.db, agent.id, web_filter_available).await?;
    let app_usage_available = hello.capabilities.iter().any(|c| c == "app_usage");
    db::update_agent_app_usage(&state.db, agent.id, app_usage_available).await?;
    tracing::info!("Agent '{}' connected (v{}, config_v{}, capabilities={:?})", hello.hostname, hello.agent_version, hello.last_config_version, hello.capabilities);

    let agent_users = db::list_agent_users(&state.db, agent.id).await?;
    let profile_ids: Vec<Uuid> = agent_users.iter()
        .filter_map(|u| u.profile_id)
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    let mut server_version: i64 = 1;
    for pid in &profile_ids {
        let v = db::get_config_version(&state.db, *pid).await.unwrap_or(1);
        if v > server_version { server_version = v; }
    }

    if hello.last_config_version < server_version {
        let push = remaining::build_config_push(&state.db, agent.id, server_version).await?;
        let msg = WssMessage::new(MSG_CONFIG_PUSH, &push)?;
        out_tx.send(msg).await?;
        tracing::info!("Sent config_push v{server_version} to agent {}", agent.id);
    }

    Ok((hello.machine_id, agent.id))
}

const WS_IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);

async fn message_loop(
    stream: &mut (impl StreamExt<Item = Result<Message, axum::Error>> + Unpin),
    state: &Arc<AppState>,
    agent_id: Uuid,
    machine_id: &str,
) -> Result<()> {
    let online_map = state.online.read().await;
    let out_tx = online_map
        .get(&(DEFAULT_TENANT.to_string(), machine_id.to_string()))
        .map(|h| h.outbound_tx.clone())
        .ok_or_else(|| anyhow::anyhow!("Agent handle missing"))?;
    drop(online_map);

    loop {
        match tokio::time::timeout(WS_IDLE_TIMEOUT, stream.next()).await {
            Err(_) => {
                tracing::warn!("Agent {machine_id}: no message for {WS_IDLE_TIMEOUT:?}, closing stale connection");
                break;
            }
            Ok(None) => break,
            Ok(Some(msg)) => match msg {
                Ok(Message::Text(text)) => {
                    if let Err(e) = handle_agent_message(&text, state, agent_id, &out_tx).await {
                        tracing::warn!("Error handling message from {machine_id}: {e}");
                    }
                }
                Ok(Message::Ping(d)) => {
                    let _ = out_tx.send(WssMessage::new("pong", &serde_json::Value::Null)?).await;
                    let _ = d;
                }
                Ok(Message::Close(_)) | Err(_) => break,
                _ => {}
            },
        }
    }
    Ok(())
}

async fn handle_agent_message(
    text: &str,
    state: &Arc<AppState>,
    agent_id: Uuid,
    out_tx: &mpsc::Sender<WssMessage>,
) -> Result<()> {
    let envelope = WssMessage::from_json(text)?;

    match envelope.msg_type.as_str() {
        MSG_AGENT_HELLO => {
            let hello: AgentHello = envelope.parse_payload()?;
            db::update_agent_hello(&state.db, agent_id, &hello.hostname, &hello.timezone, &hello.agent_version).await?;
        }

        MSG_USER_LIST_UPDATE => {
            let update: UserListUpdate = envelope.parse_payload()?;
            db::upsert_agent_users(&state.db, agent_id, &update.users).await?;
            if !update.removed_uids.is_empty() {
                db::mark_agent_users_deleted(&state.db, agent_id, &update.removed_uids).await?;
            }
            tracing::info!("Agent {agent_id}: user list updated ({} users)", update.users.len());
        }

        MSG_HEARTBEAT => {
            let hb: Heartbeat = envelope.parse_payload()?;
            let agent = db::get_agent_by_id(&state.db, agent_id).await?
                .ok_or_else(|| anyhow::anyhow!("Agent not found"))?;

            let hb_input: Vec<(u32, u32)> = hb.users.iter()
                .map(|u| (u.local_uid, u.active_seconds_since_last))
                .collect();

            let admin_tz = db::get_admin_timezone(&state.db).await.unwrap_or_else(|_| "UTC".to_string());
            let entries = remaining::calculate_remaining_for_agent(
                &state.db,
                agent_id,
                &agent.timezone,
                &admin_tz,
                &hb_input,
            ).await?;

            let reply = WssMessage::new(MSG_REMAINING_UPDATE, &RemainingUpdate { users: entries })?;
            out_tx.send(reply).await?;
        }

        MSG_USAGE_SYNC => {
            let sync: UsageSync = envelope.parse_payload()?;
            tracing::info!("Agent {agent_id}: usage sync ({} records)", sync.usage.len());
            for entry in &sync.usage {
                let date_str = entry.date.to_string();
                if let Some(au) = db::get_agent_user(&state.db, agent_id, entry.local_uid).await? {
                    // entry.used_seconds is the agent's whole cumulative total for the
                    // day (a snapshot), not a delta — reconcile, don't add (#13).
                    db::reconcile_usage_seconds(&state.db, au.id, &date_str, entry.used_seconds as i64).await?;
                }
            }
        }

        MSG_APP_USAGE_SYNC => {
            let sync: AppUsageSync = envelope.parse_payload()?;
            // the server's local date, like the screen-time usage API
            let today = chrono::Local::now().date_naive();
            let r = crate::app_usage::ingest(&state.db, agent_id, &sync, today).await?;
            if r.rejected > 0 {
                tracing::warn!("Agent {agent_id}: app usage sync rejected {} of {} entries", r.rejected, sync.entries.len());
            } else {
                tracing::debug!("Agent {agent_id}: app usage sync ({} entries)", r.stored);
            }
        }

        MSG_APP_REQUEST => {
            let req: AppRequest = envelope.parse_payload()?;
            // a new app: into the child's Blocked group, with a notice on the dashboard
            if let Some(profile) = crate::app_usage::app_request(&state.db, agent_id, &req).await? {
                tracing::info!("Agent {agent_id}: new app '{}' ({}) blocked", req.display_name, req.app_id);
                crate::api::profiles::bump_and_propagate(state, profile).await?;
            }
        }

        MSG_LOG_RESPONSE => {
            let resp: LogResponse = envelope.parse_payload()?;
            if let Some(tx) = state.log_requests.write().await.remove(&(DEFAULT_TENANT.to_string(), agent_id)) {
                let _ = tx.send(resp.lines);
            }
        }

        other => {
            tracing::debug!("Agent {agent_id}: unknown message type '{other}'");
        }
    }

    Ok(())
}
