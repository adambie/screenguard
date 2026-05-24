use anyhow::Result;
use axum::{extract::{State, WebSocketUpgrade}, response::Response, routing::get, Router};
use std::sync::Arc;
use tokio::signal::unix::{signal, SignalKind};

mod api;
mod config;
mod db;
mod remaining;
mod state;
mod ws;

use state::AppState;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cfg = config::load(None)?;
    tracing::info!("Server starting on {}:{}", cfg.listen_addr, cfg.listen_port);

    // Open database.
    let pool = db::open(&cfg.db_path)?;

    // Resolve or generate JWT secret.
    let jwt_secret = cfg.jwt_secret.clone().unwrap_or_else(|| {
        use rand::Rng;
        let secret: String = rand::thread_rng()
            .sample_iter(&rand::distributions::Alphanumeric)
            .take(64)
            .map(char::from)
            .collect();
        tracing::warn!("No jwt_secret configured — generated ephemeral secret (tokens won't survive restart)");
        secret
    });

    let state = AppState::new(pool, jwt_secret, cfg.jwt_expiry_hours);

    // Warn if no admin account exists.
    if db::admin_count(&state.db)? == 0 {
        tracing::warn!("No admin account — call POST /api/v1/auth/setup to create one");
    }

    // Build router: WebSocket at /ws, REST API at /api/v1/**.
    let ws_router = Router::new()
        .route("/ws", get(ws_handler))
        .with_state(state.clone());
    let app = ws_router.merge(api::router(state.clone()));

    // mDNS advertisement.
    let _mdns = if cfg.enable_mdns {
        Some(register_mdns(cfg.listen_port))
    } else {
        None
    };

    // Systemd notify ready.
    let _ = libsystemd::daemon::notify(false, &[libsystemd::daemon::NotifyState::Ready]);

    // Watchdog.
    if let Some(dur) = libsystemd::daemon::watchdog_enabled(false) {
        let interval = dur / 2;
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            loop {
                ticker.tick().await;
                let _ = libsystemd::daemon::notify(false, &[libsystemd::daemon::NotifyState::Watchdog]);
            }
        });
    }

    // SIGTERM handler.
    let mut sigterm = signal(SignalKind::terminate())?;
    let addr = format!("{}:{}", cfg.listen_addr, cfg.listen_port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("Listening on {addr}");

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            sigterm.recv().await;
            let _ = libsystemd::daemon::notify(
                false,
                &[libsystemd::daemon::NotifyState::Stopping],
            );
            tracing::info!("Shutting down");
        })
        .await?;

    Ok(())
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> Response {
    ws.on_upgrade(move |socket| ws::handle_ws(socket, state))
}

fn register_mdns(port: u16) -> mdns_sd::ServiceDaemon {
    let mdns = mdns_sd::ServiceDaemon::new().expect("mDNS daemon failed");
    let ip = local_ip();
    let hostname = std::fs::read_to_string("/etc/hostname")
        .unwrap_or_else(|_| "parental-server".to_string())
        .trim()
        .to_string();
    let instance = format!("pc-server.{hostname}");
    let service = mdns_sd::ServiceInfo::new(
        "_parctrl._tcp.local.",
        &instance,
        &format!("{hostname}.local."),
        ip.to_string().as_str(),
        port,
        None,
    )
    .expect("mDNS ServiceInfo failed");
    mdns.register(service).expect("mDNS register failed");
    tracing::info!("mDNS: advertising _parctrl._tcp.local. on {ip}:{port}");
    mdns
}

fn local_ip() -> std::net::IpAddr {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").expect("bind failed");
    socket.connect("8.8.8.8:80").expect("connect failed");
    socket.local_addr().expect("local_addr failed").ip()
}
