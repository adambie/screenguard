#!/usr/bin/env python3
"""Phase 0 spike for docs/feature-app-usage-limits.md — READ-ONLY.

Lists the processes of one user and derives the candidate app_id the agent would
use, following the identification rules in the plan:

  1. systemd unit of the process (from /proc/<pid>/cgroup):
       app[-<launcher>]-<ApplicationID>-<RANDOM>.scope
       app[-<launcher>]-<ApplicationID>[@<RANDOM>].service
     inside .../user@<uid>.service/app.slice (possibly nested, e.g. app-graphical.slice)
  2. flatpak (launcher "flatpak") and snap (snap.<name>.<app>-<uuid>.scope)
  3. special cases from the command line (Steam AppId, Minecraft Java, Wine .exe)
  4. exe basename as last resort

Nothing is modified. Command lines are NOT printed unless --show-cmdline is given
(they can contain URLs, file names, tokens). Run as root to see `exe` of other
users' processes; without root, exe is only readable for your own processes.

Usage:  app-scan.py [UID] [--show-cmdline] [--all]
        --all  also list processes outside app.slice (session/background/other)
"""
import os
import re
import sys

DESKTOP_DIRS = [
    "/usr/share/applications",
    "/usr/local/share/applications",
    "/var/lib/flatpak/exports/share/applications",
    "/var/lib/snapd/desktop/applications",
]

UNIT_RE = re.compile(r"^app-(?P<rest>.+?)(?:-(?P<rand>[0-9a-f]{6,})\.scope|@(?P<srand>[^.]+)\.service|\.service)$")


def unescape(s):
    """systemd unit-name escaping: \\x2d -> '-' etc."""
    return re.sub(r"\\x([0-9a-fA-F]{2})", lambda m: chr(int(m.group(1), 16)), s)


def read(path, binary=False):
    try:
        with open(path, "rb" if binary else "r") as f:
            return f.read()
    except OSError:
        return None


def proc_uid(pid):
    status = read(f"/proc/{pid}/status") or ""
    m = re.search(r"^Uid:\s+(\d+)", status, re.M)
    return int(m.group(1)) if m else None


def cgroup_path(pid):
    data = read(f"/proc/{pid}/cgroup") or ""
    for line in data.splitlines():
        if line.startswith("0::"):
            return line[3:]
    return ""


def classify_cgroup(path):
    """Return (kind, unit): app / autostart / user-service / session / background / ..."""
    parts = [p for p in path.split("/") if p]
    unit = parts[-1] if parts else ""
    if "app.slice" in parts:
        # deepest .scope/.service below app.slice
        idx = parts.index("app.slice")
        units = [p for p in parts[idx + 1:] if p.endswith((".scope", ".service"))]
        unit = units[-1] if units else unit
        if not unit.startswith("app-"):
            # plain user services (syncthing.service, dbus-:1.x-...@0.service, portals)
            return "user-service", unit
        if unit.endswith("@autostart.service"):
            return "autostart", unit
        return "app", unit
    if "session.slice" in parts:
        return "session", unit
    if "background.slice" in parts:
        return "background", unit
    if unit.startswith("snap."):
        return "app", unit
    if re.match(r"session-\w+\.scope$", unit):
        return "login-session", unit
    return "other", unit


def parse_unit(unit):
    """-> (launcher, application_id) or (None, None)."""
    if unit.startswith("snap."):
        name = unit.split(".")[1]
        return "snap", name
    m = UNIT_RE.match(unit)
    if not m:
        return None, None
    fields = m.group("rest").split("-")  # real '-' inside ids are escaped as \x2d
    if len(fields) >= 2:
        launcher, app = fields[0], "-".join(fields[1:])
    else:
        launcher, app = None, fields[0]
    return (unescape(launcher) if launcher else None), unescape(app)


def desktop_dirs(uid):
    dirs = list(DESKTOP_DIRS)
    try:
        import pwd
        home = pwd.getpwuid(uid).pw_dir
        dirs += [f"{home}/.local/share/applications",
                 f"{home}/.local/share/flatpak/exports/share/applications"]
    except KeyError:
        pass
    return dirs


def find_desktop(app, dirs):
    """Look for <app>.desktop, case-insensitively. Returns (desktop_id, Name)."""
    if not app:
        return None, None
    want = (app + ".desktop").lower()
    for d in dirs:
        try:
            for f in os.listdir(d):
                if f.lower() == want:
                    text = read(os.path.join(d, f)) or ""
                    m = re.search(r"^Name=(.+)$", text, re.M)
                    return f[:-8], (m.group(1).strip() if m else None)
        except OSError:
            continue
    return None, None


_EXEC_INDEX = {}


def desktop_by_exec(exe, dirs):
    """Second chance: the .desktop file whose Exec= binary is `exe`."""
    if not exe:
        return None, None
    key = tuple(dirs)
    if key not in _EXEC_INDEX:
        index = {}
        for d in dirs:
            try:
                files = os.listdir(d)
            except OSError:
                continue
            for f in files:
                if not f.endswith(".desktop"):
                    continue
                text = read(os.path.join(d, f)) or ""
                if re.search(r"^NoDisplay=true", text, re.M):
                    continue
                m = re.search(r"^Exec=(\S+)", text, re.M)
                n = re.search(r"^Name=(.+)$", text, re.M)
                if m:
                    binary = os.path.basename(m.group(1).strip('"'))
                    index.setdefault(binary, (f[:-8], n.group(1).strip() if n else None))
        _EXEC_INDEX[key] = index
    return _EXEC_INDEX[key].get(exe, (None, None))


def special_case(cmdline_args):
    joined = " ".join(cmdline_args)
    m = re.search(r"SteamLaunch AppId=(\d+)", joined)
    if m:
        return f"steam:{m.group(1)}"
    if cmdline_args and os.path.basename(cmdline_args[0]).startswith("java") and "minecraft" in joined.lower():
        return "minecraft:java"
    for a in cmdline_args:
        if a.lower().endswith(".exe"):
            return "wine:" + os.path.basename(a.replace("\\", "/")).lower()
    return None


def candidate_app_id(kind, unit, args, exe, dirs):
    special = special_case(args)
    if special:
        return special, "special-case"
    if kind in ("app", "autostart"):
        launcher, app = parse_unit(unit)
        if launcher == "snap":
            return f"snap:{app}", "snap scope"
        if launcher == "flatpak":
            return f"flatpak:{app}", "flatpak scope"
        desktop_id, _ = find_desktop(app, dirs)
        if desktop_id:
            return f"desktop:{desktop_id}", f"scope ({launcher or '-'}) + .desktop"
        desktop_id, _ = desktop_by_exec(exe, dirs)
        if desktop_id:
            return f"desktop:{desktop_id}", f"scope ({launcher or '-'}) + exe→.desktop"
        if app:
            return f"scope:{app}", f"scope ({launcher or '-'}), no .desktop"
    if exe:
        return f"exe:{exe}", "exe fallback"
    return "exe:?", "exe unreadable (run as root)"


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    show_cmdline = "--show-cmdline" in sys.argv
    show_all = "--all" in sys.argv
    uid = int(args[0]) if args else os.getuid()
    dirs = desktop_dirs(uid)

    rows = []
    for pid in sorted((int(p) for p in os.listdir("/proc") if p.isdigit())):
        if proc_uid(pid) != uid:
            continue
        raw = read(f"/proc/{pid}/cmdline", binary=True)
        if raw is None:
            continue
        argv = [a.decode(errors="replace") for a in raw.split(b"\0") if a]
        if not argv:
            continue  # kernel thread / zombie
        comm = (read(f"/proc/{pid}/comm") or "").strip()
        try:
            exe = os.path.basename(os.readlink(f"/proc/{pid}/exe"))
        except OSError:
            exe = None
        kind, unit = classify_cgroup(cgroup_path(pid))
        if kind not in ("app", "autostart") and not show_all:
            continue
        app_id, how = candidate_app_id(kind, unit, argv, exe, dirs)
        rows.append((pid, comm, exe or "?", kind, unit, app_id, how,
                     " ".join(argv)[:160] if show_cmdline else ""))

    print(f"# uid={uid}  desktop={os.environ.get('XDG_CURRENT_DESKTOP', '?')}  "
          f"session={os.environ.get('XDG_SESSION_TYPE', '?')}  processes={len(rows)}")
    print(f"{'PID':>7}  {'COMM':<16} {'EXE':<18} {'KIND':<10} {'APP_ID':<40} HOW / UNIT")
    for pid, comm, exe, kind, unit, app_id, how, cmd in rows:
        print(f"{pid:>7}  {comm[:16]:<16} {exe[:18]:<18} {kind:<10} {app_id[:40]:<40} {how} | {unit}")
        if cmd:
            print(f"{'':>9}cmd: {cmd}")

    summary = {}
    for r in rows:
        summary.setdefault(r[5], 0)
        summary[r[5]] += 1
    print("\n# summary: app_id -> process count")
    for app_id, n in sorted(summary.items(), key=lambda kv: (-kv[1], kv[0])):
        print(f"  {n:>4}  {app_id}")


if __name__ == "__main__":
    main()
