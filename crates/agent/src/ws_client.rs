use anyhow::{Context, Result};
use common::messages::{
    ConfigPush, ConfigReload, FetchLogs, LockNow, NotifyUser, PairingAccepted, RemainingUpdate,
    ServerMessage, Unpair, MSG_CONFIG_PUSH, MSG_CONFIG_RELOAD, MSG_FETCH_LOGS, MSG_LOCK_NOW,
    MSG_NOTIFY_USER, MSG_PAIRING_ACCEPTED, MSG_REMAINING_UPDATE, MSG_UNPAIR,
};
use common::protocol::WssMessage;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::client::IntoClientRequest, tungstenite::Message};

const BACKOFF_STEPS: &[u64] = &[5, 10, 30, 60, 120, 300];

#[derive(Debug, Clone)]
pub enum ConnectionEvent {
    Connected,
    Disconnected,
}

pub struct WsClient {
    pub outbound_tx: mpsc::Sender<WssMessage>,
    pub inbound_rx: mpsc::Receiver<ServerMessage>,
    pub connection_rx: mpsc::Receiver<ConnectionEvent>,
}

pub fn spawn(server_url: String, auth_token: String) -> WsClient {
    let (outbound_tx, mut outbound_rx) = mpsc::channel::<WssMessage>(64);
    let (inbound_tx, inbound_rx) = mpsc::channel::<ServerMessage>(64);
    let (connection_tx, connection_rx) = mpsc::channel::<ConnectionEvent>(8);

    tokio::spawn(async move {
        let mut backoff_idx = 0usize;

        loop {
            match connect_authenticated(&server_url, &auth_token).await {
                Ok((mut write, mut read)) => {
                    tracing::info!("WebSocket connected to {server_url}");
                    backoff_idx = 0;
                    let _ = connection_tx.send(ConnectionEvent::Connected).await;

                    loop {
                        tokio::select! {
                            Some(msg) = outbound_rx.recv() => {
                                match msg.to_json() {
                                    Ok(json) => {
                                        if write.send(Message::Text(json)).await.is_err() {
                                            tracing::warn!("WebSocket write failed, reconnecting");
                                            break;
                                        }
                                    }
                                    Err(e) => tracing::error!("Failed to serialize message: {e}"),
                                }
                            }
                            msg = read.next() => {
                                match msg {
                                    Some(Ok(Message::Text(text))) => {
                                        match parse_server_message(&text) {
                                            Ok(parsed) => { let _ = inbound_tx.send(parsed).await; }
                                            Err(e) => tracing::warn!("Failed to parse server message: {e}"),
                                        }
                                    }
                                    Some(Ok(Message::Ping(data))) => {
                                        let _ = write.send(Message::Pong(data)).await;
                                    }
                                    Some(Ok(Message::Close(_))) | None => {
                                        tracing::warn!("WebSocket connection closed, reconnecting");
                                        break;
                                    }
                                    Some(Err(e)) => {
                                        tracing::warn!("WebSocket error: {e}, reconnecting");
                                        break;
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }

                    let _ = connection_tx.send(ConnectionEvent::Disconnected).await;
                }
                Err(e) => {
                    tracing::warn!("WebSocket connection failed: {e}");
                }
            }

            let delay = BACKOFF_STEPS[backoff_idx.min(BACKOFF_STEPS.len() - 1)];
            tracing::info!("Reconnecting in {delay}s...");
            tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
            backoff_idx = (backoff_idx + 1).min(BACKOFF_STEPS.len() - 1);
        }
    });

    WsClient { outbound_tx, inbound_rx, connection_rx }
}

async fn connect_authenticated(
    url: &str,
    token: &str,
) -> Result<(
    impl SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
    impl StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
)> {
    let mut request = url.into_client_request()?;
    request.headers_mut().insert(
        "Authorization",
        format!("Bearer {token}").parse()?,
    );

    let (stream, _) = connect_async(request)
        .await
        .with_context(|| format!("Failed to connect to {url}"))?;

    Ok(stream.split())
}

fn parse_server_message(text: &str) -> Result<ServerMessage> {
    let envelope = WssMessage::from_json(text)?;
    let msg = match envelope.msg_type.as_str() {
        MSG_CONFIG_PUSH => ServerMessage::ConfigPush(envelope.parse_payload::<ConfigPush>()?),
        MSG_REMAINING_UPDATE => {
            ServerMessage::RemainingUpdate(envelope.parse_payload::<RemainingUpdate>()?)
        }
        MSG_PAIRING_ACCEPTED => {
            ServerMessage::PairingAccepted(envelope.parse_payload::<PairingAccepted>()?)
        }
        MSG_LOCK_NOW => ServerMessage::LockNow(envelope.parse_payload::<LockNow>()?),
        MSG_NOTIFY_USER => ServerMessage::NotifyUser(envelope.parse_payload::<NotifyUser>()?),
        MSG_CONFIG_RELOAD => {
            let _ = envelope.parse_payload::<ConfigReload>();
            ServerMessage::ConfigReload
        }
        MSG_UNPAIR => {
            let _ = envelope.parse_payload::<Unpair>();
            ServerMessage::Unpair
        }
        MSG_FETCH_LOGS => {
            let _ = envelope.parse_payload::<FetchLogs>();
            ServerMessage::FetchLogs
        }
        other => ServerMessage::Unknown(other.to_string()),
    };
    Ok(msg)
}

pub async fn send<T: serde::Serialize>(
    tx: &mpsc::Sender<WssMessage>,
    msg_type: &str,
    payload: &T,
) -> Result<()> {
    let msg = WssMessage::new(msg_type, payload)?;
    tx.send(msg).await.context("WebSocket send channel closed")?;
    Ok(())
}
