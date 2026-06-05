use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;
use zbus::{interface, object_server::SignalEmitter, proxy, Connection};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, StructureBuilder, Value};

// ── agent D-Bus proxy (system bus) ────────────────────────────────────────────

#[proxy(
    interface = "org.screenguard.Agent1",
    default_service = "org.screenguard.Agent",
    default_path = "/org/screenguard/Agent",
)]
trait AgentInterface {
    /// Returns (remaining_seconds, enforce, updated_at, server_http_url).
    async fn status(&self, uid: u32) -> zbus::Result<(i64, String, u64, String)>;
}

// ── tray state ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
struct TrayState {
    status: String,
    icon_name: String,
    title: String,
    tooltip: String,
}

impl TrayState {
    fn passive() -> Self {
        Self {
            status: "Passive".into(),
            icon_name: "chronometer".into(),
            title: "ScreenGuard".into(),
            tooltip: "Status unavailable".into(),
        }
    }

    fn from_values(remaining_seconds: i64, enforce: &str, updated_at: u64) -> Self {
        let now_ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        if updated_at == 0 || now_ts.saturating_sub(updated_at) > 120 {
            return Self::passive();
        }

        let elapsed = now_ts.saturating_sub(updated_at) as i64;
        let effective = (remaining_seconds - elapsed).max(0);

        match enforce {
            "lock" => Self {
                status: "NeedsAttention".into(),
                icon_name: "system-lock-screen".into(),
                title: "Locked".into(),
                tooltip: "Screen time limit reached".into(),
            },
            "warn" => Self {
                status: "Active".into(),
                icon_name: "dialog-warning".into(),
                title: fmt_remaining(effective),
                tooltip: format!("Warning — {} remaining", fmt_remaining(effective)),
            },
            _ => {
                let (title, tooltip) = if effective > 2 * 3600 {
                    ("Unlimited".into(), "No time limit today".into())
                } else {
                    let t = fmt_remaining(effective);
                    let tip = format!("{t} remaining today");
                    (t, tip)
                };
                Self {
                    status: "Active".into(),
                    icon_name: "chronometer".into(),
                    title,
                    tooltip,
                }
            }
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

fn read_uid() -> u32 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("Uid:"))?
                .split_whitespace()
                .nth(1)?
                .parse::<u32>()
                .ok()
        })
        .unwrap_or(1000)
}

// ── bitmap font renderer ──────────────────────────────────────────────────────

const FONT_W: usize = 4;
const FONT_H: usize = 5;

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

fn render_text_icon(text: &str, fg: u32, bg: u32) -> (i32, i32, Vec<u8>) {
    const W: usize = 22;
    const H: usize = 22;
    const SCALE: usize = 2;

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

    let bytes: Vec<u8> = buf.iter().flat_map(|&px| px.to_be_bytes()).collect();
    (W as i32, H as i32, bytes)
}

/// Short label for the KDE pixmap: first space-separated word of title.
/// "1h 23m" → "1h", "45m 30s" → "45m", "Unlimited" → "U", "Locked" → "L!"
fn pixmap_label(state: &TrayState) -> String {
    match state.title.as_str() {
        "Unlimited" => "U".into(),
        "Locked" => "L!".into(),
        other => other.split_whitespace().next().unwrap_or("").to_string(),
    }
}

fn make_pixmap(state: &TrayState) -> Vec<(i32, i32, Vec<u8>)> {
    let fg: u32 = match state.status.as_str() {
        "NeedsAttention" => 0xFF_FF4444,
        "Active" if state.icon_name == "dialog-warning" => 0xFF_FFA500,
        _ => 0xFF_FFFFFF,
    };
    let bg: u32 = 0x00_000000;
    let label = pixmap_label(state);
    let (w, h, data) = render_text_icon(&label, fg, bg);
    vec![(w, h, data)]
}

// ── com.canonical.dbusmenu ────────────────────────────────────────────────────

type MenuProps = HashMap<String, OwnedValue>;
type MenuItem = (i32, MenuProps, Vec<OwnedValue>);

fn str_ov(s: &str) -> OwnedValue {
    OwnedValue::try_from(Value::from(s.to_string())).unwrap()
}

fn item_ov(id: i32, props: MenuProps) -> OwnedValue {
    let children: Vec<OwnedValue> = vec![];
    let s = StructureBuilder::new()
        .append_field(Value::I32(id))
        .append_field(Value::from(props))
        .append_field(Value::from(children))
        .build()
        .expect("menu item");
    OwnedValue::try_from(s).expect("menu item owned")
}

fn label_item(id: i32, label: &str, enabled: bool) -> OwnedValue {
    let mut props = MenuProps::new();
    props.insert("label".into(), str_ov(label));
    props.insert("enabled".into(), OwnedValue::from(enabled));
    item_ov(id, props)
}

fn separator_item(id: i32) -> OwnedValue {
    let mut props = MenuProps::new();
    props.insert("type".into(), str_ov("separator"));
    item_ov(id, props)
}

struct DbusMenu {
    server_url: Arc<std::sync::Mutex<String>>,
}

#[interface(name = "com.canonical.dbusmenu")]
impl DbusMenu {
    fn get_layout(
        &self,
        _parent_id: i32,
        _recursion: i32,
        _props: Vec<String>,
    ) -> (u32, MenuItem) {
        let has_url = !self.server_url.lock().unwrap().is_empty();
        let children = vec![
            label_item(1, "Open Admin Page", has_url),
            separator_item(2),
            label_item(99, "Quit", true),
        ];
        let root: MenuItem = (0, MenuProps::new(), children);
        (1u32, root)
    }

    fn get_group_properties(
        &self,
        _ids: Vec<i32>,
        _props: Vec<String>,
    ) -> Vec<(i32, MenuProps)> {
        vec![]
    }

    fn event(&self, id: i32, event_id: String, _data: OwnedValue, _ts: u32) {
        if event_id != "clicked" {
            return;
        }
        match id {
            1 => {
                let url = self.server_url.lock().unwrap().clone();
                if !url.is_empty() {
                    let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
                }
            }
            99 => std::process::exit(0),
            _ => {}
        }
    }

    fn about_to_show(&self, _id: i32) -> bool {
        false
    }

    fn event_group(&self, _events: Vec<(i32, String, OwnedValue, u32)>) -> Vec<i32> {
        vec![]
    }

    fn about_to_show_group(&self, _ids: Vec<i32>) -> (Vec<i32>, Vec<i32>) {
        (vec![], vec![])
    }

    #[zbus(property)]
    fn version(&self) -> u32 {
        4
    }

    #[zbus(property)]
    fn status(&self) -> &str {
        "normal"
    }

    #[zbus(property)]
    fn text_direction(&self) -> &str {
        "ltr"
    }

    #[zbus(property)]
    fn icon_theme_path(&self) -> Vec<String> {
        vec![]
    }

    #[zbus(signal)]
    async fn layout_updated(
        emitter: &SignalEmitter<'_>,
        revision: u32,
        parent: i32,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn items_properties_updated(
        emitter: &SignalEmitter<'_>,
        updated: Vec<(i32, MenuProps)>,
        removed: Vec<(i32, Vec<String>)>,
    ) -> zbus::Result<()>;
}

// ── StatusNotifierItem ────────────────────────────────────────────────────────

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

    #[zbus(property)]
    async fn icon_pixmap(&self) -> Vec<(i32, i32, Vec<u8>)> {
        make_pixmap(&*self.state.lock().await)
    }

    #[zbus(property)]
    async fn x_ayatana_label(&self) -> String {
        self.state.lock().await.title.clone()
    }

    #[zbus(property)]
    fn x_ayatana_label_guide(&self) -> &str {
        "00h 00m"
    }

    #[zbus(property)]
    fn menu(&self) -> OwnedObjectPath {
        OwnedObjectPath::try_from("/StatusNotifierItem/Menu").unwrap()
    }

    #[zbus(property)]
    fn tool_tip_icon_name(&self) -> &str {
        "chronometer"
    }

    #[zbus(property)]
    fn tool_tip_icon_pixmap(&self) -> Vec<(i32, i32, Vec<u8>)> {
        vec![]
    }

    #[zbus(property)]
    fn tool_tip_title(&self) -> &str {
        "ScreenGuard"
    }

    #[zbus(property)]
    async fn tool_tip_sub_title(&self) -> String {
        self.state.lock().await.tooltip.clone()
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
    async fn new_tool_tip(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

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
    let uid = read_uid();

    // Connect to system bus and wait for agent service.
    let sys_conn = Connection::system().await?;
    let agent = loop {
        if let Ok(p) = AgentInterfaceProxy::new(&sys_conn).await {
            if p.status(uid).await.is_ok() {
                break p;
            }
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    };

    // Wait for an Active state before registering with the SNI watcher.
    // GNOME AppIndicator caches the initial status and drops Passive icons.
    let initial_state = loop {
        if let Ok((remaining, enforce, updated_at, _)) = agent.status(uid).await {
            let s = TrayState::from_values(remaining, &enforce, updated_at);
            if s.is_active() {
                break s;
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    };

    let server_url = Arc::new(std::sync::Mutex::new(String::new()));
    let state = Arc::new(Mutex::new(initial_state));

    let pid = std::process::id();
    let service_name = format!("org.kde.StatusNotifierItem-{pid}-1");

    let conn = Connection::session().await?;
    conn.request_name(service_name.as_str()).await?;

    conn.object_server()
        .at("/StatusNotifierItem", Sni { state: state.clone() })
        .await?;

    conn.object_server()
        .at(
            "/StatusNotifierItem/Menu",
            DbusMenu { server_url: server_url.clone() },
        )
        .await?;

    if let Ok(watcher) = StatusNotifierWatcherProxy::new(&conn).await {
        let _ = watcher.register_status_notifier_item(&service_name).await;
    }

    let iface_ref = conn
        .object_server()
        .interface::<_, Sni>("/StatusNotifierItem")
        .await?;
    let emitter = iface_ref.signal_emitter().clone();

    // Poll agent every 2 s and push SNI signals on change.
    let mut ticker = tokio::time::interval(Duration::from_secs(2));
    loop {
        ticker.tick().await;

        let (remaining, enforce, updated_at, url) =
            match agent.status(uid).await {
                Ok(v) => v,
                Err(_) => continue,
            };

        *server_url.lock().unwrap() = url;

        let new_state = TrayState::from_values(remaining, &enforce, updated_at);
        let mut current = state.lock().await;

        if new_state != *current {
            let new_status = new_state.status.clone();
            let new_title = new_state.title.clone();
            let icon_changed = new_state.icon_name != current.icon_name
                || new_state.title != current.title;
            let status_changed = new_state.status != current.status;
            let title_changed = new_state.title != current.title;
            let tooltip_changed = new_state.tooltip != current.tooltip;
            *current = new_state;
            drop(current);

            if title_changed {
                let _ = Sni::new_title(&emitter).await;
                let _ = Sni::x_ayatana_new_label(&emitter, &new_title, "00h 00m").await;
            }
            if icon_changed {
                let _ = Sni::new_icon(&emitter).await;
            }
            if status_changed {
                let _ = Sni::new_status(&emitter, &new_status).await;
            }
            if tooltip_changed {
                let _ = Sni::new_tool_tip(&emitter).await;
            }
        }
    }
}
