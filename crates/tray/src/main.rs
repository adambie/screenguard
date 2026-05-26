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

// ── tray state ────────────────────────────────────────────────────────────────

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
            icon_name: "chronometer".into(),
            title: "ScreenGuard".into(),
        }
    }

    fn from_file(sf: &StatusFile) -> Self {
        let now_ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        if now_ts.saturating_sub(sf.written_at) > 120 {
            return Self::passive();
        }

        let elapsed = now_ts.saturating_sub(sf.written_at) as i64;
        let effective = (sf.remaining_seconds - elapsed).max(0);

        match sf.enforce.as_str() {
            "lock" => Self {
                status: "NeedsAttention".into(),
                icon_name: "system-lock-screen".into(),
                title: "Locked".into(),
            },
            "warn" => Self {
                status: "Active".into(),
                icon_name: "dialog-warning".into(),
                title: fmt_remaining(effective),
            },
            _ => Self {
                status: "Active".into(),
                icon_name: "chronometer".into(),
                // More than 2 h → treat as unlimited for display purposes.
                title: if effective > 2 * 3600 {
                    "Unlimited".into()
                } else {
                    fmt_remaining(effective)
                },
            },
        }
    }

    fn is_active(&self) -> bool {
        self.status != "Passive"
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

// ── bitmap text renderer (for IconPixmap) ─────────────────────────────────────
//
// Renders a short string into a 22×22 ARGB32 icon so the time is always
// visible in KDE Plasma's compact system tray without needing to hover.
// Uses a 4×5 pixel font for digits / letters, centred on the canvas.

const FONT_W: usize = 4;
const FONT_H: usize = 5;

// Glyphs for: 0-9, h, m, s, U (Unlimited), L (Locked), space
// Each glyph is FONT_H rows of FONT_W bits, stored MSB-first.
const GLYPHS: &[(char, [u8; FONT_H])] = &[
    ('0', [0b0110, 0b1010, 0b1010, 0b1010, 0b0110]),
    ('1', [0b0010, 0b0110, 0b0010, 0b0010, 0b0111]),
    ('2', [0b0110, 0b1010, 0b0100, 0b1000, 0b1110]),
    ('3', [0b1110, 0b0010, 0b0110, 0b0010, 0b1110]),
    ('4', [0b1010, 0b1010, 0b1110, 0b0010, 0b0010]),
    ('5', [0b1110, 0b1000, 0b1110, 0b0010, 0b1110]),
    ('6', [0b0110, 0b1000, 0b1110, 0b1010, 0b0110]),
    ('7', [0b1110, 0b0010, 0b0100, 0b0100, 0b0100]),
    ('8', [0b0110, 0b1010, 0b0110, 0b1010, 0b0110]),
    ('9', [0b0110, 0b1010, 0b0110, 0b0010, 0b0110]),
    ('h', [0b1000, 0b1000, 0b1110, 0b1010, 0b1010]),
    ('m', [0b0000, 0b1010, 0b1110, 0b1010, 0b1010]),
    ('s', [0b0110, 0b1000, 0b0110, 0b0010, 0b1100]),
    ('U', [0b1010, 0b1010, 0b1010, 0b1010, 0b0110]),
    ('L', [0b1000, 0b1000, 0b1000, 0b1000, 0b1110]),
    ('!', [0b0100, 0b0100, 0b0100, 0b0000, 0b0100]),
    (' ', [0b0000, 0b0000, 0b0000, 0b0000, 0b0000]),
];

fn glyph(c: char) -> [u8; FONT_H] {
    GLYPHS
        .iter()
        .find(|(ch, _)| *ch == c)
        .map(|(_, g)| *g)
        .unwrap_or([0u8; FONT_H])
}

/// Render `text` into a 22×22 ARGB32 (big-endian) pixel buffer.
/// Returns (width, height, argb32_bytes) suitable for IconPixmap.
fn render_text_icon(text: &str, fg: u32, bg: u32) -> (i32, i32, Vec<u8>) {
    const W: usize = 22;
    const H: usize = 22;
    const SCALE: usize = 2; // each font pixel → 2×2 screen pixels

    let chars: Vec<char> = text.chars().collect();
    let text_w = chars.len() * (FONT_W * SCALE + 1);
    let text_h = FONT_H * SCALE;

    let x0 = ((W as isize - text_w as isize) / 2).max(0) as usize;
    let y0 = ((H as isize - text_h as isize) / 2).max(0) as usize;

    let mut buf = vec![bg; W * H];

    for (ci, &c) in chars.iter().enumerate() {
        let glyph_data = glyph(c);
        let cx = x0 + ci * (FONT_W * SCALE + 1);
        for row in 0..FONT_H {
            for col in 0..FONT_W {
                let bit = (glyph_data[row] >> (FONT_W - 1 - col)) & 1;
                if bit == 1 {
                    for dy in 0..SCALE {
                        for dx in 0..SCALE {
                            let px = cx + col * SCALE + dx;
                            let py = y0 + row * SCALE + dy;
                            if px < W && py < H {
                                buf[py * W + px] = fg;
                            }
                        }
                    }
                }
            }
        }
    }

    // Convert u32 ARGB to big-endian bytes.
    let bytes: Vec<u8> = buf.iter().flat_map(|&px| px.to_be_bytes()).collect();
    (W as i32, H as i32, bytes)
}

fn make_pixmap(state: &TrayState) -> Vec<(i32, i32, Vec<u8>)> {
    // Choose foreground colour per state.
    let fg: u32 = match state.status.as_str() {
        "NeedsAttention" => 0xFF_FF4444, // red
        "Active" if state.icon_name == "dialog-warning" => 0xFF_FFA500, // orange
        _ => 0xFF_FFFFFF, // white
    };
    let bg: u32 = 0x00_000000; // transparent

    // Shorten the label to fit (max ~5 chars at scale 2 in 22px).
    let label: String = state.title.chars().take(6).collect();
    let (w, h, data) = render_text_icon(&label, fg, bg);
    vec![(w, h, data)]
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

    /// IconPixmap renders the remaining-time text so KDE's compact tray
    /// shows it without the user needing to hover.
    #[zbus(property)]
    async fn icon_pixmap(&self) -> Vec<(i32, i32, Vec<u8>)> {
        make_pixmap(&*self.state.lock().await)
    }

    /// XAyatanaLabel shows text next to the icon in Ubuntu GNOME AppIndicator.
    #[zbus(property)]
    async fn x_ayatana_label(&self) -> String {
        self.state.lock().await.title.clone()
    }

    #[zbus(property)]
    fn x_ayatana_label_guide(&self) -> &str {
        "00h 00m"
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

    #[zbus(signal)]
    async fn x_ayatana_new_label(
        emitter: &SignalEmitter<'_>,
        label: &str,
        guide: &str,
    ) -> zbus::Result<()>;
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

    // Wait until we have a non-Passive state before registering with the
    // StatusNotifierWatcher. Some shells (GNOME AppIndicator) cache the
    // initial Status and don't reliably handle a Passive→Active transition
    // shortly after registration, causing the icon to disappear immediately.
    let initial_state = loop {
        let s = read_state(&status_path);
        if s.is_active() {
            break s;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    };

    let state = Arc::new(Mutex::new(initial_state));

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
            let new_title = new_state.title.clone();
            let icon_changed = new_state.icon_name != current.icon_name
                || new_state.title != current.title; // pixmap changes with title
            let status_changed = new_state.status != current.status;
            let title_changed = new_state.title != current.title;
            *current = new_state;
            drop(current);

            if title_changed {
                let _ = Sni::new_title(&signal_emitter).await;
                let _ = Sni::x_ayatana_new_label(&signal_emitter, &new_title, "00h 00m").await;
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
