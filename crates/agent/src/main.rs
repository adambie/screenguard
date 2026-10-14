mod config;
mod db;
mod dbus;
mod discovery;
mod dns_proxy;
mod enforcement;
mod heartbeat;
mod i18n;
mod nftables;
mod pairing;
mod password;
mod status_dbus;
mod users;
mod web_filter;
mod ws_client;

use anyhow::{Context, Result, bail};
use db::{AgentMode, Db, ServerConnection};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

/// Read `--flag value` or `--flag=value` out of the raw args (no clap in this crate).
fn arg_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if let Some(v) = a.strip_prefix(flag).and_then(|r| r.strip_prefix('=')) {
            return Some(v);
        }
        if a == flag {
            return it.next().map(String::as_str);
        }
    }
    None
}

/// Case-insensitive key for comparing two cloud-account values. Only used for
/// the equality check — the address is stored and sent to the server with its
/// original case preserved (the cloud account lookup may or may not fold case).
fn account_key(s: &str) -> String {
    s.trim().to_lowercase()
}

/// Whether the agent must drop its current pairing and re-pair, given whether it
/// is already paired, the account stored with that pairing, and the account it
/// was asked to use now.
///
/// Only an *explicit, different* `--cloud-account` (or `agent.toml` value)
/// triggers this. Starting with **no** account while paired is left alone — that
/// keeps `systemctl restart` (unit file carries no flag) from wiping a pairing,
/// and "go back to local" stays the job of `--reset`, same as switching servers.
fn should_rebind(paired: bool, stored: Option<&str>, requested: Option<&str>) -> bool {
    match (paired, requested) {
        (true, Some(req)) => stored.map(account_key) != Some(account_key(req)),
        _ => false,
    }
}

fn wss_to_http(url: &str) -> String {
    let s = url.replacen("wss://", "https://", 1).replacen("ws://", "http://", 1);
    if let Some((scheme, rest)) = s.split_once("//") {
        let host_port = rest.split('/').next().unwrap_or(rest);
        format!("{scheme}//{host_port}")
    } else {
        s
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let _ = libsystemd::daemon::notify(
        false,
        &[libsystemd::daemon::NotifyState::Status("starting".into())],
    );

    // rustls 0.23 with the `ring` provider does not auto-select a process-level
    // CryptoProvider; install it before any wss:// (cloud mode) connection.
    // Harmless when only ws:// is used.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let args: Vec<String> = std::env::args().collect();

    // Handle --lock-cinnamon: lock the Cinnamon screensaver as the current session user and exit.
    // The agent spawns itself under the target uid with the user session bus set in the environment.
    if args.get(1).map(String::as_str) == Some("--lock-cinnamon") {
        std::process::exit(crate::dbus::lock_cinnamon_as_current_user().await);
    }

    // Handle --notify <summary> <body>: send a desktop notification as the current user and exit.
    // The agent spawns itself under the target uid, so at this point we are already that user.
    if args.get(1).map(String::as_str) == Some("--notify") {
        let summary = args.get(2).map(String::as_str).unwrap_or("");
        let body = args.get(3).map(String::as_str).unwrap_or("");
        if let Err(e) = crate::dbus::notify_as_current_user(summary, body).await {
            tracing::warn!("Notification failed: {e}");
        }
        return Ok(());
    }

    // Handle --reset: clear pairing state and exit.
    if args.iter().any(|a| a == "--reset") {
        let db = Db::open(None).context("Failed to open agent database")?;
        db.reset_pairing()?;
        println!("Agent reset. Re-run without --reset to start pairing.");
        return Ok(());
    }

    let cfg = config::load(None)?;
    tracing::info!("Agent starting, config loaded");

    // ── experimental cloud mode ───────────────────────────────────────────────
    // `--cloud-account <email>` (or `cloud_account` in agent.toml) binds this
    // agent to a cloud tenant and makes it connect to the cloud endpoint instead
    // of doing mDNS / local discovery. Absent ⇒ behaviour is byte-for-byte the
    // same as before this feature existed.
    let cloud_account: Option<String> = arg_value(&args, "--cloud-account")
        .map(str::to_string)
        .or_else(|| cfg.cloud_account.clone())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    if let Some(acct) = cloud_account.as_deref().filter(|a| !a.contains('@')) {
        bail!(
            "--cloud-account expects an email address, got {acct:?}. \
             The account identifies your cloud tenant, not the server host \
             (use --server-url for a custom endpoint)."
        );
    }

    // Endpoint used only in cloud mode: --server-url > cloud_url > compiled default.
    let cloud_server_url: Option<String> = arg_value(&args, "--server-url")
        .map(str::to_string)
        .or_else(|| cfg.cloud_url.clone());

    let db = Arc::new(Mutex::new(
        Db::open(None).context("Failed to open agent database")?,
    ));

    let nft_available = nftables::is_available();
    {
        let db = db.lock().await;
        db.save_capability("web_filter", nft_available)?;
        if nft_available {
            tracing::info!("nft available — web filtering capability enabled");
        } else {
            tracing::warn!("nft not found — web filtering disabled. Install nftables to enable it.");
        }
    }

    let mode = {
        let db = db.lock().await;
        db.get_agent_mode()?
    };

    // The account this pairing was made with (None ⇒ a local pairing).
    let stored_account: Option<String> = {
        let db = db.lock().await;
        db.get_server_connection()?.and_then(|c| c.cloud_account)
    };
    // A different explicit `--cloud-account` means the old pairing + all
    // downloaded policy belong to another tenant and must be dropped first.
    // Absence of an account while paired is *not* a re-bind (see `should_rebind`).
    let rebind = should_rebind(
        mode != AgentMode::Unpaired,
        stored_account.as_deref(),
        cloud_account.as_deref(),
    );
    if rebind {
        tracing::warn!(
            "Cloud account changed ({} → {}); clearing this machine's pairing, \
             cached rules and usage — it will be UNMANAGED until the new account \
             approves it.",
            stored_account.as_deref().unwrap_or("<local>"),
            cloud_account.as_deref().unwrap_or("<local>"),
        );
        let db = db.lock().await;
        db.wipe_for_rebind()?;
    }

    // What actually goes on the wire: an explicit account wins, otherwise fall
    // back to the one saved with the pairing so reconnects after a plain restart
    // still tell the cloud server which tenant this agent belongs to.
    let effective_account: Option<String> =
        cloud_account.clone().or_else(|| stored_account.clone());

    if let Some(acct) = &effective_account {
        tracing::warn!("EXPERIMENTAL cloud mode: account {acct}");
    }

    let (server_url, auth_token) = if mode == AgentMode::Unpaired || rebind {
        let server_url = if cloud_account.is_some() {
            let url = cloud_server_url
                .clone()
                .unwrap_or_else(|| config::DEFAULT_CLOUD_URL.to_string());
            tracing::info!("Cloud mode: connecting to {url} (discovery skipped)");
            url
        } else {
            tracing::info!("Agent is unpaired, starting discovery + pairing flow");
            discovery::resolve_server_url(cfg.server_url.as_deref())
                .await?
                .context("Could not discover or connect to a management server")?
        };

        let result = pairing::run_pairing(&server_url, cloud_account.as_deref())
            .await
            .context("Pairing failed")?;

        {
            let db = db.lock().await;
            db.save_server_connection(&ServerConnection {
                server_url: server_url.clone(),
                auth_token: result.auth_token.clone(),
                agent_id: result.agent_id.clone(),
                cloud_account: cloud_account.clone(),
            })?;
            // Mode will be set to Online by heartbeat loop on first ConnectionEvent::Connected.
        }
        tracing::info!("Pairing complete, agent_id={}", result.agent_id);
        (server_url, result.auth_token)
    } else {
        let conn = {
            let db = db.lock().await;
            db.get_server_connection()?.context(
                "Paired but no server_connection record found — run with --reset to re-pair",
            )?
        };
        (conn.server_url, conn.auth_token)
    };

    tracing::info!("Connecting to server at {server_url}");

    let status_handle = {
        let http_url = cfg.webui_url.clone().unwrap_or_else(|| wss_to_http(&server_url));
        match status_dbus::start(http_url).await {
            Ok(h) => {
                tracing::info!("Tray D-Bus interface registered");
                Some(Arc::new(h))
            }
            Err(e) => {
                tracing::warn!("Tray D-Bus interface unavailable: {e}");
                None
            }
        }
    };

    let ws = ws_client::spawn(server_url, auth_token);

    let (session_tx, session_rx) = mpsc::channel(64);
    let dbus_monitor = dbus::DbusMonitor::new(session_tx)
        .await
        .context("Failed to connect to D-Bus system bus")?;
    tokio::spawn(async move {
        if let Err(e) = dbus_monitor.run().await {
            tracing::error!("D-Bus monitor error: {e}");
        }
    });

    let _ = libsystemd::daemon::notify(false, &[libsystemd::daemon::NotifyState::Ready]);
    tracing::info!("Agent ready");

    if let Some(watchdog_dur) = libsystemd::daemon::watchdog_enabled(false) {
        let interval = watchdog_dur / 2;
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            loop {
                ticker.tick().await;
                let _ = libsystemd::daemon::notify(
                    false,
                    &[libsystemd::daemon::NotifyState::Watchdog],
                );
            }
        });
    }

    let loop_handle = heartbeat::HeartbeatLoop::new(
        db.clone(),
        ws.outbound_tx,
        ws.inbound_rx,
        ws.connection_rx,
        session_rx,
        cfg.heartbeat_interval,
        cfg.user_scan_interval,
        cfg.min_uid,
        cfg.cache_ttl_hours,
        status_handle,
        nft_available,
        effective_account,
    );

    // SIGTERM: notify systemd STOPPING=1 then exit.
    tokio::spawn(async {
        if let Ok(mut signal) = tokio::signal::unix::signal(
            tokio::signal::unix::SignalKind::terminate(),
        ) {
            signal.recv().await;
            tracing::info!("SIGTERM received, shutting down");
            let _ = libsystemd::daemon::notify(
                false,
                &[libsystemd::daemon::NotifyState::Stopping],
            );
            std::process::exit(0);
        }
    });

    loop_handle.run().await
}

#[cfg(test)]
mod tests {
    use super::{arg_value, should_rebind};

    fn args(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn arg_value_reads_space_and_equals_forms() {
        assert_eq!(arg_value(&args(&["x", "--cloud-account", "a@b.net"]), "--cloud-account"), Some("a@b.net"));
        assert_eq!(arg_value(&args(&["x", "--cloud-account=a@b.net"]), "--cloud-account"), Some("a@b.net"));
        assert_eq!(arg_value(&args(&["x", "--other"]), "--cloud-account"), None);
        assert_eq!(arg_value(&args(&["x", "--cloud-account"]), "--cloud-account"), None); // no value
    }

    #[test]
    fn rebind_only_on_explicit_different_account() {
        // not paired yet -> never a rebind, whatever is asked
        assert!(!should_rebind(false, None, Some("a@x")));
        // same account, any case / whitespace -> no rebind
        assert!(!should_rebind(true, Some("a@x"), Some("a@x")));
        assert!(!should_rebind(true, Some("a@x"), Some("  A@X ")));
        // paired, no account requested (plain restart / flag dropped) -> left alone
        assert!(!should_rebind(true, Some("a@x"), None));
        assert!(!should_rebind(true, None, None));
        // genuinely different targets -> rebind
        assert!(should_rebind(true, Some("a@x"), Some("b@y")));
        assert!(should_rebind(true, None, Some("a@x")));
    }
}
