mod config;
mod db;
mod dbus;
mod discovery;
mod enforcement;
mod heartbeat;
mod i18n;
mod pairing;
mod status_dbus;
mod users;
mod ws_client;

use anyhow::{Context, Result};
use db::{AgentMode, Db, ServerConnection};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

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

    let args: Vec<String> = std::env::args().collect();

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

    let db = Arc::new(Mutex::new(
        Db::open(None).context("Failed to open agent database")?,
    ));

    let mode = {
        let db = db.lock().await;
        db.get_agent_mode()?
    };

    let (server_url, auth_token) = match mode {
        AgentMode::Unpaired => {
            tracing::info!("Agent is unpaired, starting discovery + pairing flow");
            let server_url = discovery::resolve_server_url(cfg.server_url.as_deref())
                .await?
                .context("Could not discover or connect to a management server")?;

            let result = pairing::run_pairing(&server_url)
                .await
                .context("Pairing failed")?;

            {
                let db = db.lock().await;
                db.save_server_connection(&ServerConnection {
                    server_url: server_url.clone(),
                    auth_token: result.auth_token.clone(),
                    agent_id: result.agent_id.clone(),
                })?;
                // Mode will be set to Online by heartbeat loop on first ConnectionEvent::Connected.
            }
            tracing::info!("Pairing complete, agent_id={}", result.agent_id);
            (server_url, result.auth_token)
        }

        AgentMode::Online | AgentMode::Offline => {
            let conn = {
                let db = db.lock().await;
                db.get_server_connection()?.context(
                    "Paired but no server_connection record found — run with --reset to re-pair",
                )?
            };
            (conn.server_url, conn.auth_token)
        }
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
