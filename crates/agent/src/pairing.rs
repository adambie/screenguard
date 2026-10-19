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

/// Root-only file holding the code while a pairing request is open, so
/// `screenguard-agent --show-pairing-code` (and the installer) can show it
/// without digging through the journal.
pub const PAIRING_CODE_FILE: &str = "/var/lib/screenguard/pairing-code";

/// Writes `code` and its expiry (unix seconds) to `path`, readable by root only.
fn write_code_file(path: &std::path::Path, code: &str, expires_at: u64) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let _ = std::fs::remove_file(path);
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    writeln!(f, "{code}\n{expires_at}")
}

/// The code from `path` if it is still valid at `now` (unix seconds).
pub fn read_code_file(path: &std::path::Path, now: u64) -> std::io::Result<Option<String>> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let mut lines = text.lines();
    let code = lines.next().unwrap_or("").trim().to_string();
    let expires_at: u64 = lines.next().and_then(|l| l.trim().parse().ok()).unwrap_or(0);
    Ok((!code.is_empty() && now < expires_at).then_some(code))
}

pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Removes the code file when pairing ends, however it ends.
struct CodeFileGuard(&'static str);

impl Drop for CodeFileGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(self.0);
    }
}

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
///
/// `cloud_account` is `Some(email)` only in experimental cloud mode; it is put
/// on the `pairing_request` so the cloud server can route the agent to the right
/// tenant. `None` ⇒ the field is omitted from the wire entirely (self-hosted).
pub async fn run_pairing(server_url: &str, cloud_account: Option<&str>) -> Result<PairingResult> {
    let hostname = gethostname();
    let mid = machine_id().unwrap_or_else(|_| uuid::Uuid::new_v4().to_string());
    let code = generate_pairing_code();

    println!("\n=== PAIRING CODE: {code} ===");
    println!("Enter this code in the ScreenGuard web UI to approve this machine.");
    println!("(Show it again any time with: sudo screenguard-agent --show-pairing-code)\n");
    tracing::info!("Pairing code: {code}");

    let (ws_stream, _) = connect_async(server_url)
        .await
        .with_context(|| format!("Failed to connect to server for pairing: {server_url}"))?;

    let (mut write, mut read) = ws_stream.split();

    let request = PairingRequest {
        machine_id: mid,
        hostname,
        pairing_code: code,
        cloud_account: cloud_account.map(str::to_string),
    };
    let msg = WssMessage::new(MSG_PAIRING_REQUEST, &request)?;
    write.send(Message::Text(msg.to_json()?)).await?;

    // Only now is the code valid on the server.
    let _code_file = CodeFileGuard(PAIRING_CODE_FILE);
    if let Err(e) = write_code_file(
        std::path::Path::new(PAIRING_CODE_FILE),
        &request.pairing_code,
        unix_now() + PAIRING_TIMEOUT_SECS,
    ) {
        tracing::warn!("Could not write {PAIRING_CODE_FILE}: {e}");
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_file_round_trip_and_expiry() {
        let dir = std::env::temp_dir().join(format!("sg-pair-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("pairing-code");

        assert_eq!(read_code_file(&path, 100).unwrap(), None);
        write_code_file(&path, "ABCDEF", 200).unwrap();
        assert_eq!(read_code_file(&path, 100).unwrap().as_deref(), Some("ABCDEF"));
        assert_eq!(read_code_file(&path, 200).unwrap(), None, "expired");

        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);

        // Rewriting replaces the old code.
        write_code_file(&path, "GHJKLM", 300).unwrap();
        assert_eq!(read_code_file(&path, 100).unwrap().as_deref(), Some("GHJKLM"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
