//! Which app does a process belong to? Implements the identification algorithm
//! and the "is it an app" filter of docs/feature-app-usage-limits.md (validated
//! in Phase 0 with `docs/spikes/app-scan.py`).

#![cfg_attr(not(test), allow(dead_code))]

use std::collections::HashMap;

use common::apps::{AppId, AppKind};

use crate::app_catalog::{Catalog, DesktopEntry};
use crate::app_scan::ProcInfo;

/// Root processes started this soon after the user's first process count as
/// autostart even without an `@autostart` unit (compositor `exec-once` etc.).
const AUTOSTART_WINDOW_TICKS: u64 = 60 * 100; // USER_HZ = 100
/// Chromium and every Electron app move their main process into a scope with
/// this id, whatever the real app is.
const SELF_SCOPE_IDS: &[&str] = &["org.chromium.Chromium"];
const MAX_DEPTH: usize = 64;
/// In a `java` command line: Minecraft main classes / mod loaders, and the
/// entry points of Prism Launcher and MultiMC (they pass the main class on stdin).
const MINECRAFT_MARKERS: &[&str] =
    &["net.minecraft", "cpw.mods", "fabricmc", "quiltmc", "org.prismlauncher.entrypoint", "org.multimc.entrypoint"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Launch {
    /// Counts as usage.
    App,
    /// Tracked, not counted by default.
    Autostart,
    /// Not an app (`exe:` bucket).
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identified {
    pub app_id: AppId,
    pub launch: Launch,
    /// The `.desktop` entry it resolved to, for name and icon.
    pub desktop_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CgKind {
    App,
    Autostart,
    UserService,
    Session,
    Background,
    Other,
}

/// cgroup path → (kind, unit). The unit is the first `.scope`/`.service` below the
/// slice: only slices nest, deeper cgroups are the unit's own (Konsole's
/// `app-org.kde.konsole-<pid>.scope/main.scope`).
fn classify(path: &str) -> (CgKind, &str) {
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let last = parts.last().copied().unwrap_or("");
    if last.starts_with("snap.") {
        return (CgKind::App, last);
    }
    for (slice, kind) in [("app.slice", None), ("session.slice", Some(CgKind::Session)), ("background.slice", Some(CgKind::Background))] {
        if let Some(i) = parts.iter().position(|p| *p == slice) {
            let unit = parts[i + 1..]
                .iter()
                .find(|p| p.ends_with(".scope") || p.ends_with(".service"))
                .copied()
                .unwrap_or(last);
            return match kind {
                Some(k) => (k, unit),
                None if !unit.starts_with("app-") => (CgKind::UserService, unit),
                None if unit.ends_with("@autostart.service") => (CgKind::Autostart, unit),
                None => (CgKind::App, unit),
            };
        }
    }
    (CgKind::Other, last)
}

/// systemd unit-name unescaping (`\x2d` → `-`).
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("\\x") {
        out.push_str(&rest[..i]);
        match rest.get(i + 2..i + 4).and_then(|h| u8::from_str_radix(h, 16).ok()) {
            Some(b) => {
                out.push(b as char);
                rest = &rest[i + 4..];
            }
            None => {
                out.push_str("\\x");
                rest = &rest[i + 2..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// `app[-<launcher>]-<ApplicationID>-<RANDOM>.scope`,
/// `app[-<launcher>]-<ApplicationID>[@<RANDOM>].service`, `snap.<name>.<app>-<uuid>.scope`
/// → (launcher, application id).
fn parse_unit(unit: &str) -> Option<(Option<String>, String)> {
    if let Some(rest) = unit.strip_prefix("snap.") {
        return Some((Some("snap".into()), rest.split('.').next()?.to_string()));
    }
    let rest = unit.strip_prefix("app-")?;
    let body = if let Some(s) = rest.strip_suffix(".scope") {
        s.rsplit_once('-')?.0
    } else {
        let s = rest.strip_suffix(".service")?;
        s.split_once('@').map_or(s, |(a, _)| a)
    };
    // real '-' inside ids are escaped as \x2d
    match body.split_once('-') {
        Some((launcher, app)) => Some((Some(unescape(launcher)), unescape(app))),
        None => Some((None, unescape(body))),
    }
}

fn basename(s: &str) -> &str {
    s.rsplit(['/', '\\']).next().unwrap_or(s)
}

struct Ctx<'a> {
    procs: &'a HashMap<u32, ProcInfo>,
    cat: &'a Catalog,
    children: HashMap<u32, Vec<u32>>,
    /// unit → its processes, oldest first
    units: HashMap<&'a str, Vec<&'a ProcInfo>>,
    session_start: HashMap<u32, u64>,
}

/// Identifies every process in `procs` (the managed users' processes).
pub fn identify_all(procs: &HashMap<u32, ProcInfo>, cat: &Catalog) -> HashMap<u32, Identified> {
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut units: HashMap<&str, Vec<&ProcInfo>> = HashMap::new();
    let mut session_start: HashMap<u32, u64> = HashMap::new();
    for p in procs.values() {
        children.entry(p.ppid).or_default().push(p.pid);
        units.entry(classify(&p.cgroup).1).or_default().push(p);
        let s = session_start.entry(p.uid).or_insert(p.start_ticks);
        *s = (*s).min(p.start_ticks);
    }
    for v in units.values_mut() {
        v.sort_by_key(|p| (p.start_ticks, p.pid));
    }
    let ctx = Ctx { procs, cat, children, units, session_start };
    procs.values().map(|p| (p.pid, ctx.identify(p))).collect()
}

impl<'a> Ctx<'a> {
    fn parent(&self, p: &ProcInfo) -> Option<&'a ProcInfo> {
        self.procs.get(&p.ppid)
    }

    fn ancestors(&self, p: &'a ProcInfo) -> impl Iterator<Item = &'a ProcInfo> + '_ {
        std::iter::successors(Some(p), |q| self.parent(q)).take(MAX_DEPTH)
    }

    fn other(&self, p: &ProcInfo) -> Identified {
        let name = p.exe.as_deref().map(basename).unwrap_or(&p.comm);
        let app_id = AppId::new(AppKind::Exe, name)
            .unwrap_or_else(|| AppId { kind: AppKind::Exe, name: "?".into() });
        Identified { app_id, launch: Launch::Other, desktop_id: None }
    }

    fn desktop(&self, e: &DesktopEntry, launch: Launch) -> Option<Identified> {
        Some(Identified { app_id: AppId::new(AppKind::Desktop, &e.id)?, launch, desktop_id: Some(e.id.clone()) })
    }

    fn identify(&self, p: &'a ProcInfo) -> Identified {
        if let Some(id) = self.special_case(p) {
            return Identified { app_id: id, launch: Launch::App, desktop_id: None };
        }

        let (mut kind, mut unit) = classify(&p.cgroup);
        if matches!(kind, CgKind::UserService | CgKind::Background) {
            // real services: portals, syncthing, input methods. session.slice is
            // not excluded: uwsm keybinding launches stay in the compositor's unit.
            return self.other(p);
        }
        let mut parsed = matches!(kind, CgKind::App | CgKind::Autostart).then(|| parse_unit(unit)).flatten();
        if kind == CgKind::App && parsed.as_ref().is_some_and(|(_, a)| SELF_SCOPE_IDS.contains(&a.as_str())) {
            // moved itself out of the launcher's unit; its children stayed there
            let child_unit = self.children.get(&p.pid).into_iter().flatten().find_map(|c| {
                let (k, u) = classify(&self.procs[c].cgroup);
                (u != unit && matches!(k, CgKind::App | CgKind::Autostart)).then_some((k, u))
            });
            match child_unit {
                Some((k, u)) => {
                    (kind, unit) = (k, u);
                    parsed = parse_unit(u);
                }
                None => parsed = None,
            }
        }
        let autostart_unit = kind == CgKind::Autostart;

        // 2. Flatpak / Snap
        if let Some((Some(launcher), app)) = &parsed {
            let kind = match launcher.as_str() {
                "flatpak" => Some(AppKind::Flatpak),
                "snap" => Some(AppKind::Snap),
                _ => None,
            };
            if let Some(id) = kind.and_then(|k| AppId::new(k, app.as_str())) {
                let launch = if autostart_unit { Launch::Autostart } else { Launch::App };
                let desktop_id = self.cat.get(app).map(|e| e.id.clone());
                return Identified { app_id: id, launch, desktop_id };
            }
        }

        // 3. GIO launch env (before the unit: a nested GIO launch inherits the
        // launcher's unit, but the env names the new app)
        let mut found: Option<(&DesktopEntry, Option<&ProcInfo>)> = self.gio(p).map(|e| (e, None));

        // 4. app unit
        if found.is_none() {
            if let Some((_, app)) = &parsed {
                found = self.cat.by_unit_id(app).map(|e| (e, None)).or_else(|| {
                    // uwsm / xdg-terminal-exec units carry a command, not an id
                    self.units.get(unit)?.iter().find_map(|q| self.cat.by_process(q).map(|e| (e, None)))
                });
            }
        }

        // 5. process tree
        if found.is_none() {
            found = self.launch_root(p).map(|(root, e)| (e, Some(root)));
        }

        let Some((e, root)) = found.filter(|(e, _)| e.visible) else {
            return self.other(p);
        };
        if autostart_unit {
            return self.desktop(e, Launch::Autostart).unwrap_or_else(|| self.other(p));
        }
        let r = root.unwrap_or(p);
        let started_with_session =
            r.start_ticks.saturating_sub(self.session_start.get(&r.uid).copied().unwrap_or(0)) < AUTOSTART_WINDOW_TICKS;
        let root_bin = r.exe.as_deref().map(basename).unwrap_or(&r.comm);
        let launch = if started_with_session && (self.cat.is_autostart(&e.id) || self.cat.is_autostart(root_bin)) {
            Launch::Autostart
        } else {
            Launch::App
        };
        self.desktop(e, launch).unwrap_or_else(|| self.other(p))
    }

    /// Steam reaper, Minecraft Java, Wine `.exe` — on the process or an ancestor.
    /// Each rule is tried along the whole chain before the next one, so a Proton
    /// game (`Game.exe` under `reaper`) is `steam:`, not `wine:`.
    fn special_case(&self, p: &'a ProcInfo) -> Option<AppId> {
        let a0 = |q: &'a ProcInfo| q.argv.first().map(|s| basename(s)).unwrap_or("");
        for q in self.ancestors(p) {
            if a0(q) == "reaper" && q.argv.iter().any(|a| a == "SteamLaunch") {
                if let Some(id) = q.argv.iter().find_map(|a| a.strip_prefix("AppId=")) {
                    if let Some(id) = AppId::new(AppKind::Steam, id) {
                        return Some(id);
                    }
                }
            }
        }
        for q in self.ancestors(p) {
            if a0(q).starts_with("java") {
                let joined = q.argv.join(" ").to_lowercase();
                if MINECRAFT_MARKERS.iter().any(|m| joined.contains(m))
                    || q.argv.iter().any(|a| {
                        let b = basename(a).to_lowercase();
                        b.contains("minecraft") && b.ends_with(".jar")
                    })
                {
                    return AppId::new(AppKind::Minecraft, "java");
                }
            }
        }
        for q in self.ancestors(p) {
            for a in q.argv.iter().take(2) {
                if a.to_lowercase().ends_with(".exe") {
                    return AppId::new(AppKind::Wine, basename(a).to_lowercase());
                }
            }
        }
        None
    }

    fn gio(&self, p: &'a ProcInfo) -> Option<&'a DesktopEntry> {
        let (file, gio_pid) = (p.gio_desktop_file.as_deref()?, p.gio_pid?);
        // launched process must be alive, an ancestor, owned by the same user and
        // not younger than p (else the env is stale or the PID was reused)
        let launched = self.ancestors(p).find(|q| q.pid == gio_pid)?;
        if launched.pid == 1 || launched.uid != p.uid || launched.start_ticks > p.start_ticks {
            return None;
        }
        let id = basename(file).strip_suffix(".desktop")?;
        self.cat.get(id)
    }

    /// Highest ancestor that resolves to a visible entry; the walk stops when the
    /// parent is not a managed user's process (PID 1, root, another user) or is the
    /// user's systemd. A terminal or file manager owns what it starts.
    fn launch_root(&self, p: &'a ProcInfo) -> Option<(&'a ProcInfo, &'a DesktopEntry)> {
        let mut best = None;
        for q in self.ancestors(p) {
            if let Some(e) = self.cat.by_process(q).filter(|e| e.visible) {
                best = Some((q, e));
            }
            match self.parent(q) {
                Some(parent) if parent.uid == q.uid && parent.comm != "systemd" => {}
                _ => break,
            }
        }
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_catalog::fixture;
    use crate::app_scan::fake_proc::{FakeProc, P};
    use crate::app_scan::ProcScanner;
    use std::collections::HashSet;

    const U: &str = "/user.slice/user-1000.slice/user@1000.service";

    struct World {
        fp: FakeProc,
    }

    impl World {
        fn new() -> Self {
            let fp = FakeProc::new();
            // the user's systemd and a login session leader
            fp.add(P { pid: 900, ppid: 1, start: 100, comm: "systemd", argv: &["/usr/lib/systemd/systemd", "--user"], cgroup: "/user.slice/user-1000.slice/user@1000.service/init.scope", ..Default::default() });
            World { fp }
        }

        fn add(&self, p: P) -> &Self {
            self.fp.add(p);
            self
        }

        fn run(&self) -> HashMap<u32, Identified> {
            let apps = self.fp.path().join("share/applications");
            let auto = self.fp.path().join("autostart");
            fixture::standard(&apps, &auto);
            let cat = Catalog::load(&[apps], &[auto]);
            let procs = ProcScanner::with_root(self.fp.path()).scan(&HashSet::from([1000]));
            identify_all(&procs, &cat)
        }
    }

    fn cg(rest: &str) -> String {
        format!("{U}/{rest}")
    }

    #[track_caller]
    fn is(r: &HashMap<u32, Identified>, pid: u32, app_id: &str, launch: Launch) {
        let got = &r[&pid];
        assert_eq!((got.app_id.to_string().as_str(), got.launch), (app_id, launch), "pid {pid}");
    }

    #[test]
    fn unit_parsing() {
        assert_eq!(parse_unit("app-gnome-org.gnome.Nautilus-4242.scope"), Some((Some("gnome".into()), "org.gnome.Nautilus".into())));
        assert_eq!(parse_unit("app-org.kde.dolphin@c0a5622a.service"), Some((None, "org.kde.dolphin".into())));
        assert_eq!(parse_unit("app-Hyprland-xdg\\x2dterminal\\x2dexec-4ac1367f.scope"), Some((Some("Hyprland".into()), "xdg-terminal-exec".into())));
        assert_eq!(parse_unit("app-flatpak-org.mozilla.firefox-12345.scope"), Some((Some("flatpak".into()), "org.mozilla.firefox".into())));
        assert_eq!(parse_unit("snap.spotify.spotify-1b2c.scope"), Some((Some("snap".into()), "spotify".into())));
        assert_eq!(parse_unit("app-remmina\\x2dapplet@autostart.service"), Some((None, "remmina-applet".into())));
        assert_eq!(parse_unit("dconf.service"), None);
        assert_eq!(classify(&cg("app.slice/app-org.kde.konsole-4858.scope/main.scope")), (CgKind::App, "app-org.kde.konsole-4858.scope"));
        assert_eq!(classify(&cg("app.slice/app-graphical.slice/app-x@autostart.service")).0, CgKind::Autostart);
        assert_eq!(classify(&cg("app.slice/dconf.service")).0, CgKind::UserService);
        assert_eq!(classify("/user.slice/user-1000.slice/session-2.scope").0, CgKind::Other);
    }

    #[test]
    fn gnome_scope() {
        let w = World::new();
        let c = cg("app.slice/app-gnome-org.gnome.TextEditor-5000.scope");
        w.add(P { pid: 5000, ppid: 900, start: 90_000, comm: "gnome-text-edit", exe: Some("/usr/bin/gnome-text-editor"), argv: &["gnome-text-editor"], cgroup: &c, ..Default::default() });
        is(&w.run(), 5000, "desktop:org.gnome.TextEditor", Launch::App);
    }

    #[test]
    fn kde_service_with_helpers_and_sub_cgroups() {
        let w = World::new();
        let d = cg("app.slice/app-org.kde.dolphin@c0a5.service");
        let k = cg("app.slice/app-org.kde.konsole-6000.scope/tab(1).scope");
        w.add(P { pid: 5045, ppid: 900, start: 90_000, comm: "dolphin", exe: Some("/usr/bin/dolphin"), argv: &["/usr/bin/dolphin"], cgroup: &d, ..Default::default() })
            .add(P { pid: 5080, ppid: 5045, start: 90_100, comm: "kioworker", exe: Some("/usr/lib/kf6/kioworker"), argv: &["kioworker", "file"], cgroup: &d, ..Default::default() })
            .add(P { pid: 6000, ppid: 900, start: 90_000, comm: "konsole", exe: Some("/usr/bin/konsole"), argv: &["/usr/bin/konsole"], cgroup: &k, ..Default::default() });
        let r = w.run();
        is(&r, 5045, "desktop:org.kde.dolphin", Launch::App);
        is(&r, 5080, "desktop:org.kde.dolphin", Launch::App);
        is(&r, 6000, "desktop:org.kde.konsole", Launch::App);
    }

    #[test]
    fn uwsm_command_scope_resolves_via_oldest_process() {
        let w = World::new();
        let c = cg("app.slice/app-graphical.slice/app-Hyprland-xdg\\x2dterminal\\x2dexec-4ac1367f.scope");
        w.add(P { pid: 8782, ppid: 900, start: 90_000, comm: "foot", exe: Some("/usr/bin/foot"), argv: &["foot"], cgroup: &c, ..Default::default() })
            .add(P { pid: 8803, ppid: 8782, start: 90_010, comm: "bash", exe: Some("/usr/bin/bash"), argv: &["bash"], cgroup: &c, ..Default::default() });
        let r = w.run();
        is(&r, 8782, "desktop:foot", Launch::App);
        is(&r, 8803, "desktop:foot", Launch::App);
    }

    #[test]
    fn chromium_and_electron_self_scopes() {
        let w = World::new();
        let launcher = cg("app.slice/app-graphical.slice/app-Hyprland-chromium-2125de24.scope");
        let own = cg("app.slice/app-org.chromium.Chromium-3350.scope");
        let bw_own = cg("app.slice/app-org.chromium.Chromium-1691.scope");
        let bw_auto = cg("app.slice/app-graphical.slice/app-bitwarden@autostart.service");
        w.add(P { pid: 3350, ppid: 900, start: 90_000, comm: "chromium", exe: Some("/usr/lib/chromium/chromium"), argv: &["/usr/lib/chromium/chromium"], cgroup: &own, ..Default::default() })
            .add(P { pid: 3374, ppid: 3350, start: 90_010, comm: "chromium", exe: Some("/usr/lib/chromium/chromium"), argv: &["/usr/lib/chromium/chromium", "--type=zygote"], cgroup: &launcher, ..Default::default() })
            .add(P { pid: 11661, ppid: 3374, start: 95_000, comm: "chromium", exe: Some("/usr/lib/chromium/chromium"), argv: &["/usr/lib/chromium/chromium", "--type=renderer"], cgroup: &own, ..Default::default() })
            // Electron: exe unreadable without root, argv names the archive
            .add(P { pid: 1691, ppid: 900, start: 500, comm: "electron", argv: &["/usr/lib/electron39/electron", "/usr/lib/bitwarden/app.asar"], cgroup: &bw_own, ..Default::default() })
            .add(P { pid: 1967, ppid: 1691, start: 510, comm: "electron", argv: &["/usr/lib/electron39/electron", "--type=zygote"], cgroup: &bw_auto, ..Default::default() });
        let r = w.run();
        is(&r, 3350, "desktop:chromium", Launch::App);
        is(&r, 3374, "desktop:chromium", Launch::App);
        is(&r, 11661, "desktop:chromium", Launch::App);
        is(&r, 1691, "desktop:bitwarden", Launch::Autostart);
        is(&r, 1967, "desktop:bitwarden", Launch::Autostart);
    }

    #[test]
    fn flatpak_and_snap() {
        let w = World::new();
        let f = cg("app.slice/app-flatpak-org.mozilla.firefox-12345.scope");
        w.add(P { pid: 7000, ppid: 900, start: 90_000, comm: "bwrap", exe: Some("/usr/bin/bwrap"), argv: &["bwrap", "--args", "40"], cgroup: &f, ..Default::default() })
            .add(P { pid: 7100, ppid: 900, start: 90_000, comm: "spotify", argv: &["/snap/spotify/80/usr/share/spotify/spotify"], cgroup: "/user.slice/user-1000.slice/user@1000.service/app.slice/snap.spotify.spotify-1b2c3d.scope", ..Default::default() });
        let r = w.run();
        is(&r, 7000, "flatpak:org.mozilla.firefox", Launch::App);
        assert_eq!(r[&7000].desktop_id.as_deref(), Some("org.mozilla.firefox"));
        is(&r, 7100, "snap:spotify", Launch::App);
    }

    #[test]
    fn xdg_autostart_unit_and_session_start_heuristic() {
        let w = World::new();
        let a = cg("app.slice/app-graphical.slice/app-nextcloud@autostart.service");
        // XFCE-style: no units; nextcloud started right after login vs. hours later
        let s = "/user.slice/user-1000.slice/session-2.scope";
        w.add(P { pid: 1689, ppid: 900, start: 600, comm: "nextcloud", exe: Some("/usr/bin/nextcloud"), argv: &["nextcloud", "--background"], cgroup: &a, ..Default::default() })
            .add(P { pid: 2000, ppid: 1999, start: 700, comm: "nextcloud", exe: Some("/usr/bin/nextcloud"), argv: &["nextcloud"], cgroup: s, ..Default::default() })
            .add(P { pid: 1999, ppid: 1, start: 150, comm: "xfce4-session", exe: Some("/usr/bin/xfce4-session"), argv: &["xfce4-session"], cgroup: s, ..Default::default() })
            .add(P { pid: 3000, ppid: 1999, start: 900_000, comm: "nextcloud", exe: Some("/usr/bin/nextcloud"), argv: &["nextcloud"], cgroup: s, ..Default::default() });
        let r = w.run();
        is(&r, 1689, "desktop:nextcloud", Launch::Autostart);
        is(&r, 2000, "desktop:nextcloud", Launch::Autostart);
        is(&r, 3000, "desktop:nextcloud", Launch::App);
    }

    #[test]
    fn gio_env_without_units_and_stale_env() {
        let w = World::new();
        let s = "/user.slice/user-1000.slice/session-3.scope";
        let gio = |pid: &str| format!("GIO_LAUNCHED_DESKTOP_FILE_PID={pid}");
        let (g8000, g1) = (gio("8000"), gio("1"));
        let file = "GIO_LAUNCHED_DESKTOP_FILE=/usr/share/applications/org.gnome.TextEditor.desktop";
        // Cinnamon-style: cinnamon (no visible entry) launches via GIO
        w.add(P { pid: 7900, ppid: 1, start: 200, comm: "cinnamon", exe: Some("/usr/bin/cinnamon"), argv: &["cinnamon"], cgroup: s, ..Default::default() })
            .add(P { pid: 8000, ppid: 7900, start: 90_000, comm: "gnome-text-edit", exe: Some("/usr/bin/gnome-text-editor"), argv: &["gnome-text-editor"], cgroup: s, env: &[file, &g8000], ..Default::default() })
            .add(P { pid: 8001, ppid: 8000, start: 90_001, comm: "helper", exe: Some("/usr/libexec/helper"), argv: &["helper"], cgroup: s, env: &[file, &g8000], ..Default::default() })
            // stale env pointing at PID 1: ignored, falls through to the tree
            .add(P { pid: 8100, ppid: 7900, start: 90_000, comm: "dolphin", exe: Some("/usr/bin/dolphin"), argv: &["dolphin"], cgroup: s, env: &[file, &g1], ..Default::default() });
        let r = w.run();
        is(&r, 8000, "desktop:org.gnome.TextEditor", Launch::App);
        is(&r, 8001, "desktop:org.gnome.TextEditor", Launch::App);
        is(&r, 8100, "desktop:org.kde.dolphin", Launch::App);
        is(&r, 7900, "exe:cinnamon", Launch::Other);
    }

    #[test]
    fn gio_launch_from_inside_another_apps_unit() {
        let w = World::new();
        let c = cg("app.slice/app-gnome-org.gnome.Nautilus-4242.scope");
        let file = "GIO_LAUNCHED_DESKTOP_FILE=/usr/share/applications/org.gnome.TextEditor.desktop";
        w.add(P { pid: 4242, ppid: 900, start: 90_000, comm: "nautilus", exe: Some("/usr/bin/nautilus"), argv: &["nautilus"], cgroup: &c, ..Default::default() })
            .add(P { pid: 4300, ppid: 4242, start: 91_000, comm: "gnome-text-edit", exe: Some("/usr/bin/gnome-text-editor"), argv: &["gnome-text-editor"], cgroup: &c, env: &[file, "GIO_LAUNCHED_DESKTOP_FILE_PID=4300"], ..Default::default() });
        let r = w.run();
        is(&r, 4242, "desktop:org.gnome.Nautilus", Launch::App);
        is(&r, 4300, "desktop:org.gnome.TextEditor", Launch::App);
    }

    #[test]
    fn plain_process_tree_and_terminal_children() {
        let w = World::new();
        let s = "/user.slice/user-1000.slice/session-2.scope";
        w.add(P { pid: 1999, ppid: 1, start: 150, comm: "xfce4-session", exe: Some("/usr/bin/xfce4-session"), argv: &["xfce4-session"], cgroup: s, ..Default::default() })
            .add(P { pid: 2100, ppid: 1999, start: 200, comm: "xfce4-panel", exe: Some("/usr/bin/xfce4-panel"), argv: &["xfce4-panel"], cgroup: s, ..Default::default() })
            .add(P { pid: 2200, ppid: 2100, start: 90_000, comm: "mousepad", exe: Some("/usr/bin/mousepad"), argv: &["mousepad"], cgroup: s, ..Default::default() })
            .add(P { pid: 2300, ppid: 2100, start: 90_000, comm: "xfce4-terminal", exe: Some("/usr/bin/xfce4-terminal"), argv: &["xfce4-terminal"], cgroup: s, ..Default::default() })
            .add(P { pid: 2301, ppid: 2300, start: 90_100, comm: "bash", exe: Some("/usr/bin/bash"), argv: &["bash"], cgroup: s, ..Default::default() })
            .add(P { pid: 2302, ppid: 2301, start: 90_200, comm: "vim", exe: Some("/usr/bin/vim"), argv: &["vim", "notes.txt"], cgroup: s, ..Default::default() });
        let r = w.run();
        is(&r, 2200, "desktop:mousepad", Launch::App);
        is(&r, 2300, "desktop:xfce4-terminal", Launch::App);
        is(&r, 2301, "desktop:xfce4-terminal", Launch::App);
        is(&r, 2302, "desktop:xfce4-terminal", Launch::App); // Q6: terminal owns it
        is(&r, 2100, "exe:xfce4-panel", Launch::Other);
        is(&r, 1999, "exe:xfce4-session", Launch::Other);
    }

    #[test]
    fn steam_minecraft_wine() {
        let w = World::new();
        let st = cg("app.slice/app-gnome-steam-3000.scope");
        w.add(P { pid: 3000, ppid: 900, start: 90_000, comm: "steam", exe: Some("/home/kid/.local/share/Steam/ubuntu12_32/steam"), argv: &["/home/kid/.local/share/Steam/ubuntu12_32/steam"], cgroup: &st, ..Default::default() })
            .add(P { pid: 3100, ppid: 3000, start: 91_000, comm: "reaper", exe: Some("/home/kid/.local/share/Steam/ubuntu12_32/reaper"), argv: &["/home/kid/.local/share/Steam/ubuntu12_32/reaper", "SteamLaunch", "AppId=1091500", "--", "/proton", "waitforexitandrun", "Cyberpunk2077.exe"], cgroup: &st, ..Default::default() })
            .add(P { pid: 3200, ppid: 3100, start: 91_100, comm: "Cyberpunk2077.e", argv: &["Z:\\games\\Cyberpunk2077.exe"], cgroup: &st, ..Default::default() })
            .add(P { pid: 4000, ppid: 900, start: 90_000, comm: "java", exe: Some("/usr/lib/jvm/java-21/bin/java"), argv: &["/usr/lib/jvm/java-21/bin/java", "-Xmx2G", "-cp", "/home/kid/.minecraft/libraries/x.jar", "net.minecraft.client.main.Main"], cgroup: &cg("app.slice/app-gnome-minecraft\\x2dlauncher-3999.scope"), ..Default::default() })
            .add(P { pid: 4100, ppid: 900, start: 90_000, comm: "java", exe: Some("/usr/lib/jvm/java-21/bin/java"), argv: &["/usr/lib/jvm/java-21/bin/java", "-cp", "/usr/share/prismlauncher/NewLaunch.jar", "org.prismlauncher.EntryPoint"], cgroup: &cg("app.slice/app-gnome-org.prismlauncher.PrismLauncher-4099.scope"), ..Default::default() })
            .add(P { pid: 4200, ppid: 900, start: 90_000, comm: "java", exe: Some("/usr/lib/jvm/java-21/bin/java"), argv: &["java", "-jar", "/opt/tool/server.jar"], cgroup: &cg("app.slice/app-gnome-tool-4199.scope"), ..Default::default() })
            .add(P { pid: 4500, ppid: 900, start: 90_000, comm: "game.exe", exe: Some("/usr/bin/wine64-preloader"), argv: &["C:\\Games\\game.exe"], cgroup: "/user.slice/user-1000.slice/session-2.scope", ..Default::default() });
        let r = w.run();
        is(&r, 3000, "desktop:steam", Launch::App);
        is(&r, 3100, "steam:1091500", Launch::App);
        is(&r, 3200, "steam:1091500", Launch::App);
        is(&r, 4000, "minecraft:java", Launch::App);
        is(&r, 4100, "minecraft:java", Launch::App);
        assert_ne!(r[&4200].app_id.to_string(), "minecraft:java");
        is(&r, 4500, "wine:game.exe", Launch::App);
    }

    #[test]
    fn deleted_binary_still_resolves() {
        let w = World::new();
        let c = cg("app.slice/app-gnome-firefox-3000.scope");
        w.add(P { pid: 3000, ppid: 900, start: 90_000, comm: "firefox", exe: Some("/usr/lib/firefox/firefox (deleted)"), argv: &["/usr/lib/firefox/firefox"], cgroup: &c, ..Default::default() });
        // unit id "firefox" resolves directly; check the process path too
        is(&w.run(), 3000, "desktop:firefox", Launch::App);
        let s = "/user.slice/user-1000.slice/session-2.scope";
        let w = World::new();
        w.add(P { pid: 3001, ppid: 1, start: 90_000, comm: "firefox", exe: Some("/usr/lib/firefox/firefox (deleted)"), argv: &["firefox-bin"], cgroup: s, ..Default::default() });
        is(&w.run(), 3001, "desktop:firefox", Launch::App);
    }

    #[test]
    fn desktop_components_are_not_apps() {
        let w = World::new();
        let ss = |u: &str| cg(&format!("session.slice/{u}"));
        let (shell, kded, portal, fcitx, tracker) = (
            ss("plasma-plasmashell.service"),
            ss("plasma-kded6.service"),
            cg("app.slice/xdg-desktop-portal-gtk.service"),
            cg("app.slice/app-graphical.slice/fcitx5.service"),
            cg("background.slice/localsearch-3.service"),
        );
        let hidden = cg("app.slice/app-graphical.slice/app-Hyprland-brave-1234.scope");
        let applet = cg("app.slice/app-graphical.slice/app-remmina\\x2dapplet@autostart.service");
        w.add(P { pid: 714, ppid: 900, start: 300, comm: "plasmashell", exe: Some("/usr/bin/plasmashell"), argv: &["/usr/bin/plasmashell"], cgroup: &shell, ..Default::default() })
            .add(P { pid: 702, ppid: 900, start: 300, comm: "kded6", exe: Some("/usr/bin/kded6"), argv: &["kded6"], cgroup: &kded, ..Default::default() })
            .add(P { pid: 925, ppid: 900, start: 300, comm: "xdg-desktop-por", exe: Some("/usr/lib/xdg-desktop-portal-gtk"), argv: &["/usr/lib/xdg-desktop-portal-gtk"], cgroup: &portal, ..Default::default() })
            .add(P { pid: 1707, ppid: 900, start: 300, comm: "fcitx5", exe: Some("/usr/bin/fcitx5"), argv: &["fcitx5"], cgroup: &fcitx, ..Default::default() })
            .add(P { pid: 1800, ppid: 900, start: 300, comm: "localsearch-3", exe: Some("/usr/lib/localsearch-3"), argv: &["/usr/lib/localsearch-3"], cgroup: &tracker, ..Default::default() })
            .add(P { pid: 1900, ppid: 900, start: 90_000, comm: "brave", exe: Some("/usr/bin/brave"), argv: &["brave"], cgroup: &hidden, ..Default::default() })
            .add(P { pid: 1701, ppid: 900, start: 300, comm: "remmina", exe: Some("/usr/bin/remmina"), argv: &["remmina", "-i"], cgroup: &applet, ..Default::default() });
        let r = w.run();
        for pid in [714, 702, 925, 1707, 1800, 1900, 1701] {
            assert_eq!(r[&pid].launch, Launch::Other, "pid {pid} ({:?})", r[&pid]);
            assert!(!r[&pid].app_id.is_app());
        }
        // the user's systemd itself
        is(&r, 900, "exe:systemd", Launch::Other);
    }
}

#[cfg(test)]
mod real {
    /// Manual check on a real session (not root: only your own processes):
    /// `cargo test -p agent print_real_scan -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn print_real_scan() {
        use super::*;
        use std::collections::{BTreeMap, HashSet};
        let status = std::fs::read_to_string("/proc/self/status").unwrap();
        let uid: u32 = status.lines().find_map(|l| l.strip_prefix("Uid:")).unwrap().split_whitespace().next().unwrap().parse().unwrap();
        let home = std::env::var("HOME").unwrap();
        let (apps, auto) = Catalog::dirs_for_home(std::path::Path::new(&home));
        let cat = Catalog::load(&apps, &auto);
        let procs = crate::app_scan::ProcScanner::new().scan(&HashSet::from([uid]));
        let mut summary: BTreeMap<(String, String), usize> = BTreeMap::new();
        for id in identify_all(&procs, &cat).values() {
            *summary.entry((format!("{:?}", id.launch), id.app_id.to_string())).or_default() += 1;
        }
        for ((launch, app), n) in summary {
            if launch != "Other" {
                println!("{launch:<10} {n:>4}  {app}");
            }
        }
    }
}
