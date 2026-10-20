//! `.desktop` catalogue for per-app usage (docs/feature-app-usage-limits.md).
//!
//! Maps unit ids, executables and scripts to visible launcher entries. "An app is
//! something the child could launch from a menu" — a `Type=Application` entry
//! without `NoDisplay`/`Hidden`.


use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use crate::app_scan::ProcInfo;

#[derive(Debug, Clone)]
pub struct DesktopEntry {
    /// Desktop file id: path below `applications/` with `/` → `-`, no `.desktop`.
    pub id: String,
    pub name: String,
    /// `Name[xx]=` / `Name[xx_YY]=` keyed by the locale in brackets.
    pub localized_names: HashMap<String, String>,
    pub icon: Option<String>,
    pub visible: bool,
    /// Basenames of `Exec=` (first word after `env VAR=…`) and `TryExec=`.
    pub binaries: Vec<String>,
    /// `Exec=` has no arguments besides field codes.
    pub plain: bool,
    /// `StartupWMClass`, lower-cased.
    pub wmclass: Option<String>,
    /// Steam game shortcut: `Exec=steam steam://rungameid/<id>`.
    pub steam_appid: Option<String>,
    /// freedesktop `Categories=` (e.g. `Game`, `WebBrowser`), main and additional.
    pub categories: Vec<String>,
}

#[derive(Default)]
pub struct Catalog {
    entries: HashMap<String, DesktopEntry>,
    by_lower: HashMap<String, String>,
    by_bin: HashMap<String, Vec<String>>,
    by_wm: HashMap<String, Vec<String>>,
    by_short: HashMap<String, Vec<String>>,
    by_steam: HashMap<String, String>,
    /// Lower-cased ids and `Exec` basenames of XDG autostart entries.
    autostart: HashSet<String>,
}

/// Executables that run a script/archive named in argv; identified by argv instead.
fn is_interpreter(name: &str) -> bool {
    let base = name.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
    matches!(
        base,
        "electron" | "python" | "node" | "java" | "bash" | "sh" | "dash" | "zsh" | "perl" | "ruby" | "mono" | "dotnet"
    ) || name.starts_with("wine")
        || name.starts_with("qemu")
}

impl Catalog {
    /// Application and autostart dirs for a user, in XDG precedence order.
    pub fn dirs_for_home(home: &Path) -> (Vec<PathBuf>, Vec<PathBuf>) {
        let apps = [
            home.join(".local/share/applications"),
            home.join(".local/share/flatpak/exports/share/applications"),
            PathBuf::from("/usr/local/share/applications"),
            PathBuf::from("/usr/share/applications"),
            PathBuf::from("/var/lib/flatpak/exports/share/applications"),
            PathBuf::from("/var/lib/snapd/desktop/applications"),
        ];
        let autostart = [home.join(".config/autostart"), PathBuf::from("/etc/xdg/autostart")];
        (apps.to_vec(), autostart.to_vec())
    }

    pub fn load(app_dirs: &[PathBuf], autostart_dirs: &[PathBuf]) -> Self {
        let mut cat = Catalog::default();
        for base in app_dirs {
            let mut files = Vec::new();
            walk(base, &mut files);
            for path in files {
                let Some(id) = desktop_id(base, &path) else { continue };
                if cat.entries.contains_key(&id) {
                    continue; // an earlier dir wins
                }
                if let Some(e) = fs::read_to_string(&path).ok().and_then(|t| parse_entry(&t, &id)) {
                    cat.insert(e);
                }
            }
        }
        for dir in autostart_dirs {
            let Ok(rd) = fs::read_dir(dir) else { continue };
            for f in rd.flatten() {
                let name = f.file_name().to_string_lossy().into_owned();
                let Some(id) = name.strip_suffix(".desktop") else { continue };
                let Ok(text) = fs::read_to_string(f.path()) else { continue };
                let kv = desktop_group(&text);
                if kv.get("Hidden").map(String::as_str) == Some("true") {
                    continue;
                }
                cat.autostart.insert(id.to_lowercase());
                if let Some((bin, _)) = kv.get("Exec").and_then(|e| exec_binary(e)) {
                    cat.autostart.insert(bin);
                }
            }
        }
        cat
    }

    fn insert(&mut self, e: DesktopEntry) {
        let id = e.id.clone();
        self.by_lower.entry(id.to_lowercase()).or_insert_with(|| id.clone());
        for b in &e.binaries {
            self.by_bin.entry(b.clone()).or_default().push(id.clone());
        }
        if let Some(wm) = &e.wmclass {
            self.by_wm.entry(wm.clone()).or_default().push(id.clone());
        }
        self.by_short.entry(short(&id)).or_default().push(id.clone());
        if let Some(appid) = &e.steam_appid {
            self.by_steam.entry(appid.clone()).or_insert_with(|| id.clone());
        }
        self.entries.insert(id, e);
    }

    /// Exact desktop id, then case-insensitive.
    pub fn get(&self, id: &str) -> Option<&DesktopEntry> {
        self.entries.get(id).or_else(|| self.entries.get(self.by_lower.get(&id.to_lowercase())?))
    }

    pub fn steam_shortcut(&self, appid: &str) -> Option<&DesktopEntry> {
        self.entries.get(self.by_steam.get(appid)?)
    }

    pub fn is_autostart(&self, id_or_binary: &str) -> bool {
        self.autostart.contains(&id_or_binary.to_lowercase()) || self.autostart.contains(id_or_binary)
    }

    /// Visible first, then id equal to `key`, then `Exec` without extra args.
    fn best(&self, ids: Option<&Vec<String>>, key: &str) -> Option<&DesktopEntry> {
        let key = key.to_lowercase();
        ids?.iter()
            .filter_map(|id| self.entries.get(id))
            .min_by_key(|e| (!e.visible, short(&e.id) != key, !e.plain, e.id.clone()))
    }

    /// Unit application id → entry: `<id>.desktop`, then `StartupWMClass`.
    pub fn by_unit_id(&self, app: &str) -> Option<&DesktopEntry> {
        self.get(app).or_else(|| self.best(self.by_wm.get(&app.to_lowercase()), app))
    }

    /// Executable / script name → entry: `Exec`/`TryExec`, `StartupWMClass`, id.
    pub fn by_name(&self, name: &str) -> Option<&DesktopEntry> {
        if name.is_empty() {
            return None;
        }
        let lower = name.to_lowercase();
        self.best(self.by_bin.get(name), name)
            .or_else(|| self.best(self.by_wm.get(&lower), name))
            .or_else(|| self.best(self.by_short.get(&lower), name))
    }

    pub fn by_process(&self, p: &ProcInfo) -> Option<&DesktopEntry> {
        let base = |s: &str| s.rsplit('/').next().unwrap_or(s).to_string();
        let mut names: Vec<String> = Vec::new();
        if let Some(exe) = &p.exe {
            names.push(base(exe));
        }
        if let Some(a0) = p.argv.first() {
            names.push(base(a0));
        }
        names.push(p.comm.clone());
        if names.iter().any(|n| is_interpreter(n)) {
            // electron39 /usr/lib/bitwarden/app.asar -> "bitwarden"
            if let Some(script) = p.argv.iter().skip(1).find(|a| !a.starts_with('-')) {
                let parts: Vec<&str> = script.split('/').filter(|s| !s.is_empty()).collect();
                if let Some((last, dirs)) = parts.split_last() {
                    let stem = [".py", ".js", ".jar", ".asar", ".sh", ".pl", ".rb"]
                        .iter()
                        .find_map(|ext| last.strip_suffix(ext))
                        .unwrap_or(last);
                    let mut from_script = vec![stem.to_string()];
                    from_script.extend(dirs.iter().rev().map(|d| d.to_string()));
                    from_script.append(&mut names);
                    names = from_script;
                }
            }
        }
        names.iter().filter(|n| !is_interpreter(n)).find_map(|n| self.by_name(n))
    }
}

impl DesktopEntry {
    /// Name for a profile language like `pl` or `pt_BR`: exact match, then the
    /// language part, then any `xx_YY` of that language, then `Name=`.
    pub fn name_for(&self, lang: &str) -> &str {
        let base = lang.split(['_', '-', '.', '@']).next().unwrap_or(lang);
        self.localized_names
            .get(lang)
            .or_else(|| self.localized_names.get(base))
            .or_else(|| {
                let prefix = format!("{base}_");
                let mut regional: Vec<_> = self.localized_names.iter().filter(|(k, _)| k.starts_with(&prefix)).collect();
                regional.sort();
                regional.first().map(|(_, v)| *v)
            })
            .unwrap_or(&self.name)
    }
}

/// Catalogues per home directory, reloaded when an applications or autostart
/// dir (or a subdir) changes mtime — package managers install by rename, which
/// updates the dir. Checked at most every `CHECK_EVERY`.
pub struct CatalogCache {
    homes: HashMap<PathBuf, CachedCatalog>,
}

struct CachedCatalog {
    catalog: Catalog,
    signature: Vec<(PathBuf, Option<SystemTime>)>,
    checked: Instant,
}

const CHECK_EVERY: Duration = Duration::from_secs(30);

impl CatalogCache {
    pub fn new() -> Self {
        Self { homes: HashMap::new() }
    }

    pub fn get(&mut self, home: &Path) -> &Catalog {
        let (apps, auto) = Catalog::dirs_for_home(home);
        self.get_with_dirs(home, &apps, &auto)
    }

    fn get_with_dirs(&mut self, home: &Path, apps: &[PathBuf], auto: &[PathBuf]) -> &Catalog {
        let stale = match self.homes.get(home) {
            None => true,
            Some(c) if c.checked.elapsed() < CHECK_EVERY => false,
            Some(c) => signature(apps, auto) != c.signature,
        };
        if stale {
            let signature = signature(apps, auto);
            let catalog = Catalog::load(apps, auto);
            self.homes.insert(home.to_path_buf(), CachedCatalog { catalog, signature, checked: Instant::now() });
        }
        let c = self.homes.get_mut(home).expect("inserted above");
        if !stale && c.checked.elapsed() >= CHECK_EVERY {
            c.checked = Instant::now();
        }
        &c.catalog
    }
}

/// mtimes of the dirs and their subdirs (applications dirs nest, e.g. `wine/Programs`).
fn signature(apps: &[PathBuf], auto: &[PathBuf]) -> Vec<(PathBuf, Option<SystemTime>)> {
    fn add(dir: &Path, out: &mut Vec<(PathBuf, Option<SystemTime>)>, depth: usize) {
        out.push((dir.to_path_buf(), fs::metadata(dir).and_then(|m| m.modified()).ok()));
        if depth == 0 {
            return;
        }
        if let Ok(rd) = fs::read_dir(dir) {
            let mut subs: Vec<PathBuf> =
                rd.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_dir())).map(|e| e.path()).collect();
            subs.sort();
            for d in subs {
                add(&d, out, depth - 1);
            }
        }
    }
    let mut out = Vec::new();
    for d in apps {
        add(d, &mut out, 4);
    }
    for d in auto {
        add(d, &mut out, 0);
    }
    out
}

/// Last dotted component of a desktop id, lower-cased (`org.kde.dolphin` → `dolphin`).
fn short(id: &str) -> String {
    id.rsplit('.').next().unwrap_or(id).to_lowercase()
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let path = e.path();
        match e.file_type() {
            Ok(t) if t.is_dir() => walk(&path, out),
            Ok(_) if path.extension().is_some_and(|x| x == "desktop") => out.push(path),
            _ => {}
        }
    }
}

fn desktop_id(base: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(base).ok()?.to_str()?;
    Some(rel.strip_suffix(".desktop")?.replace('/', "-"))
}

/// Key/values of the `[Desktop Entry]` group only (not `[Desktop Action …]`).
fn desktop_group(text: &str) -> HashMap<String, String> {
    let mut kv = HashMap::new();
    let mut in_group = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_group = line == "[Desktop Entry]";
        } else if in_group
            && !line.starts_with('#')
            && let Some((k, v)) = line.split_once('=')
        {
            kv.entry(k.trim().to_string()).or_insert_with(|| v.trim().to_string());
        }
    }
    kv
}

fn parse_entry(text: &str, id: &str) -> Option<DesktopEntry> {
    let kv = desktop_group(text);
    if kv.get("Type").map(String::as_str) != Some("Application") {
        return None;
    }
    let flag = |k: &str| kv.get(k).map(String::as_str) == Some("true");
    let localized_names = kv
        .iter()
        .filter_map(|(k, v)| Some((k.strip_prefix("Name[")?.strip_suffix(']')?.to_string(), v.clone())))
        .collect();
    let mut binaries = Vec::new();
    let mut plain = false;
    if let Some((bin, extra)) = kv.get("Exec").and_then(|e| exec_binary(e)) {
        binaries.push(bin);
        plain = !extra;
    }
    if let Some(t) = kv.get("TryExec") {
        binaries.push(t.rsplit('/').next().unwrap_or(t).to_string());
    }
    Some(DesktopEntry {
        id: id.to_string(),
        name: kv.get("Name").cloned().unwrap_or_else(|| id.to_string()),
        localized_names,
        icon: kv.get("Icon").cloned().filter(|s| !s.is_empty()),
        visible: !flag("NoDisplay") && !flag("Hidden"),
        binaries,
        plain,
        wmclass: kv.get("StartupWMClass").map(|s| s.to_lowercase()),
        categories: kv
            .get("Categories")
            .map(|c| c.split(';').map(str::trim).filter(|c| !c.is_empty()).map(str::to_string).collect())
            .unwrap_or_default(),
        steam_appid: kv.get("Exec").and_then(|e| {
            let id: String = e.split("steam://rungameid/").nth(1)?.chars().take_while(char::is_ascii_digit).collect();
            (!id.is_empty()).then_some(id)
        }),
    })
}

/// `Exec=` value → (binary basename, has extra args). Drops field codes and a
/// leading `env VAR=…`.
fn exec_binary(cmd: &str) -> Option<(String, bool)> {
    let mut argv: Vec<String> = split_exec(cmd)
        .into_iter()
        .filter(|a| !(a.len() == 2 && a.starts_with('%')))
        .collect();
    while let Some(first) = argv.first() {
        let base = first.rsplit('/').next().unwrap_or(first);
        let is_assignment = first.split_once('=').is_some_and(|(k, _)| {
            !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        });
        if base == "env" || is_assignment {
            argv.remove(0);
        } else {
            break;
        }
    }
    let first = argv.first()?;
    Some((first.rsplit('/').next().unwrap_or(first).to_string(), argv.len() > 1))
}

/// Splits an `Exec=` value: whitespace-separated, `"…"` quoting with `\` escapes.
fn split_exec(cmd: &str) -> Vec<String> {
    let (mut out, mut cur, mut quoted, mut in_word) = (Vec::new(), String::new(), false, false);
    let mut chars = cmd.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                in_word = true;
            }
            '\\' if quoted => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            c if c.is_whitespace() && !quoted => {
                if in_word {
                    out.push(std::mem::take(&mut cur));
                    in_word = false;
                }
            }
            c => {
                cur.push(c);
                in_word = true;
            }
        }
    }
    if in_word {
        out.push(cur);
    }
    out
}

#[cfg(test)]
pub(crate) mod fixture {
    //! `.desktop` fixtures built from the formats real distros ship.
    use std::fs;
    use std::path::Path;

    pub fn write(dir: &Path, rel: &str, body: &str) {
        let path = dir.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, format!("[Desktop Entry]\nType=Application\n{body}\n")).unwrap();
    }

    /// A catalogue covering the identification test cases.
    pub fn standard(apps: &Path, autostart: &Path) {
        for (rel, body) in [
            ("org.gnome.Nautilus.desktop", "Name=Files\nExec=nautilus --new-window %U\nIcon=org.gnome.Nautilus\nDBusActivatable=true"),
            ("org.gnome.TextEditor.desktop", "Name=Text Editor\nExec=gnome-text-editor %U\nIcon=org.gnome.TextEditor"),
            ("org.kde.dolphin.desktop", "Name=Dolphin\nName[de]=Dolphin-Dateiverwaltung\nName[pt_BR]=Gerenciador Dolphin\nName[pt_PT]=Gestor Dolphin\nExec=dolphin %u\nIcon=system-file-manager"),
            ("org.kde.konsole.desktop", "Name=Konsole\nExec=konsole\nTryExec=konsole\nIcon=utilities-terminal\n\n[Desktop Action NewTab]\nExec=konsole --new-tab"),
            ("org.kde.kded6.desktop", "Name=KDED\nExec=kded6\nNoDisplay=true"),
            ("org.kde.plasmashell.desktop", "Name=Plasma\nExec=plasmashell\nNoDisplay=true"),
            ("firefox.desktop", "Name=Firefox\nExec=/usr/lib/firefox/firefox %u\nIcon=firefox\nStartupWMClass=firefox\nCategories=Network;WebBrowser;"),
            ("chromium.desktop", "Name=Chromium\nExec=/usr/bin/chromium %U\nIcon=chromium\nStartupWMClass=chromium"),
            ("bitwarden.desktop", "Name=Bitwarden\nExec=bitwarden-desktop %u\nIcon=bitwarden\nStartupWMClass=Bitwarden"),
            ("foot.desktop", "Name=Foot\nExec=foot\nIcon=foot"),
            ("foot-server.desktop", "Name=Foot Server\nExec=foot --server\nIcon=foot"),
            ("xfce4-terminal.desktop", "Name=Xfce Terminal\nExec=xfce4-terminal\nIcon=org.xfce.terminal"),
            ("vim.desktop", "Name=Vim\nExec=vim %F\nTerminal=true\nIcon=gvim"),
            ("mousepad.desktop", "Name=Mousepad\nExec=env GDK_BACKEND=x11 mousepad %F\nIcon=org.xfce.mousepad"),
            ("xfce4-panel.desktop", "Name=Panel\nExec=xfce4-panel\nNoDisplay=true"),
            ("steam.desktop", "Name=Steam\nExec=/usr/bin/steam %U\nIcon=steam"),
            ("nextcloud.desktop", "Name=Nextcloud\nExec=nextcloud\nIcon=Nextcloud"),
            ("org.mozilla.firefox.desktop", "Name=Firefox (Flatpak)\nExec=/usr/bin/flatpak run --branch=stable --command=firefox org.mozilla.firefox @@u %u @@\nIcon=org.mozilla.firefox"),
            ("wine/Programs/Game Name.desktop", "Name=Game Name\nExec=env WINEPREFIX=\"/home/kid/.wine\" wine C:\\\\Games\\\\game.exe"),
            ("Dota 2.desktop", "Name=Dota 2\nExec=steam steam://rungameid/570\nIcon=steam_icon_570\nTerminal=false\nCategories=Game;"),
            ("org.supertuxproject.SuperTux.desktop", "Name=SuperTux\nExec=supertux2\nIcon=supertux\nCategories=Game;ArcadeGame;"),
            ("spotify_spotify.desktop", "Name=Spotify\nExec=env BAMF_DESKTOP_FILE_HINT=/var/lib/snapd/desktop/applications/spotify_spotify.desktop /snap/bin/spotify %U\nIcon=/snap/spotify/80/usr/share/spotify/icons/spotify-linux-128.png"),
            ("brave.desktop", "Name=Brave\nExec=brave\nIcon=brave\nHidden=true"),
        ] {
            write(apps, rel, body);
        }
        write(autostart, "nextcloud.desktop", "Name=Nextcloud\nExec=nextcloud --background");
        write(autostart, "disabled.desktop", "Name=Disabled\nExec=disabled-app\nHidden=true");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let d = std::env::temp_dir()
            .join(format!("sg-catalog-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn exec_parsing() {
        assert_eq!(exec_binary("nautilus --new-window %U"), Some(("nautilus".into(), true)));
        assert_eq!(exec_binary("/usr/bin/chromium %U"), Some(("chromium".into(), false)));
        assert_eq!(exec_binary("env GDK_BACKEND=x11 mousepad %F"), Some(("mousepad".into(), false)));
        assert_eq!(exec_binary("\"/opt/My App/run\" --x"), Some(("run".into(), true)));
        assert_eq!(exec_binary("%u"), None);
    }

    #[test]
    fn loads_and_resolves() {
        let d = tmp();
        let (apps, auto) = (d.join("apps"), d.join("autostart"));
        fixture::standard(&apps, &auto);
        let cat = Catalog::load(std::slice::from_ref(&apps), &[auto]);

        assert_eq!(cat.get("org.kde.dolphin").unwrap().name, "Dolphin");
        assert_eq!(cat.get("org.kde.dolphin").unwrap().icon.as_deref(), Some("system-file-manager"));
        assert_eq!(cat.get("org.kde.kded6").unwrap().icon, None);
        assert_eq!(cat.get("firefox").unwrap().categories, vec!["Network", "WebBrowser"]);
        assert!(cat.get("org.kde.dolphin").unwrap().categories.is_empty());
        assert_eq!(cat.get("ORG.KDE.DOLPHIN").unwrap().id, "org.kde.dolphin");
        assert_eq!(cat.get("wine-Programs-Game Name").unwrap().name, "Game Name");
        // [Desktop Action] Exec doesn't count
        assert_eq!(cat.get("org.kde.konsole").unwrap().binaries, vec!["konsole", "konsole"]);
        assert!(cat.get("org.kde.konsole").unwrap().plain);
        assert!(!cat.get("org.kde.kded6").unwrap().visible);
        assert!(!cat.get("brave").unwrap().visible);
        // foot vs foot-server: plain Exec and matching id wins
        assert_eq!(cat.by_name("foot").unwrap().id, "foot");
        assert_eq!(cat.by_name("bitwarden").unwrap().id, "bitwarden"); // StartupWMClass
        assert_eq!(cat.by_name("dolphin").unwrap().id, "org.kde.dolphin");
        assert_eq!(cat.by_unit_id("org.chromium.Chromium").map(|e| e.id.as_str()), None);
        assert!(cat.is_autostart("nextcloud"));
        assert!(!cat.is_autostart("disabled-app"));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn localized_names() {
        let d = tmp();
        fixture::standard(&d.join("apps"), &d.join("autostart"));
        let cat = Catalog::load(&[d.join("apps")], &[]);
        let e = cat.get("org.kde.dolphin").unwrap();
        assert_eq!(e.name_for("en"), "Dolphin");
        assert_eq!(e.name_for("de"), "Dolphin-Dateiverwaltung");
        assert_eq!(e.name_for("de_AT.UTF-8"), "Dolphin-Dateiverwaltung");
        assert_eq!(e.name_for("pt_PT"), "Gestor Dolphin");
        assert_eq!(e.name_for("pt"), "Gerenciador Dolphin"); // first regional variant
        assert_eq!(e.name_for("pl"), "Dolphin");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn cache_reloads_when_a_dir_changes() {
        let d = tmp();
        let (apps, home) = (vec![d.join("apps")], d.join("home"));
        fixture::write(&apps[0], "foot.desktop", "Name=Foot\nExec=foot");
        let mut cache = CatalogCache::new();
        assert!(cache.get_with_dirs(&home, &apps, &[]).get("kcalc").is_none());

        fixture::write(&apps[0], "sub/kcalc.desktop", "Name=KCalc\nExec=kcalc");
        // within CHECK_EVERY nothing is re-checked
        assert!(cache.get_with_dirs(&home, &apps, &[]).get("sub-kcalc").is_none());
        // once due, the changed signature triggers a reload
        cache.homes.get_mut(&home).unwrap().checked -= CHECK_EVERY;
        assert_eq!(cache.get_with_dirs(&home, &apps, &[]).get("sub-kcalc").unwrap().name, "KCalc");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn user_dirs_come_first() {
        let (apps, auto) = Catalog::dirs_for_home(Path::new("/home/kid"));
        assert_eq!(apps[0], Path::new("/home/kid/.local/share/applications"));
        assert!(apps.contains(&PathBuf::from("/var/lib/flatpak/exports/share/applications")));
        assert_eq!(auto[0], Path::new("/home/kid/.config/autostart"));
    }

    #[test]
    fn earlier_dir_wins() {
        let d = tmp();
        fixture::write(&d.join("home"), "foot.desktop", "Name=My Foot\nExec=foot");
        fixture::write(&d.join("sys"), "foot.desktop", "Name=Foot\nExec=foot");
        let cat = Catalog::load(&[d.join("home"), d.join("sys")], &[]);
        assert_eq!(cat.get("foot").unwrap().name, "My Foot");
        let _ = fs::remove_dir_all(&d);
    }
}
