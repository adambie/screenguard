use anyhow::Result;
use axum::extract::ws::{Message, WebSocket};
use common::messages::{
    AgentHello, Heartbeat, PairingAccepted, PairingRequest, RemainingUpdate, Unpair,
    UsageSync, UserListUpdate, MSG_AGENT_HELLO, MSG_CONFIG_PUSH, MSG_HEARTBEAT,
    MSG_PAIRING_ACCEPTED, MSG_PAIRING_REQUEST, MSG_REMAINING_UPDATE, MSG_UNPAIR,
    MSG_USAGE_SYNC, MSG_USER_LIST_UPDATE,
};
use common::protocol::WssMessage;
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

use crate::db;
use crate::remaining;
use crate::state::{AgentHandle, AppState, PairingDecision, PairingHandle};

pub async fn handle_ws(socket: WebSocket, state: Arc<AppState>) {
    if let Err(e) = handle_ws_inner(socket, state).await {
        tracing::warn!("WebSocket handler error: {e}");
    }
}

async fn handle_ws_inner(socket: WebSocket, state: Arc<AppState>) -> Result<()> {
    let (mut sink, mut stream) = socket.split();
    let (out_tx, mut out_rx) = mpsc::channel::<WssMessage>(64);

    // Spawn outbound writer task.
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
            // After pairing_accepted is sent the agent closes its pairing
            // connection and reconnects with the auth token. Close here.
            handle_pairing_request(req, &state, out_tx.clone()).await?;
            return Ok(());
        }
        MSG_AGENT_HELLO => {
            let hello: AgentHello = envelope.parse_payload()?;
            // If agent is pending_delete, send unpair and wait for it to disconnect.
            if let Ok(Some(agent)) = db::get_agent_by_machine_id(&state.db, &hello.machine_id) {
                if agent.status == "pending_delete" {
                    tracing::info!("Agent {} is pending_delete — sending unpair", hello.machine_id);
                    let msg = WssMessage::new(MSG_UNPAIR, &Unpair {})?;
                    let _ = out_tx.send(msg).await;
                    // Drain until the agent closes the connection.
                    while let Some(Ok(msg)) = stream.next().await {
                        if matches!(msg, Message::Close(_)) { break; }
                    }
                    // Agent has acknowledged — delete the record so re-pairing starts fresh.
                    let _ = db::delete_agent(&state.db, agent.id);
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
    state.add_online(machine_id.clone(), AgentHandle {
        agent_id: agent_db_id,
        outbound_tx: out_tx,
    }).await;

    let result = message_loop(&mut stream, &state, agent_db_id, &machine_id).await;

    state.remove_online(&machine_id).await;

    // If the agent was pending_delete, it disconnected after receiving unpair — clean up now.
    if let Ok(Some(agent)) = db::get_agent_by_id(&state.db, agent_db_id) {
        if agent.status == "pending_delete" {
            let _ = db::delete_agent(&state.db, agent_db_id);
            tracing::info!("Agent {} deleted after unpair (was online)", machine_id);
            return result;
        }
    }

    let _ = db::update_agent_last_seen(&state.db, agent_db_id);
    tracing::info!("Agent {machine_id} disconnected");

    result
}

async fn handle_pairing_request(
    req: PairingRequest,
    state: &Arc<AppState>,
    out_tx: mpsc::Sender<WssMessage>,
) -> Result<()> {
    tracing::info!(
        "Pairing request from '{}' (machine_id={}, code={})",
        req.hostname, req.machine_id, req.pairing_code
    );

    // Upsert agent record as pending.
    let agent = db::upsert_agent_pending(
        &state.db,
        &req.machine_id,
        &req.hostname,
        "UTC",
        "",
    )?;

    // Register a pairing handle so REST /accept can deliver the decision.
    let (tx, rx) = oneshot::channel::<PairingDecision>();
    {
        let mut pending = state.pending.write().await;
        pending.insert(req.machine_id.clone(), PairingHandle {
            pairing_code: req.pairing_code.clone(),
            tx,
        });
    }

    tracing::info!(
        "Agent '{}' pending — pairing code: {}. Run: POST /api/v1/agents/{}/accept",
        req.hostname, req.pairing_code, agent.id
    );

    // Wait for admin to accept (no timeout — agent will reconnect if needed).
    let decision = rx.await?;

    // Remove from pending map.
    state.pending.write().await.remove(&req.machine_id);

    let accepted = PairingAccepted {
        agent_id: decision.agent_db_id.to_string(),
        auth_token: decision.auth_token,
    };
    let msg = WssMessage::new(MSG_PAIRING_ACCEPTED, &accepted)?;
    out_tx.send(msg).await?;

    tracing::info!("Pairing accepted for agent {} — agent will reconnect with auth token", decision.agent_db_id);
    Ok(())
}

async fn handle_agent_hello(
    hello: AgentHello,
    state: &Arc<AppState>,
    out_tx: mpsc::Sender<WssMessage>,
) -> Result<(String, Uuid)> {
    // Validate auth: look up agent by machine_id, check token hash.
    let Some(agent) = db::get_agent_by_machine_id(&state.db, &hello.machine_id)? else {
        tracing::warn!("Unknown agent: {}", hello.machine_id);
        return Err(anyhow::anyhow!("Unknown agent"));
    };

    if agent.status != "paired" {
        tracing::warn!("Agent {} not paired (status={})", hello.machine_id, agent.status);
        return Err(anyhow::anyhow!("Agent not paired"));
    }

    db::update_agent_hello(&state.db, agent.id, &hello.hostname, &hello.timezone, &hello.agent_version)?;
    tracing::info!("Agent '{}' connected (v{}, config_v{})", hello.hostname, hello.agent_version, hello.last_config_version);

    // Check if config is stale → push updated config.
    let agent_users = db::list_agent_users(&state.db, agent.id)?;
    let profile_ids: Vec<Uuid> = agent_users.iter()
        .filter_map(|u| u.profile_id)
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    let server_version: i64 = profile_ids.iter()
        .map(|pid| db::get_config_version(&state.db, *pid).unwrap_or(1))
        .max()
        .unwrap_or(1);

    if hello.last_config_version < server_version {
        let push = remaining::build_config_push(&state.db, agent.id, server_version)?;
        let msg = WssMessage::new(MSG_CONFIG_PUSH, &push)?;
        out_tx.send(msg).await?;
        tracing::info!("Sent config_push v{server_version} to agent {}", agent.id);
    }

    Ok((hello.machine_id, agent.id))
}

/// If no message arrives within this window the connection is considered stale
/// (e.g. PC went offline abruptly without a clean TCP close) and is dropped so
/// the agent shows as offline in the UI.  The default heartbeat interval is 10 s,
/// so 90 s (9× that) gives ample headroom for any reasonable configuration.
const WS_IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);

async fn message_loop(
    stream: &mut (impl StreamExt<Item = Result<Message, axum::Error>> + Unpin),
    state: &Arc<AppState>,
    agent_id: Uuid,
    machine_id: &str,
) -> Result<()> {
    let online_map = state.online.read().await;
    let out_tx = online_map
        .get(machine_id)
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
            db::update_agent_hello(&state.db, agent_id, &hello.hostname, &hello.timezone, &hello.agent_version)?;
        }

        MSG_USER_LIST_UPDATE => {
            let update: UserListUpdate = envelope.parse_payload()?;
            db::upsert_agent_users(&state.db, agent_id, &update.users)?;
            if !update.removed_uids.is_empty() {
                db::mark_agent_users_deleted(&state.db, agent_id, &update.removed_uids)?;
            }
            tracing::info!("Agent {agent_id}: user list updated ({} users)", update.users.len());
        }

        MSG_HEARTBEAT => {
            let hb: Heartbeat = envelope.parse_payload()?;
            let agent = db::get_agent_by_id(&state.db, agent_id)?
                .ok_or_else(|| anyhow::anyhow!("Agent not found"))?;

            let hb_input: Vec<(u32, u32)> = hb.users.iter()
                .map(|u| (u.local_uid, u.active_seconds_since_last))
                .collect();

            let entries = remaining::calculate_remaining_for_agent(
                &state.db,
                agent_id,
                &agent.timezone,
                &hb_input,
            )?;

            let reply = WssMessage::new(MSG_REMAINING_UPDATE, &RemainingUpdate { users: entries })?;
            out_tx.send(reply).await?;
        }

        MSG_USAGE_SYNC => {
            let sync: UsageSync = envelope.parse_payload()?;
            tracing::info!("Agent {agent_id}: usage sync ({} records)", sync.usage.len());
            for entry in &sync.usage {
                let date_str = entry.date.to_string();
                if let Some(au) = db::get_agent_user(&state.db, agent_id, entry.local_uid)? {
                    db::add_usage_seconds(&state.db, au.id, &date_str, entry.used_seconds as i64)?;
                }
            }
        }

        other => {
            tracing::debug!("Agent {agent_id}: unknown message type '{other}'");
        }
    }

    Ok(())
}
