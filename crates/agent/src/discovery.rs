use anyhow::Result;
use mdns_sd::{ServiceDaemon, ServiceEvent};
use std::time::Duration;
use tokio::sync::oneshot;

const SERVICE_TYPE: &str = "_parctrl._tcp.local.";
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(30);

/// Discover the management server via mDNS.
/// Returns a WS URL like `ws://192.168.1.100:8765` on success, or None on timeout.
pub async fn discover_server() -> Result<Option<String>> {
    let mdns = ServiceDaemon::new()?;
    let receiver = mdns.browse(SERVICE_TYPE)?;

    let (tx, rx) = oneshot::channel::<String>();

    // Run the blocking mdns-sd receiver in a thread pool thread.
    tokio::task::spawn_blocking(move || {
        while let Ok(event) = receiver.recv() {
            if let ServiceEvent::ServiceResolved(info) = event {
                let port = info.get_port();
                // Prefer a concrete IP over the .local hostname to avoid mDNS resolution round-trips.
                let host = info
                    .get_addresses()
                    .iter()
                    .next()
                    .map(|a| a.to_string())
                    .unwrap_or_else(|| info.get_hostname().trim_end_matches('.').to_string());
                let url = format!("ws://{}:{}", host, port);
                let _ = tx.send(url);
                break;
            }
        }
    });

    let result = match tokio::time::timeout(DISCOVERY_TIMEOUT, rx).await {
        Ok(Ok(url)) => {
            tracing::info!("Discovered server via mDNS: {url}");
            Ok(Some(url))
        }
        _ => {
            tracing::warn!(
                "mDNS discovery timed out after {}s",
                DISCOVERY_TIMEOUT.as_secs()
            );
            Ok(None)
        }
    };

    // Stop the browse so the daemon shuts down cleanly without channel errors.
    let _ = mdns.stop_browse(SERVICE_TYPE);
    result
}

/// Resolve the server URL: use configured URL if present, otherwise run mDNS discovery.
pub async fn resolve_server_url(configured_url: Option<&str>) -> Result<Option<String>> {
    if let Some(url) = configured_url {
        tracing::info!("Using configured server URL: {url}");
        return Ok(Some(url.to_string()));
    }
    tracing::info!("No server URL configured, starting mDNS discovery...");
    discover_server().await
}
