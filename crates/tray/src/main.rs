use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;
use zbus::{interface, object_server::SignalEmitter, proxy, Connection};

// ── status file (written by agent) ───────────────────────────────────────────

#[derive(Debug, Deserialize, Clone, PartialEq)]
struct StatusFile {
    remaining_seconds: i64,
    enforce: String,
    written_at: u64,
}

fn status_file_path() -> PathBuf {
    let uid = std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("Uid:"))?
                .split_whitespace()
                .nth(1)?
                .parse::<u32>()
                .ok()
        })
        .unwrap_or(1000);
    PathBuf::from(format!("/var/lib/screenguard/tray/{uid}/status.json"))
}

// ── tray state (derived from status file) ────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
struct TrayState {
    status: String,
    icon_name: String,
    title: String,
}

impl TrayState {
    fn passive() -> Self {
        Self {
            status: "Passive".into(),
            icon_name: "appointment-soon".into(),
            title: "ScreenGuard".into(),
        }
    }

    fn from_file(sf: &StatusFile) -> Self {
        let now_ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // Hide the icon if the file hasn't been refreshed in 120 s — the agent
        // is likely down or this user is no longer being tracked.
        if now_ts.saturating_sub(sf.written_at) > 120 {
            return Self::passive();
        }

        let elapsed = now_ts.saturating_sub(sf.written_at) as i64;
        let effective = (sf.remaining_seconds - elapsed).max(0);

        let (icon_name, status) = match sf.enforce.as_str() {
            "lock" => (
                "system-lock-screen".to_string(),
                "NeedsAttention".to_string(),
            ),
            "warn" => ("dialog-warning".to_string(), "Active".to_string()),
            _ => ("appointment-soon".to_string(), "Active".to_string()),
        };

        Self {
            status,
            icon_name,
            title: fmt_remaining(effective),
        }
    }
}

fn fmt_remaining(secs: i64) -> String {
    if secs <= 0 {
        return "Time's up!".to_string();
    }
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{h}h {m:02}m")
    } else if m > 0 {
        format!("{m}m {s:02}s")
    } else {
        format!("{s}s")
    }
}

fn read_state(path: &PathBuf) -> TrayState {
    let Ok(content) = std::fs::read_to_string(path) else {
        return TrayState::passive();
    };
    match serde_json::from_str::<StatusFile>(&content) {
        Ok(sf) => TrayState::from_file(&sf),
        Err(_) => TrayState::passive(),
    }
}

// ── StatusNotifierItem D-Bus interface ────────────────────────────────────────

struct Sni {
    state: Arc<Mutex<TrayState>>,
}

#[interface(name = "org.kde.StatusNotifierItem")]
impl Sni {
    #[zbus(property)]
    fn category(&self) -> &str {
        "ApplicationStatus"
    }

    #[zbus(property)]
    fn id(&self) -> &str {
        "ScreenGuard"
    }

    #[zbus(property)]
    async fn title(&self) -> String {
        self.state.lock().await.title.clone()
    }

    #[zbus(property)]
    async fn status(&self) -> String {
        self.state.lock().await.status.clone()
    }

    #[zbus(property)]
    async fn icon_name(&self) -> String {
        self.state.lock().await.icon_name.clone()
    }

    fn activate(&self, _x: i32, _y: i32) {}
    fn context_menu(&self, _x: i32, _y: i32) {}
    fn secondary_activate(&self, _x: i32, _y: i32) {}

    #[zbus(signal)]
    async fn new_title(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn new_icon(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn new_status(emitter: &SignalEmitter<'_>, status: &str) -> zbus::Result<()>;
}

// ── StatusNotifierWatcher proxy ───────────────────────────────────────────────

#[proxy(
    interface = "org.kde.StatusNotifierWatcher",
    default_service = "org.kde.StatusNotifierWatcher",
    default_path = "/StatusNotifierWatcher"
)]
trait StatusNotifierWatcher {
    fn register_status_notifier_item(&self, service: &str) -> zbus::Result<()>;
}

// ── main ──────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let status_path = status_file_path();

    // Stay running even if the status file doesn't exist yet — it will appear
    // once the agent connects and sends its first RemainingUpdate. This removes
    // the login timing race where the tray would exit before the agent was ready.
    // The icon stays Passive (hidden) until the file appears.
    let state = Arc::new(Mutex::new(TrayState::passive()));

    let pid = std::process::id();
    let service_name = format!("org.kde.StatusNotifierItem-{pid}-1");

    let conn = Connection::session().await?;
    conn.request_name(service_name.as_str()).await?;
    conn.object_server()
        .at("/StatusNotifierItem", Sni { state: state.clone() })
        .await?;

    // Register with StatusNotifierWatcher — non-fatal if not running.
    if let Ok(watcher) = StatusNotifierWatcherProxy::new(&conn).await {
        let _ = watcher
            .register_status_notifier_item(&service_name)
            .await;
    }

    let iface_ref = conn
        .object_server()
        .interface::<_, Sni>("/StatusNotifierItem")
        .await?;
    let signal_emitter = iface_ref.signal_emitter().clone();

    // Poll every second and emit SNI signals when state changes.
    let mut ticker = tokio::time::interval(Duration::from_secs(1));
    loop {
        ticker.tick().await;

        let new_state = read_state(&status_path);
        let mut current = state.lock().await;

        if new_state != *current {
            let new_status = new_state.status.clone();
            let icon_changed = new_state.icon_name != current.icon_name;
            let status_changed = new_state.status != current.status;
            let title_changed = new_state.title != current.title;
            *current = new_state;
            drop(current);

            if title_changed {
                let _ = Sni::new_title(&signal_emitter).await;
            }
            if icon_changed {
                let _ = Sni::new_icon(&signal_emitter).await;
            }
            if status_changed {
                let _ = Sni::new_status(&signal_emitter, &new_status).await;
            }
        }
    }
}
