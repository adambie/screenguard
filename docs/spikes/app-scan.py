#!/usr/bin/env python3
"""Phase 0 spike for docs/feature-app-usage-limits.md — READ-ONLY.

Implements the identification algorithm and the "is it an app" filter from the
plan, so they can be sanity-checked on real machines before the Rust version:

  0. special cases on the process or an ancestor (Steam reaper, Minecraft, Wine)
  1. Flatpak / Snap unit
  2. app unit (app-... under app.slice) -> .desktop
     (Chromium/Electron self-scopes are resolved via the children's unit)
  3. GIO_LAUNCHED_DESKTOP_FILE(+_PID) from /proc/<pid>/environ
  4. process tree: launch root -> exe / argv -> .desktop
  5. no visible .desktop -> "other" bucket (not an app)

Nothing is modified. Command lines are NOT printed unless --show-cmdline is given
(they can contain URLs, file names, tokens). Without root, exe/environ are only
readable for your own processes (and not for non-dumpable ones like Electron).

Usage:  app-scan.py [UID] [--show-cmdline] [--all] [--time]
        --all   also list processes that are not apps
        --time  only time N full scans (step 0.6) and print the cost
"""
import os
import re
import shlex
import sys
import time

CLK_TCK = os.sysconf("SC_CLK_TCK")
AUTOSTART_WINDOW_S = 60  # root started this soon after the session -> autostart
# Chromium and every Electron app move their main process into a scope with this
# id, whatever the real app is. The id says nothing about the app.
SELF_SCOPE_IDS = {"org.chromium.Chromium"}
# Executables that run a script/archive named in argv: identify by argv instead.
INTERPRETERS = re.compile(r"^(electron\d*|python[\d.]*|node|java|bash|sh|dash|zsh|perl|ruby|mono|dotnet|wine.*|qemu.*)$")
SCOPE_RE = re.compile(r"^app-(?P<rest>.+)-(?P<rand>[^-]+)\.scope$")
SERVICE_RE = re.compile(r"^app-(?P<rest>.+?)(?:@(?P<inst>[^.]*))?\.service$")


def read(path, binary=False):
    try:
        with open(path, "rb" if binary else "r") as f:
            return f.read()
    except OSError:
        return None


def unescape(s):
    """systemd unit-name escaping: \\x2d -> '-' etc."""
    return re.sub(r"\\x([0-9a-fA-F]{2})", lambda m: chr(int(m.group(1), 16)), s)


# --------------------------------------------------------------------- processes

class Proc:
    __slots__ = ("pid", "ppid", "uid", "start", "comm", "exe", "argv", "cgroup",
                 "unit", "kind", "gio_file", "gio_pid", "children")


def scan_procs(want_env=True):
    procs = {}
    for name in os.listdir("/proc"):
        if not name.isdigit():
            continue
        pid = int(name)
        stat = read(f"/proc/{pid}/stat")
        status = read(f"/proc/{pid}/status")
        if not stat or not status:
            continue
        p = Proc()
        p.pid = pid
        rest = stat[stat.rindex(")") + 2:].split()
        p.ppid = int(rest[1])
        p.start = int(rest[19]) / CLK_TCK
        m = re.search(r"^Uid:\s+(\d+)", status, re.M)
        p.uid = int(m.group(1)) if m else -1
        p.comm = stat[stat.index("(") + 1:stat.rindex(")")]
        raw = read(f"/proc/{pid}/cmdline", binary=True) or b""
        p.argv = [a.decode(errors="replace") for a in raw.split(b"\0") if a]
        try:
            exe = os.readlink(f"/proc/{pid}/exe")
            p.exe = exe[:-10] if exe.endswith(" (deleted)") else exe
        except OSError:
            p.exe = None
        cg = read(f"/proc/{pid}/cgroup") or ""
        p.cgroup = next((l[3:] for l in cg.splitlines() if l.startswith("0::")), "")
        p.kind, p.unit = classify_cgroup(p.cgroup)
        p.gio_file = p.gio_pid = None
        if want_env:
            env = read(f"/proc/{pid}/environ", binary=True)
            if env:
                for kv in env.split(b"\0"):
                    if kv.startswith(b"GIO_LAUNCHED_DESKTOP_FILE="):
                        p.gio_file = kv.split(b"=", 1)[1].decode(errors="replace")
                    elif kv.startswith(b"GIO_LAUNCHED_DESKTOP_FILE_PID="):
                        try:
                            p.gio_pid = int(kv.split(b"=", 1)[1])
                        except ValueError:
                            pass
        p.children = []
        procs[pid] = p
    for p in procs.values():
        if p.ppid in procs:
            procs[p.ppid].children.append(p)
    return procs


def classify_cgroup(path):
    """-> (kind, unit). kind: app / autostart / user-service / session / background
    / login-session / other."""
    parts = [x for x in path.split("/") if x]
    unit = parts[-1] if parts else ""
    if unit.startswith("snap."):
        return "app", unit
    if "app.slice" in parts:
        idx = parts.index("app.slice")
        # only slices nest; anything below the first unit is a sub-cgroup the unit
        # made itself (Konsole: app-org.kde.konsole-<pid>.scope/main.scope, tabs)
        units = [x for x in parts[idx + 1:] if x.endswith((".scope", ".service"))]
        unit = units[0] if units else unit
        if not unit.startswith("app-"):
            return "user-service", unit
        if unit.endswith("@autostart.service"):
            return "autostart", unit
        return "app", unit
    if "session.slice" in parts:
        return "session", unit
    if "background.slice" in parts:
        return "background", unit
    if re.match(r"session-\w+\.scope$", unit):
        return "login-session", unit
    return "other", unit


def parse_unit(unit):
    """-> (launcher, application_id) or (None, None)."""
    if unit.startswith("snap."):
        return "snap", unit.split(".")[1]
    m = SCOPE_RE.match(unit) or SERVICE_RE.match(unit)
    if not m:
        return None, None
    fields = m.group("rest").split("-")  # real '-' inside ids are escaped as \x2d
    if len(fields) >= 2:
        return unescape(fields[0]), unescape("-".join(fields[1:]))
    return None, unescape(fields[0])


# ----------------------------------------------------------------- .desktop files

class Entry:
    __slots__ = ("id", "path", "name", "visible", "binaries", "plain", "wmclass",
                 "flatpak")


def exec_binary(cmd):
    """Exec= value -> (binary basename, has extra args, flatpak id)."""
    try:
        argv = shlex.split(cmd)
    except ValueError:
        argv = cmd.split()
    argv = [a for a in argv if not re.fullmatch(r"%[fFuUdDnNickvm]", a)]
    while argv and (os.path.basename(argv[0]) == "env" or re.match(r"^\w+=", argv[0])):
        argv = argv[1:]
    if not argv:
        return None, False, None
    binary = os.path.basename(argv[0])
    flatpak = None
    if binary == "flatpak" and "run" in argv:
        ids = [a for a in argv[argv.index("run") + 1:] if not a.startswith("-")]
        flatpak = ids[0] if ids else None
    return binary, len(argv) > 1, flatpak


def load_entries(home):
    data_home = os.environ.get("XDG_DATA_HOME") or f"{home}/.local/share"
    data_dirs = (os.environ.get("XDG_DATA_DIRS") or "/usr/local/share:/usr/share").split(":")
    roots = [data_home, f"{data_home}/flatpak/exports/share"] + data_dirs + [
        "/var/lib/flatpak/exports/share", "/var/lib/snapd/desktop"]
    entries = {}
    for root in roots:
        base = os.path.join(root, "applications")
        for dirpath, _, files in os.walk(base):
            for f in files:
                if not f.endswith(".desktop"):
                    continue
                path = os.path.join(dirpath, f)
                did = os.path.relpath(path, base)[:-8].replace("/", "-")
                if did in entries:
                    continue  # earlier XDG dir wins
                e = parse_entry(path, did)
                if e:
                    entries[did] = e
    return entries


def parse_entry(path, did):
    text = read(path) or ""
    # only the [Desktop Entry] group, not [Desktop Action ...]
    m = re.search(r"^\[Desktop Entry\]\s*$(.*?)(?=^\[|\Z)", text, re.M | re.S)
    if not m:
        return None
    kv = {}
    for line in m.group(1).splitlines():
        if "=" in line and not line.startswith("#"):
            k, v = line.split("=", 1)
            kv.setdefault(k.strip(), v.strip())
    if kv.get("Type") != "Application":
        return None
    e = Entry()
    e.id, e.path, e.name = did, path, kv.get("Name", did)
    e.visible = kv.get("NoDisplay") != "true" and kv.get("Hidden") != "true"
    e.binaries, e.plain, e.flatpak = set(), False, None
    if "Exec" in kv:
        b, extra, e.flatpak = exec_binary(kv["Exec"])
        if b:
            e.binaries.add(b)
            e.plain = not extra
    if "TryExec" in kv:
        e.binaries.add(os.path.basename(kv["TryExec"]))
    e.wmclass = kv.get("StartupWMClass", "").lower() or None
    return e


class Catalog:
    def __init__(self, entries, autostart):
        self.entries = entries
        self.by_lower = {k.lower(): e for k, e in entries.items()}
        self.by_bin, self.by_wm, self.by_short = {}, {}, {}
        for e in entries.values():
            for b in e.binaries:
                self.by_bin.setdefault(b, []).append(e)
            if e.wmclass:
                self.by_wm.setdefault(e.wmclass, []).append(e)
            self.by_short.setdefault(e.id.lower().rsplit(".", 1)[-1], []).append(e)
        self.autostart = autostart  # set of desktop ids and binaries

    @staticmethod
    def best(cands, key):
        """visible first, then id == key, then Exec without extra args."""
        if not cands:
            return None
        return sorted(cands, key=lambda e: (not e.visible,
                                            e.id.lower().rsplit(".", 1)[-1] != key.lower(),
                                            not e.plain, e.id))[0]

    def by_id(self, app_id):
        if not app_id:
            return None
        return (self.by_lower.get(app_id.lower())
                or self.best(self.by_wm.get(app_id.lower()), app_id))

    def by_name(self, name):
        """exe / script name -> entry via Exec/TryExec, StartupWMClass, id."""
        if not name:
            return None
        return (self.best(self.by_bin.get(name), name)
                or self.best(self.by_wm.get(name.lower()), name)
                or self.best(self.by_short.get(name.lower()), name))

    def by_process(self, p):
        names = []
        if p.exe:
            names.append(os.path.basename(p.exe))
        if p.argv:
            names.append(os.path.basename(p.argv[0]))
        names.append(p.comm)
        if any(INTERPRETERS.match(n) for n in names if n) and len(p.argv) > 1:
            # interpreter: identify by the script/archive, e.g.
            # electron39 /usr/lib/bitwarden/app.asar -> "bitwarden"
            script = next((a for a in p.argv[1:] if not a.startswith("-")), None)
            if script:
                parts = [x for x in script.split("/") if x]
                names = [re.sub(r"\.(py|js|jar|asar|sh|pl|rb)$", "", parts[-1])] + parts[-2::-1] + names
        for n in names:
            if n and not INTERPRETERS.match(n):
                e = self.by_name(n)
                if e:
                    return e
        return None


def load_autostart(home):
    ids = set()
    for d in [f"{home}/.config/autostart", "/etc/xdg/autostart"]:
        try:
            files = os.listdir(d)
        except OSError:
            continue
        for f in files:
            if f.endswith(".desktop"):
                text = read(os.path.join(d, f)) or ""
                if re.search(r"^Hidden=true", text, re.M):
                    continue
                ids.add(f[:-8].lower())
                m = re.search(r"^Exec=(.+)$", text, re.M)
                if m:
                    b, _, _ = exec_binary(m.group(1))
                    if b:
                        ids.add(b)
    return ids


# -------------------------------------------------------------------- algorithm

def special_case(p, procs):
    """Checked on the process and its ancestors (a game inherits Steam's unit/env)."""
    q, depth = p, 0
    while q and depth < 64:
        joined = " ".join(q.argv)
        if q.argv and os.path.basename(q.argv[0]) == "reaper" and "SteamLaunch" in q.argv:
            m = re.search(r"(?:^| )AppId=(\d+)(?: |$)", joined)
            if m:
                return f"steam:{m.group(1)}"
        if q.argv and os.path.basename(q.argv[0]).startswith("java") and \
                re.search(r"net\.minecraft|minecraft.*\.jar|cpw\.mods|fabricmc|quiltmc", joined, re.I):
            return "minecraft:java"
        for a in q.argv[:2]:
            if a.lower().endswith(".exe"):
                return "wine:" + os.path.basename(a.replace("\\", "/")).lower()
        q, depth = procs.get(q.ppid), depth + 1
    return None


def is_session_root(p, procs, uid):
    """True if p's parent is no longer part of the child's app: other user, PID 1,
    the user's systemd, or the login session leader."""
    parent = procs.get(p.ppid)
    if parent is None or parent.pid == 1 or parent.uid != uid:
        return True
    if parent.comm == "systemd":
        return True
    return False


def launch_root(p, procs, uid, cat):
    """Highest ancestor that resolves to a visible .desktop app; the walk stops at
    the session boundary. A terminal or file manager owns what it starts (Q6)."""
    best, q, depth = None, p, 0
    while q and depth < 64:
        e = cat.by_process(q)
        if e and e.visible:
            best = (q, e)
        if is_session_root(q, procs, uid):
            break
        q, depth = procs.get(q.ppid), depth + 1
    return best


def unit_entry(unit, p, procs, cat):
    """App unit -> entry. Falls back to the oldest process of the unit."""
    launcher, app = parse_unit(unit)
    if launcher == "snap":
        return f"snap:{app}", None, "snap unit"
    if launcher == "flatpak":
        return f"flatpak:{app}", cat.by_lower.get(app.lower()), "flatpak unit"
    e = cat.by_id(app)
    if e:
        return f"desktop:{e.id}", e, f"unit id ({launcher or '-'})"
    # uwsm/xdg-terminal-exec units carry a command, not an id: use the oldest
    # process in the unit, which is what the unit was started for.
    members = [q for q in procs.values() if q.unit == unit]
    for q in sorted(members, key=lambda q: q.start):
        e = cat.by_process(q)
        if e:
            return f"desktop:{e.id}", e, f"unit ({launcher or '-'}) + exe"
    return None, None, None


def identify(p, procs, uid, cat, t_session):
    """-> (app_id, entry, kind, how). kind: app / autostart / other."""
    special = special_case(p, procs)
    if special:
        return special, None, "app", "special case"

    unit, kind = p.unit, p.kind
    if kind in ("user-service", "background"):
        # real systemd services (portals, syncthing, input methods ...). Note:
        # session.slice is NOT excluded — under uwsm, apps started by a compositor
        # keybinding without `uwsm app` stay in the compositor's unit there.
        name = os.path.basename(p.exe) if p.exe else p.comm
        return f"exe:{name}", None, "other", "service unit"
    _, uid_app = parse_unit(unit) if kind in ("app", "autostart") else (None, None)
    if kind == "app" and uid_app in SELF_SCOPE_IDS:
        # moved itself out of its launcher's unit; the children stayed there
        child_units = [c for c in p.children if c.unit != unit and c.kind in ("app", "autostart")]
        if child_units:
            unit, kind = child_units[0].unit, child_units[0].kind
        else:
            unit = None

    app_id = e = None
    how = ""
    launcher, _ = parse_unit(unit) if unit and kind in ("app", "autostart") else (None, None)
    if launcher in ("snap", "flatpak"):
        app_id, e, how = unit_entry(unit, p, procs, cat)
        return app_id, e, "autostart" if kind == "autostart" else "app", how

    # GIO env before the unit: a GIO launch from inside another app (file manager
    # opening a PDF) inherits that app's unit, but the env names the new app.
    if p.gio_file and p.gio_pid:
        q, depth = p, 0
        while q and depth < 64 and q.pid != p.gio_pid:
            q, depth = procs.get(q.ppid), depth + 1
        if q is not None and q.pid != 1 and q.uid == uid and q.start <= p.start:
            # launched process still alive, owned by the child, not a reused PID
            did = os.path.basename(p.gio_file)[:-8]
            e = cat.entries.get(did) or cat.by_lower.get(did.lower())
            if e:
                app_id, how = f"desktop:{e.id}", f"GIO env (pid {q.pid})"

    if not app_id and unit and kind in ("app", "autostart"):
        app_id, e, how = unit_entry(unit, p, procs, cat)

    root = None
    if not app_id:
        found = launch_root(p, procs, uid, cat)
        if found:
            root, e = found
            app_id, how = f"desktop:{e.id}", f"tree (root {root.pid} {root.comm})"

    if not app_id or not e or not e.visible:
        name = os.path.basename(p.exe) if p.exe else p.comm
        return f"exe:{name}", None, "other", how or "no visible .desktop"

    if kind == "autostart":
        return app_id, e, "autostart", how
    # no autostart unit: root that matches an autostart entry and started with
    # the session (exec-once, startup scripts) counts as autostart too
    r = root or p
    if (e.id.lower() in cat.autostart or os.path.basename(r.exe or r.comm) in cat.autostart) \
            and r.start - t_session < AUTOSTART_WINDOW_S:
        return app_id, e, "autostart", how + ", autostart entry"
    return app_id, e, "app", how


# ------------------------------------------------------------------------- main

def scan(uid, home, want_env=True):
    procs = scan_procs(want_env)
    cat = Catalog(load_entries(home), load_autostart(home))
    mine = [p for p in procs.values() if p.uid == uid and p.argv]
    t_session = min((p.start for p in mine), default=0)
    return procs, [(p, identify(p, procs, uid, cat, t_session)) for p in mine]


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    uid = int(args[0]) if args else os.getuid()
    import pwd
    home = pwd.getpwuid(uid).pw_dir

    if "--time" in sys.argv:
        n = 20
        cpu0, wall0 = time.process_time(), time.perf_counter()
        for _ in range(n):
            procs, rows = scan(uid, home)
        cpu, wall = (time.process_time() - cpu0) / n, (time.perf_counter() - wall0) / n
        print(f"# {len(procs)} processes total, {len(rows)} of uid {uid}")
        print(f"# per scan: cpu {cpu * 1000:.1f} ms, wall {wall * 1000:.1f} ms "
              f"(python, incl. reloading .desktop files every time)")
        return

    show_cmdline = "--show-cmdline" in sys.argv
    show_all = "--all" in sys.argv
    procs, rows = scan(uid, home)

    print(f"# uid={uid}  desktop={os.environ.get('XDG_CURRENT_DESKTOP', '?')}  "
          f"session={os.environ.get('XDG_SESSION_TYPE', '?')}  euid={os.geteuid()}  processes={len(rows)}")
    print(f"{'PID':>7}  {'COMM':<16} {'KIND':<9} {'APP_ID':<42} HOW | UNIT")
    for p, (app_id, e, kind, how) in sorted(rows, key=lambda r: r[0].pid):
        if kind == "other" and not show_all:
            continue
        print(f"{p.pid:>7}  {p.comm[:16]:<16} {kind:<9} {app_id[:42]:<42} {how} | {p.unit}")
        if show_cmdline:
            print(f"{'':>9}cmd: {' '.join(p.argv)[:160]}")

    summary = {}
    for p, (app_id, e, kind, how) in rows:
        key = (kind, app_id, e.name if e else "")
        summary[key] = summary.get(key, 0) + 1
    for want, title in [("app", "apps (counted as usage)"),
                        ("autostart", "autostart apps (tracked, not counted)"),
                        ("other", "other bucket (not apps)")]:
        items = sorted((k, n) for k, n in summary.items() if k[0] == want)
        print(f"\n# {title}: {len(items)}")
        for (kind, app_id, name), n in items:
            print(f"  {n:>4}  {app_id:<48} {name}")


if __name__ == "__main__":
    main()
