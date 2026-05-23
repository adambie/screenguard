use anyhow::{Context, Result, bail};
use common::messages::{
    MSG_PAIRING_ACCEPTED, MSG_PAIRING_REQUEST, PairingAccepted, PairingRequest,
};
use common::protocol::WssMessage;
use futures_util::{SinkExt, StreamExt};
use rand::Rng;
use tokio_tungstenite::{connect_async, tungstenite::Message};

const PAIRING_CODE_LEN: usize = 6;
const PAIRING_TIMEOUT_SECS: u64 = 300;

fn generate_pairing_code() -> String {
    const CHARSET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789"; // no 0/O/1/I ambiguity
    let mut rng = rand::thread_rng();
    (0..PAIRING_CODE_LEN)
        .map(|_| CHARSET[rng.gen_range(0..CHARSET.len())] as char)
        .collect()
}

fn machine_id() -> Result<String> {
    let id = std::fs::read_to_string("/etc/machine-id")
        .or_else(|_| std::fs::read_to_string("/var/lib/dbus/machine-id"))?;
    Ok(id.trim().to_string())
}

pub struct PairingResult {
    pub agent_id: String,
    pub auth_token: String,
}

/// Run the pairing handshake against the server at `server_url`.
/// Blocks until `pairing_accepted` is received or the timeout expires.
pub async fn run_pairing(server_url: &str) -> Result<PairingResult> {
    let hostname = gethostname();
    let mid = machine_id().unwrap_or_else(|_| uuid::Uuid::new_v4().to_string());
    let code = generate_pairing_code();

    println!("\n=== PAIRING CODE: {code} ===");
    println!("Show this code to the admin in the UI to approve pairing.\n");
    tracing::info!("Pairing code: {code}");

    let (ws_stream, _) = connect_async(server_url)
        .await
        .with_context(|| format!("Failed to connect to server for pairing: {server_url}"))?;

    let (mut write, mut read) = ws_stream.split();

    let request = PairingRequest {
        machine_id: mid,
        hostname,
        pairing_code: code,
    };
    let msg = WssMessage::new(MSG_PAIRING_REQUEST, &request)?;
    write.send(Message::Text(msg.to_json()?)).await?;

    let deadline = tokio::time::Instant::now()
        + std::time::Duration::from_secs(PAIRING_TIMEOUT_SECS);

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            bail!("Pairing timed out after {PAIRING_TIMEOUT_SECS}s — admin did not approve");
        }

        match tokio::time::timeout(remaining, read.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => {
                let envelope = WssMessage::from_json(&text)?;
                if envelope.msg_type == MSG_PAIRING_ACCEPTED {
                    let accepted: PairingAccepted = envelope.parse_payload()?;
                    tracing::info!("Pairing accepted, agent_id={}", accepted.agent_id);
                    return Ok(PairingResult {
                        agent_id: accepted.agent_id,
                        auth_token: accepted.auth_token,
                    });
                }
            }
            Ok(Some(Err(e))) => bail!("WebSocket error during pairing: {e}"),
            Ok(None) => bail!("Server closed connection during pairing"),
            Err(_) => bail!("Pairing timed out"),
            _ => {}
        }
    }
}

fn gethostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "unknown".to_string())
}
