# Feature — per-app usage reports, app groups and group limits

- **Status:** In progress — Phase 0 (detection research)
- **Started:** 2026-10-20
- **Tracking issue:** [#27](https://github.com/adambie/screenguard/issues/27) (separate game and screen time)
- **Inspiration:** Google Family Link (per-app usage, app limits); Timekpr-nExT "PlayTime" (process masks, separate allowance)

---

## How to use this document (read first, every session)

This feature spans many sessions. This file is the single source of truth for
progress — not chat history.

1. **Start of a session:** read [Current status](#current-status) and the
   [Session log](#session-log). Then find the first unchecked `- [ ]` step in the
   current phase.
2. **Work in order.** Steps are ordered so each one builds on the previous.
   Don't start a phase until the previous phase's **exit criteria** are all ticked.
3. **Tick a step (`- [x]`) only when its "Done when" check passes** — tests green,
   behaviour verified, not just "code written".
4. **Commit this file together with the code** that completes a step, so the
   checkboxes and the repo never disagree.
5. **Record every design decision or deviation** in the [Decision log](#decision-log)
   (date, what, why). If reality contradicts this plan, update the plan — don't
   silently diverge.
6. **End of a session:** update [Current status](#current-status) (what's done,
   what's next, anything half-finished) and add a line to the [Session log](#session-log).

**Repo conventions to keep:**
- Version bumps are a separate commit: `chore: bump agent + server to X.Y.Z`
  (both `crates/agent` and `crates/server` `Cargo.toml` + `Cargo.lock`).
- Commit messages end with the `Co-Authored-By:` line used in this repo.
- Never push / tag without the maintainer's explicit go-ahead.
- Local test servers must run with **`enable_mdns = false`** — a test server
  advertising `_parctrl._tcp` on the LAN can be discovered by real unpaired agents.
- Don't run `screenguard-agent` / `screenguard-server` with unknown flags on real
  machines: unknown flags are ignored and start a second instance.

---

## Current status

> Update this block at the end of every session.

- **Phase:** 1 — Per-app usage reports (Phase 0 done)
- **Last completed step:** 1.19 (real-machine e2e passed); 1.18 skipped, 1.20 mostly done
- **Next step:** Phase 2 — 2C enforcement: 2.9 warnings, 2.10 closing apps (safety rules!), 2.11 relaunch, 2.12 blocked groups. Phase 1 leftovers: rest of 1.20 (real 0.10.17 agent vs this server).
  No release until Phase 2 is done (decision log).
- **In progress / half-done:** —
- **Blockers / open questions:** none blocking; Q5 open, see
  [Open questions](#open-questions)

---

## Goal

Let the admin:
1. **See** how long each child used each application — per day, per week, in total.
2. **Group** applications (admin-defined: "Games", "Browsers", …).
3. **Limit** time per group per weekday (e.g. Games 30 min on school days), with
   0 = blocked. Group time also counts toward the existing daily screen-time limit.
4. Have the agent **warn, then close** the group's apps when its time is up, while
   the rest of the computer stays usable until the overall limit.

### Non-goals (for now)

- Distinguishing websites inside a browser (a web game counts as "browser").
  A later idea is per-domain groups via the existing DNS web filter — see Phase 3.
- Bulletproof blocking against a technical child who renames binaries. Phase 1–2
  are best-effort like Timekpr; a stricter allow-list mode is a possible later option.
- Mobile / non-Linux agents.

---

## Concept summary

### Measuring usage (agent)

- **Source of truth: systemd cgroups.** Apps launched from the desktop run in
  transient scopes under the user's manager, named after the app:
  `user@<uid>.service/app.slice/app-gnome-org.mozilla.firefox-4821.scope`,
  `app-flatpak-com.mojang.Minecraft-5120.scope`, `snap.spotify.spotify-….scope`.
  The agent reads `/proc/<pid>/cgroup` for each process of a managed UID.
- **Filtering system processes:**
  1. only processes owned by managed children (UID ≥ `min_uid` with a profile);
  2. only processes under `app.slice` (ignore `session.slice`, `background.slice`);
  3. fallback for desktops without per-app scopes (verify in Phase 0): every
     process of that UID in the graphical session, minus a built-in ignore list of
     desktop components, keyed by executable.
- **App identity (`app_id`)**, first match wins:
  `desktop:<id>` (scope name ↔ `.desktop` file) · `flatpak:<id>` · `snap:<name>` ·
  built-in special cases (`steam:<AppId>` from `SteamLaunch AppId=<n>`,
  `minecraft:java` from a `java` cmdline containing minecraft, wine/proton `.exe`
  path) · `exe:<basename>` as last resort.
- **Display name / icon** from the `.desktop` file (`Name=`, `Icon=`), else the id.
- **Accounting v1 ("running while active"):** each scan tick, every app with at
  least one live process gets the tick's seconds **iff** the user's session
  currently counts as active — exactly today's rule in `crates/agent/src/dbus.rs`
  (active, unlocked, not idle). One app counts once per tick regardless of process
  count.
- **v2 (Phase 3):** foreground-only accounting via `screenguard-tray`, which runs
  inside the user session and can see the focused window (X11 `_NET_ACTIVE_WINDOW`,
  KWin script, GNOME Shell extension).

### Privacy rule

Command lines can contain URLs, file names, tokens. **Cmdlines never leave the
machine.** They are only used locally by the built-in identifiers and by
admin match rules evaluated on the agent. The server receives `app_id`,
display name, icon name and seconds — nothing else.

### Data flow

```
agent (root)                                   server                        web UI
──────────────                                 ──────                        ──────
scan /proc every N s
  → identify app_id per process
  → if session counts: +N s per app
  → local table app_usage_log (uid,date,app_id,seconds)
  → MSG_APP_USAGE_SYNC  (day TOTALS, idempotent) ──► app_usage, app_catalog ──► reports
                                                       app_groups / limits  ◄── admin groups & limits
  ◄── config_push: UserConfig.app_groups (match rules + limits)
  ◄── remaining_update: per-group remaining (multi-machine sums)
enforce: warn → SIGTERM → cgroup.kill ; relaunch → kill again + notify
```

**Sync uses per-day totals, not deltas**, so retries and reconnects can never
double-count (server upserts with the reported total).

### Enforcement (agent, Phase 2)

1. Warn at the profile's existing warning thresholds via the existing desktop
   notification path (`--notify`), message localized in `crates/agent/src/i18n.rs`.
2. At 0: SIGTERM all processes of the matched app(s), wait a grace period (~10 s),
   then write `1` to the scope's `cgroup.kill` (kills launcher + children atomically).
   Fallback without a scope: SIGKILL the matched process tree.
3. Relaunch: the scan loop sees it again within one tick → kill again + notify
   "<group> time is up for today".
4. **Safety:** only ever kill processes that (a) belong to a managed child UID,
   (b) matched an app in a *limited* group. Never kill anything in `session.slice`,
   never a process of a non-managed UID (root, system users, the admin), never
   the agent or tray.

### Compatibility rules (non-negotiable)

- Existing session-time tracking and enforcement must be **unchanged** — the app
  feature only *adds*. Existing tests must stay green.
- New fields on existing wire types use `#[serde(default)]` /
  `skip_serializing_if` so old ↔ new agents and servers interoperate.
- New agent ↔ old server: the agent only sends `MSG_APP_USAGE_SYNC` when the
  server announced support (or the server ignores unknown message types — verify
  in step 1.4). Old agent ↔ new server: no app data, reports page says
  "update the agent".
- Agent advertises capability `"app_usage"` in `agent_hello.capabilities`
  (same mechanism as `"web_filter"`); server stores it and the UI shows it.
- Server schema changes are portable migrations (v10, v11, …) in
  `crates/server/src/db.rs` — run on SQLite **and** Postgres.
- Agent DB: new tables via `CREATE TABLE IF NOT EXISTS` in
  `crates/agent/src/db.rs::migrate`.

---

## File map (where things live today)

| Area | File |
|---|---|
| Session active/idle/locked detection | `crates/agent/src/dbus.rs` (`SessionUsageState`) |
| Agent local DB + migrations | `crates/agent/src/db.rs` |
| Agent loops / enforcement | `crates/agent/src/main.rs`, `heartbeat.rs`, `enforcement.rs` |
| Agent notifications (as user) | `crates/agent/src/dbus.rs` (`--notify` self-spawn), `i18n.rs` |
| Wire messages | `crates/common/src/messages.rs`, `crates/common/src/models.rs` |
| Server WS handling | `crates/server/src/ws.rs` |
| Remaining-time calculation | `crates/server/src/remaining.rs` |
| Server DB + migrations | `crates/server/src/db.rs` (currently schema **v9**) |
| REST API | `crates/server/src/api/{profiles,usage,agents}.rs`, router in `api/mod.rs` |
| Web UI | `webui/app.py`, `webui/templates/*.html`, `webui/translations/*.json` (6 langs) |
| Existing similar feature to copy patterns from | web filter: `docs/web-filtering-technical.md`, `crates/agent/src/web_filter.rs` |

---

## Phase 0 — Detection research (no product code)

Purpose: decide **from documentation and known behaviour** how apps can be identified
on any desktop — not by sampling every desktop (impossible to cover all
combinations). Real machines are only used to sanity-check the result
(maintainer's Hyprland machine, the KDE test machine `screenguardTest`).

- [x] **0.1 Write a read-only scan script** (`docs/spikes/app-scan.py`): lists a
  user's processes with cgroup unit, exe and derived candidate `app_id`. Used to
  sanity-check rules on available machines.
- [x] **0.2 Research the signals and where they exist** → [Signal matrix](#signal-matrix).
- [x] **0.3 Define a desktop-independent identification algorithm** →
  [Identification algorithm](#identification-algorithm).
- [x] **0.4 Define a desktop-independent "is this an app?" filter** (no per-desktop
  ignore lists) → [What counts as an app](#what-counts-as-an-app).
- [x] **0.5 Sanity-check the algorithm** on the available machines by extending
  `app-scan.py` to implement it (incl. `environ` and process-tree steps; needs
  root for other users' `environ`/`exe`). *Done when:* on Hyprland and KDE every
  real app the user opened maps to the right `app_id` and no desktop component is
  listed as an app. → Passed 2026-10-20, see [Sanity check results](#sanity-check-results-05).
- [x] **0.6 Measure cost:** time one full scan (incl. `environ` reads) on the
  slowest available machine; pick the scan interval (target ≤ 1 % CPU; expected 5 s).
  → 2026-10-20 with `docs/spikes/scanbench.rs` (the agent's `/proc` reads in Rust,
  200 iterations, % of one core):

  | Machine | Processes (all / child's) | Full scan | Cached tick | At 5 s (full / cached) |
  |---|---|---|---|---|
  | Ryzen 7 4800H, Hyprland, Chromium + Electron open | 464 / 101 | 5.6 ms | 2.0 ms | 0.11 % / 0.04 % |
  | screenguardTest, QEMU VM with 1 vCPU, KDE | 153 / 48 | 1.2 ms | 0.4 ms | 0.02 % / 0.01 % |

  Cost scales with the child's process count (~25–55 µs each), not the CPU; a
  heavy session with 500 processes would still be ~0.5 % full / ~0.2 % cached.
  Full tracker tick (scan + identification + names), measured later with
  `bench_tracker_tick`: 6 ms steady, 27 ms first tick = **0.12 %** at 5 s.
  **Decision: 5 s interval; cache identities by `(pid, start time)`** so a tick
  only lists `/proc` and checks owner + start time, doing the full reads and
  identification only for new processes. The `.desktop` catalogue is cached and
  reloaded when an applications dir's mtime changes.

**Exit criteria (Phase 0):**
- [x] Identification rules defined independently of the desktop
- [x] "Is it an app" filter defined without per-desktop lists
- [x] Sanity check passed on available machines (0.5)
- [x] Scan interval decided (0.6)

### Signal matrix

Researched 2026-10-20. "Unit" = systemd unit following the
[systemd desktop-environment convention](https://systemd.io/DESKTOP_ENVIRONMENTS/):
`app[-<launcher>]-<ApplicationID>-<RANDOM>.scope` or
`app[-<launcher>]-<ApplicationID>[@<RANDOM>].service` (ids systemd-escaped, e.g. `\x2d`).

| Signal | Where it exists | Notes |
|---|---|---|
| App **unit** under `app.slice` | GNOME (gnome-shell: `app-gnome-<id>-<pid>.scope`); KDE Plasma (KIO: `app-<id>@<uuid>.service` / scope); compositors started via **uwsm** (Hyprland, Sway, …: `app-<Compositor>-<id>-<rand>.scope`); apps that move themselves (Chromium: `app-org.chromium.Chromium-<pid>.scope`); D-Bus/xdg activation; XDG autostart (`app-<id>@autostart.service`); Fedora/RHEL also patch GLib so every GIO launch gets a scope | Best signal when present. **Not** guaranteed: upstream GLib does not create scopes; no evidence that Cinnamon, MATE, XFCE, LXQt or plain i3/Openbox do. |
| **Flatpak** scope `app-flatpak-<id>-<n>.scope` | Any desktop, Flatpak apps (created by flatpak itself) | Reliable; id = Flatpak app id. |
| **Snap** scope `snap.<name>.<app>-<uuid>.scope` | Any desktop, Snap apps (created by snapd) | Reliable. |
| `GIO_LAUNCHED_DESKTOP_FILE` + `GIO_LAUNCHED_DESKTOP_FILE_PID` in `/proc/<pid>/environ` | Anything launched through GIO's `GAppInfo` — GNOME, Cinnamon, MATE, Budgie and most GTK launchers/menus | Gives the exact `.desktop` path. Inherited by children (the `_PID` tells which process was launched). Reading another user's `environ` needs root — the agent is root. |
| **Process tree** + `.desktop` matching (`Exec=`/`TryExec=` binary, `StartupWMClass`) | Everywhere | Universal fallback; weaker with wrapper scripts. |
| Steam: ancestor `reaper SteamLaunch AppId=<n> -- …` | Steam games (native and Proton) | `steam:<n>`. |
| Cmdline patterns: `java` + Minecraft main class/jar; `*.exe` under Wine/Proton | Minecraft Java, Wine apps | Local only (privacy rule). |

Sanity-check samples: `docs/spikes/hyprland-arch.txt` (uwsm scopes, Chromium
self-scope, autostart), `docs/spikes/kde-cachyos.txt` (`app-<id>@<uuid>.service`).

### Identification algorithm

For each process of a managed child (real UID), first match wins (validated in
0.5; reference implementation: `docs/spikes/app-scan.py`, `identify()`):

0. **Special cases** (checked on the process and its ancestors) — first, because
   e.g. a Steam game otherwise inherits Steam's own unit/env:
   - an ancestor whose `argv[0]` basename is `reaper`, with a `SteamLaunch` argument
     and an `AppId=<n>` argument → `steam:<n>`. Match exact arguments only; a loose
     substring search matched an unrelated shell script that merely contained the text;
   - `argv[0]` is `java*` with a Minecraft main class or jar (`net.minecraft`,
     `minecraft*.jar`, `cpw.mods`, `fabricmc`, `quiltmc`) → `minecraft:java`;
   - `argv[0]`/`argv[1]` ends in `.exe` → `wine:<exe>`.
1. **Service units are never apps**: non-`app-` units under `app.slice` (user
   services, portals, input methods) and anything in `background.slice` → "other".
   `session.slice` is *not* excluded: under uwsm, apps started from a compositor
   keybinding without `uwsm app` stay in the compositor's unit there (the tree
   step handles them).
2. **Flatpak / Snap unit** → `flatpak:<id>` / `snap:<name>`.
3. **GIO launch env**: `environ` has `GIO_LAUNCHED_DESKTOP_FILE`, and
   `GIO_LAUNCHED_DESKTOP_FILE_PID` is the process or an ancestor, is not PID 1,
   is owned by the child and started no later than the process (otherwise the env
   is stale or the PID was reused) → `desktop:<id>` from that path. Checked
   **before** the unit: a GIO launch from inside another app (a file manager
   opening a PDF) inherits that app's unit, but the env names the new app.
4. **App unit** (`app-…` under `app.slice`) → parse launcher + application id →
   resolve to a `.desktop` file (resolution chain below). If the id doesn't
   resolve (uwsm/xdg-terminal-exec units carry a command, e.g.
   `app-Hyprland-xdg\x2dterminal\x2dexec-….scope`), resolve the **oldest process
   in the unit** instead → `desktop:<id>`.
   - **Unit = the first `.scope`/`.service` below the slice.** Only slices nest;
     deeper cgroups are sub-cgroups the app made itself (Konsole:
     `app-org.kde.konsole-<pid>.scope/main.scope`, one per tab).
   - **Chromium self-scope** `app-org.chromium.Chromium-<pid>.scope`: Chromium *and
     every Electron app* move their main process into a scope with this id, so it
     says nothing about the app (seen: Bitwarden). Ignore the id; use the unit of
     the process's children (they stay in the launcher's unit), else continue
     with step 5.
5. **Process tree**: walk up from the process until the parent is PID 1, the
   user's `systemd`, or another user; the **launch root** is the highest process
   on that path that resolves to a visible `.desktop` entry → `desktop:<id>`.
   Desktop shells/panels have no visible entry, so they're skipped naturally; a
   terminal or file manager owns what it starts (Q6).
6. **No visible `.desktop` match** → not an app (see filter below), kept only as
   `exe:<name>` in a hidden "other" bucket for diagnostics.

**Resolution chain.**
- Unit id → `<id>.desktop` (case-insensitive) → `StartupWMClass` match.
- Process → try the names exe basename, `argv[0]` basename, `comm`; each via the
  `.desktop` whose `Exec=`/`TryExec=` binary equals the name, then
  `StartupWMClass`, then the last dotted component of the desktop id.
  - `Exec=` parsing drops field codes (`%u` …) and leading `env VAR=…`; only the
    `[Desktop Entry]` group counts, not `[Desktop Action …]`.
  - Tie-break between several entries: visible first, then id equal to the name,
    then `Exec` without extra args (seen: `foot` vs `foot-server`).
- **Interpreters** (`electron*`, `python*`, `java`, `node`, shells, `wine*`, …): use
  the script/archive in `argv` instead. Try its basename without extension, then
  its parent directories, e.g. `electron39 /usr/lib/bitwarden/app.asar` →
  `bitwarden` → `bitwarden.desktop`.
- Search `$XDG_DATA_HOME` (the child's), `$XDG_DATA_DIRS`, and the Flatpak/Snap
  export dirs; recurse into subdirectories (desktop id = relative path with `/`
  replaced by `-`); an earlier dir wins.
- Always strip ` (deleted)` from `/proc/<pid>/exe`.
- Electron/Chromium processes are non-dumpable, so even their own user can't read
  their `exe`/`environ`. Root can; `argv` works either way.

### What counts as an app

Desktop-independent rule — **an app is something the child could launch from a
menu**:

- it resolves to a `.desktop` entry with `Type=Application`, not `NoDisplay=true`,
  not `Hidden=true`; **and**
- it is not an autostart launch (`@autostart.service` unit, or the root process
  matches an entry in `/etc/xdg/autostart` / `~/.config/autostart` and started
  within 60 s of the session's first process) — autostart apps are tracked but not
  counted as usage by default; **and**
- it is not in `background.slice` and not a non-`app-` unit under `app.slice`
  (user services). `session.slice` processes may be apps (see step 1).

Desktop components (panels, shells, portals, daemons) either have no visible
`.desktop` entry or are `NoDisplay`/autostart, so they drop out **without any
per-desktop ignore list**. Everything else goes to the hidden "other" bucket.

### Sanity check results (0.5)

2026-10-20, `app-scan.py` run unprivileged (own processes only):

- **Hyprland via uwsm (Arch), maintainer's machine.** Apps: Chromium (30 processes;
  main in the Chromium self-scope, the rest in `app-Hyprland-chromium-….scope`),
  foot (via the `xdg-terminal-exec` unit → oldest process). Autostart: Bitwarden
  (Electron self-scope → children's `@autostart` unit), Nextcloud. "Other": 41
  desktop components (Hyprland, quickshell, portals, pipewire, gvfs, syncthing,
  fcitx5 service, uwsm, …). Nothing misclassified.
- **KDE Plasma 6 Wayland (CachyOS), screenguardTest.** Apps opened for the test
  through KIO (`kioclient exec`, as Plasma's launcher does) and GIO (`gtk-launch`):
  Firefox (17 processes incl. sandbox helpers), Dolphin (incl. kioworkers), KCalc,
  Konsole (self-scope with `main.scope` sub-cgroup), Alacritty — all via
  `app-<id>@<uuid>.service` units — and KWrite via the GIO env (no unit;
  upstream GLib). "Other": 42 components (plasmashell, kwin, kded6, ksmserver,
  portals, kdeconnectd, tray apps, …). Nothing misclassified.
- **Simulated** (fake processes on Hyprland): GIO env with a valid PID →
  `desktop:<id>`; stale GIO env (PID 1) → ignored; `reaper SteamLaunch
  AppId=570 -- …` with a child → `steam:570` for both; Minecraft `java … net.minecraft…`
  → `minecraft:java`; `Game.exe` → `wine:game.exe`.
- **Bugs found and fixed in the algorithm:** innermost cgroup taken as the unit
  (Konsole); the Chromium self-scope id; services not excluded (fcitx5); the Steam
  match too loose; stale GIO PIDs accepted. All are reflected in the algorithm
  above.
- Not covered by real machines (by design, see the decision log): GNOME, Cinnamon/MATE,
  XFCE, Flatpak/Snap, real Steam/Minecraft. Phase 1 fixture tests (1.6) cover them.

## Phase 1 — Per-app usage reports (observe only, no enforcement)

### 1A. Shared types (`crates/common`)

- [x] **1.1 `AppId` helpers** — `crates/common/src/apps.rs`: the `app_id` string
  format (`desktop:` / `flatpak:` / `snap:` / `steam:` / `minecraft:` / `wine:` / `exe:`),
  parse/validate, max length (255). `AppId::is_app()` is false for `exe:` (the
  "other" bucket). Wire format stays a plain string (an old server drops unknown
  kinds per entry). `desktop:` names allow anything but control chars and `/`
  (Wine creates `wine-Programs-Game Name.desktop`).
  *Done when:* unit tests cover each prefix and reject malformed ids.
- [x] **1.2 Wire types** — in `messages.rs`: `MSG_APP_USAGE_SYNC` (`"app_usage_sync"`),
  `AppUsageSync { entries: Vec<AppUsageEntry> }`,
  `AppUsageEntry { local_uid, date, app_id, display_name, icon: Option<String>, seconds_total }`.
  `date` is `NaiveDate` like `UsageEntry`; `icon` is a theme icon name, omitted
  when `None`; `seconds_total` is the day's running total, not a delta.
  *Done when:* serde round-trip test passes.

### 1B. Agent — scanner and accounting (`crates/agent`)

- [x] **1.3 Capability** — advertise `"app_usage"` in `agent_hello.capabilities`
  (Linux with cgroup v2 present: `app_scan::supported()` checks
  `/sys/fs/cgroup/cgroup.controllers`). *Done when:* server log shows it on connect.
  → Builds; the server already logs `capabilities=[…]` on connect. Seeing it in
  a real log needs a root-installed agent, so that check is part of 1.19.
- [x] **1.4 Check unknown-message handling on old servers** — confirm a 0.10.x
  server ignores an unknown WS message type without dropping the connection.
  Record result in decision log; if it drops, gate sending on server version.
  → Tolerated by every server version: `handle_agent_message` has a catch-all
  arm that logs at debug and returns `Ok` since the first server commit
  (`204a3be`), and message-handling errors are only logged, never close the
  socket. No version gate needed.
- [x] **1.5 `app_scan.rs` — process enumeration**: iterate `/proc`, keep PIDs whose
  real UID is a managed child, read cgroup path, ppid, `exe` basename, cmdline and
  the two `GIO_LAUNCHED_DESKTOP_FILE*` variables from `environ` (local only).
  Cache per `(pid, start time)`; only new processes get the full reads (0.6).
  `ppid` is refreshed from `stat` on every tick (reparenting).
  *Done when:* unit tests run against a fake `/proc` tree fixture (tempdir).
  → `ProcScanner::scan(uids)` → `ProcInfo`s. Notes:
  - Real UID comes from `status`: non-dumpable processes (Electron) have
    root-owned `/proc/<pid>` entries.
  - The cache key also includes `comm`, so an `exec` (a wrapper script exec'ing
    `electron`) is re-read.
  - Other users' processes are cached without details, so their `status` isn't
    re-read on every tick. A tick reads only `stat` per process.
  - `fake_proc::FakeProc` (test-only, no new dependency) is reused by 1.6.
  - The module has `allow(dead_code)` until 1.8 wires it in; remove it then.
- [x] **1.6 Identification** — implement the [Identification algorithm](#identification-algorithm)
  and the [app filter](#what-counts-as-an-app). Needs `environ` and the process
  tree (ppid) from 1.5. *Done when:* table-driven tests pass against a **fixture
  catalogue built from the documented formats**, one fake `/proc` tree each:
  GNOME scope, KDE service, uwsm scope, Chromium self-scope, Flatpak, Snap,
  XDG autostart, GIO env without units (Cinnamon/MATE style), plain process tree
  without units or env (XFCE/i3 style), terminal with children, Steam reaper +
  Proton game, Minecraft Java, Wine `.exe`, binary `(deleted)`, plus desktop
  components that must *not* become apps.
  → `app_catalog.rs` (`.desktop` catalogue: ids, `Exec`/`TryExec`/`StartupWMClass`,
  visibility, `Name`, `Icon`, autostart set; fixture `app_catalog::fixture::standard`)
  and `app_ident.rs` (`identify_all(procs, catalog)` → `Identified { app_id,
  launch: App|Autostart|Other, desktop_id }`). 13 table tests cover every case
  above plus a nested GIO launch inside another app's unit, Prism/MultiMC, and a
  non-Minecraft `java`. Ignored `print_real_scan` test matches the Python spike
  on the maintainer's session.
  - Fix found while porting: special-case rules are tried along the whole
    ancestor chain one rule at a time (Steam → Minecraft → Wine). Otherwise a
    Proton game's own `Game.exe` matched `wine:` before the `reaper` ancestor.
- [x] **1.7 `.desktop` lookup** — map desktop ids to `Name=` (+ localized
  `Name[xx]=` if the profile language matches) and `Icon=` from
  `/usr/share/applications`, `~/.local/share/applications`, flatpak/snap export
  dirs. Cache, reload when an applications dir's mtime changes. *Done when:* tests with fixture `.desktop` files pass.
  → `DesktopEntry::name_for(lang)` (`xx_YY` → `xx` → first `xx_*` → `Name=`);
  `CatalogCache` per home: it reloads when an apps/autostart dir or a subdir changes
  mtime, checked at most every 30 s. `app_ident::display(id, catalog, lang, home)`
  covers the ids without their own entry:
  - Steam: the game shortcut (`steam://rungameid/<id>`), else
    `appmanifest_<id>.acf` in the default library, else "Steam game <id>";
  - Snap: `<snap>_<app>.desktop`;
  - Flatpak: its exported entry, else the id;
  - Minecraft: "Minecraft";
  - Wine: the `.exe` name.

  Icons are theme names or paths, kept as given (the web UI shows a generic icon
  when it can't render them).
- [x] **1.8 Accounting loop** — every scan tick, if the user's session counts
  (reuse `SessionUsageState` logic from `dbus.rs`), add the tick's seconds to each
  running `app_id` once. Handle clock jumps / suspend (cap a tick at 2× interval).
  *Done when:* unit test with a fake clock + fake scan results gives exact totals.
  → `app_usage.rs`:
  - `AppAccounting::tick(now, counting_uids, running)` returns the whole seconds
    per (uid, app). The first tick only starts the clock, gaps are capped at
    2 × 5 s, and a per-uid ms carry means 5.01 s ticks add up exactly.
  - `AppTracker::tick(now, users)` combines scanner, catalogue cache,
    identification and accounting, and also returns name/icon per app.
  - Only `Launch::App` is counted; autostart and "other" aren't reported in v1.
  - 5 tests. Wiring into `HeartbeatLoop` happens in 1.9 together with storage
    (nothing to persist without it); the `allow(dead_code)`s go then.
- [x] **1.9 Local storage** — agent DB table
  `app_usage_log(local_uid, date, app_id, display_name, icon, seconds, PRIMARY KEY(local_uid,date,app_id))`;
  midnight rollover like `usage_log`; prune > 35 days.
  *Done when:* DB tests pass; restarting the agent keeps today's totals.
  → Table plus `add_app_usage` / `get_unsynced_app_usage` / `mark_app_usage_synced`
  / `prune_app_usage` in `db.rs`:
  - Created in `create_schema` and `migrate`, so old DBs get it.
  - Cleared by `wipe_for_rebind`.
  - `mark_app_usage_synced` only marks a row if it still holds the total that was
    sent.
  - No midnight reset: rows are per date, and days older than 35 are pruned at
    midnight. Today's totals live in SQLite, so they survive a restart.

  Wired into `HeartbeatLoop`: a 5 s `app_scan_ticker` arm (only with cgroup v2)
  → `track_app_usage()`, which builds `TrackedUser`s:
  - home from `users::home_dirs()`;
  - language from the cached enforcement config;
  - `counting` = `uid_is_counting(session_states)`, the screen-time rule.

  Errors are logged and never stop the loop. The `allow(dead_code)`s are removed.
- [x] **1.10 Sync** — send today's (and any unsynced past days') totals via
  `MSG_APP_USAGE_SYNC` on connect and every heartbeat N (e.g. every 60 s);
  only when connected; never block the existing heartbeat.
  *Done when:* simulated server receives monotonically growing totals; reconnect
  re-sends totals without double counting server-side.
  → `HeartbeatLoop::send_app_usage_sync(resend_since)`: unsynced rows, in chunks
  of 500.
  - Sent every 12th scan tick (60 s) while online, and on `Connected`.
  - On `Connected` it also resends yesterday's and today's rows, because rows sent
    just before a disconnect may never have arrived.
  - Errors are logged, never `?`, so they can't stop the loop.

  Agent-side test: growing totals, nothing sent when unchanged, resend after a
  reconnect. "Without double counting server-side" is the server's MAX upsert
  (1.13).
- [x] **1.11 Feature switch** — agent config `app_tracking = true|false`
  (default true) so it can be disabled per machine without a rebuild.
  → `AgentConfig.app_tracking`, passed to `HeartbeatLoop::new`. Off (or no
  cgroup v2) means no tracker, no ticker, no sync and no `app_usage` capability.
  Not added to the installer templates, which don't list optional keys; the README
  documents it (1.21).

### 1C. Server — storage, ingestion, reports (`crates/server`)

- [x] **1.12 Migration v10** — tables:
  - `app_catalog(app_id PK, display_name, icon, first_seen, last_seen)`
  - `app_usage(agent_user_id FK, date, app_id, seconds, PRIMARY KEY(agent_user_id, date, app_id))`
  Portable SQL only. *Done when:* migration tests on SQLite pass and an
  upgrade from 0.10.17 is verified on SQLite **and** Postgres (docker).
  → `APP_USAGE_DDL` (used by `create_tables` for fresh installs and by
  `apply_v10` for upgrades), plus `agents.app_usage_available` (1.14) in the
  same migration; `CURRENT_VERSION = 10`, index on `app_usage(date)`. Unit test
  `migration_v10_adds_app_usage_and_keeps_pairings`.
  - Upgrade verified with `docs/spikes/upgrade-test.sh` (v0.10.17 built from its
    tag → this build) on SQLite **and** Postgres 16 in docker: schema 9 → 10; the
    paired agent (token hash, verified flag), the managed user, screen-time usage
    and `web_filter_available` are intact; admin login and `GET /agents` work;
    the restart is idempotent.
- [x] **1.13 Ingestion** — handle `MSG_APP_USAGE_SYNC` in `ws.rs`: upsert
  `app_usage` with `seconds = MAX(existing, reported)` (totals are monotonic per
  day), upsert catalog name/icon/last_seen. Reject ids failing `AppId` validation.
  *Done when:* tests cover retry (same total twice), growth, and bad ids.
  → `crates/server/src/app_usage.rs::ingest()`, called from the `ws.rs` arm.
  - Rejects: ids failing `AppId::parse`, `exe:` ids, dates more than a day after
    the server's date, unknown local users.
  - Clamps seconds to 86 400 and cleans names/icons (control chars, length).
  - Upsert keeps the larger total (portable `CASE`, like `reconcile_usage_seconds`).
  - 3 SQLite tests plus an ignored `postgres_ingest` test (`SG_TEST_PG_URL`),
    which passed against Postgres 16.
- [x] **1.14 Capability storage** — store `"app_usage"` capability per agent
  (like `web_filter_available`), expose in agent API.
  → `agents.app_usage_available` (v10), set on every hello
  (`update_agent_app_usage`), `app_usage_available` in `GET /agents` and
  `GET /agents/{id}` (`null` until the agent connects with a new version).
- [x] **1.15 Reports API** — `GET /api/v1/profiles/{id}/app-usage?range=day|week|total&date=YYYY-MM-DD`
  → per app: display name, icon, seconds; summed across all agents of the profile
  (profile-level, like screen time). Also per-day series for the week view.
  *Done when:* API tests with seeded data return correct sums and ordering.
  → `api::usage::get_app_usage` + `app_usage::{totals, daily, profile_has_capability}`.
  - `week` is the Monday–Sunday week containing `date`; `total` is all
    history; `date` defaults to the server's local date (like `/usage`).
  - Response: `{ range, from, to, apps: [{app_id, name, icon, seconds}],
    total_seconds, days: [{date, seconds, apps:[{app_id, seconds}]}] (week only),
    app_usage_available }`. Apps are sorted by time, then name, and summed over
    the profile's agents.
  - Tests: query tests (other profiles excluded, week bounds, catalogue name),
    an HTTP test (day/week/total, 400 on a bad range or date, 404, 401), and the
    Postgres test extended to the report queries (passed). Ingestion now also
    uses the server's local date.

### 1D. Web UI (`webui/`)

- [x] **1.16 Profile → "App usage" tab/page** — range switch (day / week / total),
  date picker, table sorted by time with bar per app; empty state; notice when no
  agent of the profile has the `app_usage` capability ("update the agent").
  → A separate page `/profiles/<id>/apps` (`profile_apps.html`), because the
  profile page is already a long column of cards. It's linked from an "App
  usage" button in the profile header and from the Usage card.
  - Day/Week/Total switch, ‹ › period navigation (future dates are clamped), a
    per-day bar chart in the week view (click a day to open it), and an app list
    with time bars.
  - Empty state, the capability notice, and a footnote explaining how time is
    counted. A letter avatar stands in for icons (theme icon names can't be
    rendered in a browser).
  - New `fmt_secs` filter (`<1m` under a minute). English strings added.
  - Checked locally (server + web UI on localhost with seeded data): totals
    match, and headless Chromium screenshots look right at desktop and phone
    widths.
- [x] **1.17 Translations** — all new strings in all 6 languages
  (`en, de, es, fr, pt, pl`).
  → The 13 new keys are added to every file without reformatting the
  hand-formatted JSON. Wording follows each file's existing terms (pt is European
  Portuguese). Rendered in pl/de/fr at phone width: the profile header now wraps
  its buttons (three didn't fit).
  - Found, not fixed (out of scope): the 17 older web-filter strings
    (`agents.badge_web_filter`, `profile.card_blocked_sites`,
    `flash.domain_*`, …) exist only in `en.json`.
- [ ] **1.18 Dashboard teaser (optional)** — top 3 apps today per child.
  → 2026-10-20: skipped for now (keep it lean); revisit after real use.

### 1E. Verification and release

- [x] **1.19 End-to-end on a real test machine**
  Prerequisites checked 2026-10-20:
  - `deploy/screenguard-agent.service` has no `CapabilityBoundingSet` (root
    keeps `CAP_SYS_PTRACE`, so it can read other users' `exe`/`environ`) and no
    `ProtectProc`/`ProcSubset`.
  - Its `ProtectHome=read-only` still lets the agent read the child's
    `.desktop` files and Steam manifests.

  → **Passed 2026-10-20** on screenguardTest (CachyOS, KDE Plasma 6 Wayland).
  The agent was built from this branch and installed as root with the real unit
  file, then paired with its code to a server running this build on the
  maintainer's PC (mDNS off). uid 1000 was linked to a profile with no limits.
  - **Capability:** the server log shows
    `capabilities=["web_filter", "app_usage"]` (1.3 ✔).
  - **Apps:** Firefox, KCalc and Dolphin were launched via KIO, KWrite via
    `gtk-launch`. After 80/140/200 s each app had 85/145/205 s; the 5 s offset is
    the launch just before T0 plus one tick.
  - **Closing and opening apps:** Dolphin, closed at +221 s, stayed at 225 s.
    Konsole, opened then, had 179 s 180 s later (it moves into its own
    `…/main.scope` sub-cgroup).
  - **Locked screen:** 60 s locked added 0 s. The first tick after unlocking
    adds up to one tick (±1 tick, as planned).
  - **Arithmetic:** Firefox total = 593 s wall − 60 s locked = 533 s, exactly as
    recorded.
  - **Server:** equals the agent's totals as of the last sync (60 s cadence);
    8 syncs arrived and none were rejected. The web UI page shows the real
    names and icons.
  - **Side observation (not this feature):** the screen-time counter
    (`usage_log`) showed 488 s for the same period, where app time was 533 s. It
    may undercount after lock/unlock or at session start; worth a separate look. — install the new agent,
  use apps for a while, compare reported minutes with a stopwatch (±1 tick).
  Also confirm the server log shows `app_usage` in the agent's capabilities (1.3).
- [ ] **1.20 Compatibility checks** — new server + old agent (no app data, no
  errors); old server + new agent (no disconnects, see 1.4); upgrade path keeps
  existing pairings and screen-time numbers identical.
  → Progress 2026-10-20, with `docs/spikes/sim-agent.py`, a simulated agent
  speaking the real protocol: pair via code, hello, user list,
  `app_usage_sync` ×3, then a heartbeat.
  - **Old server (0.10.17) + new agent:** the server logs `unknown message type
    'app_usage_sync'` at debug and keeps the connection; the following heartbeat
    gets its `remaining_update`. ✔
  - **New server + new agent:** the capability is stored, the totals grow, the
    resend doesn't double-count, and the reports API returns the data. ✔
  - **Upgrade path:** verified in 1.12 (SQLite + Postgres). ✔
  - **Still to do with a real old agent:** new server + the real 0.10.17
    agent, i.e. no app data and no errors. By code, the old agent never sends the
    message, and its hello sets `app_usage_available = false`, so the UI shows
    the "update the agent" notice.
- [x] **1.21 Docs** — README section "App usage reports" + privacy note
  (cmdlines never leave the machine); update this file.
  → Added a features bullet, a new "App usage reports" section (how it works,
  what counts, retention, the off switch) and `app_tracking` in the agent config
  example. In "What the agent does on your machine", added `app_usage_sync` to the
  uplink list and reworded the privacy paragraph: it used to say "no process or
  application names"; it now says exactly which app identifiers are sent, and
  that command lines, window titles, file names and URLs never are.
- [ ] **1.22 Release** — version bump commit, tag only with maintainer go-ahead.

**Exit criteria (Phase 1):**
- [ ] Reports correct on at least two desktops in real use for ≥ 1 week
- [ ] No regression in screen-time accounting (compare daily totals before/after)
- [ ] CPU cost within the Phase 0 target
- [ ] Shipped in a release

---

## Phase 2 — App groups, group limits, enforcement

> **Release decision (2026-10-20, maintainer):** no release after Phase 1 alone —
> Phase 1 + Phase 2 (including 2E new-app approval) ship together as one working
> feature. Step 1.22 is folded into 2.17.

### 2A. Model and admin UI

- [x] **2.0 App categories** — the agent sends each app's freedesktop
  `Categories=` (from its `.desktop` entry) in `AppUsageEntry.categories`
  (`#[serde(default)]`, so old servers/agents are unaffected); `steam:` and
  `minecraft:` ids get `Game` automatically. Server stores them in
  `app_catalog.categories`. *Done when:* catalogue + ingest tests; real-machine
  check that Firefox → `WebBrowser`, a game → `Game`.
  → `DesktopEntry.categories`, `app_ident::Shown { name, icon, categories }`, agent
  DB column `app_usage_log.categories` (added to old DBs in `migrate`),
  `AppUsageEntry.categories` (omitted when empty). Server migration **v11** adds
  `app_catalog.categories`, only if missing: `create_tables` runs before the
  migrations and already builds the final table on a v9 database. Categories are
  validated (plain words, ≤ 20) and returned by the reports API.
  - Real machine: the test machine's agent and the local server were updated (the live
    DB migrated 10 → 11). Firefox → `GNOME;GTK;Network;WebBrowser`, a test entry with
    `Categories=Game;ArcadeGame;` → `Game;ArcadeGame`, end to end.
- [x] **2.1 Matching rules in `common`** — `AppMatcher` shared by server and agent
  so they always agree. Rule kinds: `app_id` (exact), `app_id_prefix`
  (e.g. `steam:` = all Steam games), `category` (freedesktop category, e.g.
  `Game`, `WebBrowser`), and per-group **exclusions** (`app_id`) so a parent can
  take one app out of a category-based group. No regex rules (decided: keep it
  simple; revisit only if real use needs it).
  *Done when:* table-driven tests.
  → `common::apps::{RuleKind, GroupRule, group_matches}`: any include rule and
  no exclude rule; categories are case-sensitive; an empty prefix matches nothing.
  Serialised as `{"kind":"app_id|prefix|category","value":…,"exclude":bool}`.
- [x] **2.2 Migration v12** (v11 = categories, 2.0) —
  `app_groups(id, name, builtin, created_at)`,
  `app_group_members(group_id FK, kind, value, exclude)`,
  `app_group_limits(profile_id FK, group_id FK, day_of_week, allowed_minutes)` (0 = blocked),
  Groups are global (shared by all children); limits are per profile.
  **Predefined groups** created by the migration (maintainer's request: most
  parents don't know which games/browsers exist):
  - **Games** = category `Game` + prefixes `steam:`, `minecraft:`;
  - **Web browsers** = category `WebBrowser`.
  New apps of those categories join automatically. Parents can rename them,
  add/exclude apps, or delete them; no limit is set by default (they only group
  the reports until a parent sets a limit).
  → Done: the tables are in `APP_USAGE_DDL` (`app_groups(id, name, builtin, created_at)`,
  `app_group_rules(id, group_id, kind, value, exclude)`, `app_group_limits`), and
  `crates/server/src/app_groups.rs` has CRUD, rule validation and
  `seed_builtin`.
  - Seeding runs once (migration v12 or a fresh install), so a deleted predefined
    group stays deleted.
  - Adding the opposite rule (include ↔ exclude) for the same app replaces it.
  - Tests: SQLite fresh install, v9 → v12 upgrade, custom groups/rules, and the
    Postgres test extended (the `exclude` column works there).
- [x] **2.3 Groups API + "Apps" admin page** — one page listing every app seen
  on any computer (name, category, last seen, which groups it's in), with
  "add to group" / "remove"; a groups list (members, create/rename/delete);
  "ungrouped apps" filter to make grouping easy.
  → API (`api/apps.rs`): `GET /apps` (catalogue + groups each app is in,
  computed with `group_matches`), `GET/POST /app-groups`,
  `PATCH/DELETE /app-groups/{id}`, `POST /app-groups/{id}/rules`,
  `DELETE /app-groups/{id}/rules/{rule}`, `POST /app-groups/{id}/membership`.
  - Membership is "put in / take out" whatever the rules: it removes the opposite
    explicit rule, then adds an entry or an exclusion if still needed.
  - Web UI `/apps` (new nav item "Apps"): group cards (includes/except chips
    with ×, "+ Add category" from categories seen, rename, delete) and an app
    list (source/category, last seen, group chips with × to take out, "Add to
    group"). No JavaScript needed. Predefined group names are translated unless
    renamed. All 6 languages.
  - Verified against the live local server with the test machine's real
    catalogue: Firefox auto-in Web browsers, the test game auto-in Games; take
    out → "Except", add → entry, create group; screenshots at desktop and phone
    width and in Polish.
- [x] **2.4 Limits API + UI** — per profile: per group, per weekday minutes, same
  editor style as daily limits; "blocked" toggle = 0.
  → `GET /profiles/{id}/app-group-limits` (every group with this profile's
  limits) and `PUT /profiles/{id}/app-group-limits/{group}` (replace; empty = no
  limit; validated 0–1440, weekday 0–6, no duplicates). Saving bumps the config
  version, so agents get a `config_push`.
  - Profile page: "App limits" card. Simpler than the daily-limits editor: per
    group, seven fields that take `1:30` or `90`, empty = no limit, 0 = blocked
    (shown in red). No JavaScript; all 6 languages.
  - Verified against the live server: a weekday/weekend/blocked pattern saved
    and read back; an invalid `25:00` was rejected with nothing changed.
- [x] **2.5 Reports by group** — the Phase 1 reports gain a "by group" view.
  → `GET /profiles/{id}/app-usage` returns `groups: [{id, name, builtin, seconds,
  limit_minutes}]` (an app in two groups counts in both; `limit_minutes` only
  in the day view, for that weekday). The App usage page has a Groups card with
  a bar per group: progress toward the limit when there is one, red when over.
  Verified on the live server.

### 2B. Config delivery and remaining time

> **Design change (2026-10-20):** group time can't be summed from per-app totals —
> two browsers open at once would count double. Group time = wall time while
> *any* member app runs, which only the agent sees per tick. So the agent
> accounts groups itself and computes remaining group time locally.

- [x] **2.6 `UserConfig.app_groups`** (`#[serde(default)]`): *all* groups with
  `id`, `name`, `builtin`, `rules`, and this profile's `daily_limits` (empty =
  no limit). Bump the config version of every profile when groups/rules change
  (and of the profile when its group limits change, already done in 2.4) so
  `config_push` is sent.
  → `common::models::AppGroupConfig { id, name, builtin, rules, daily_limits }`;
  `UserConfig.app_groups` is omitted when empty, so the wire is unchanged
  without groups. `build_config_push` fills it; every group/rule/membership
  change calls `propagate_all` (bump + push for every profile). The agent caches
  the groups as JSON per user (`cached_app_groups`, cleared with
  `managed_users` and on rebind). Tests on both sides.
- [x] **2.7 Group accounting + sync** — agent: per tick, each group with ≥ 1
  running member app gets the tick's seconds once (same counting rule as apps);
  stored in `group_usage_log(local_uid, date, group_id, seconds, synced)`; sent
  in `AppUsageSync.groups` (`#[serde(default)]`). Server: migration v13
  `app_group_usage(agent_user_id, date, group_id, seconds)` with MAX upsert;
  `RemainingEntry.group_usage_elsewhere: [{group_id, used_seconds}]` = today's
  group time on the profile's *other* agents (`#[serde(default)]`). Reports by
  group use these totals (fallback: sum of member apps for older days).
  → Agent: `AppAccounting::seconds()` returns seconds per user, and the tracker
  gives them to each running app and to each group with a running member
  (categories from 2.0). Then `group_usage_log`, `AppUsageSync.groups` (with
  the first message, or alone). Server: v13 `app_group_usage` (MAX upsert, only
  for existing groups), `group_usage_elsewhere()` in `RemainingEntry`, and
  `group_totals()` in the reports. Tests on both sides, including "two
  browsers at once count once".
  - **Upgrade bug found on the real machine and fixed:** an upgraded agent
    reported its old config version, so the server never re-sent the config,
    and the groups the old agent had ignored never arrived. Now the agent stores
    `CONFIG_FORMAT` (2) with the config and reports version 0 on hello when its
    config is in an older format, so any server sends the full config. Bump
    `CONFIG_FORMAT` whenever the agent starts storing a new part of `UserConfig`.
  - Real machine: two `Game` apps open for 60 s → each app +60 s, the Games
    group +60 s (not 120); it reached the server and the report.
- [x] **2.8 Agent computes group remaining** — `limit_today + adjustments −
  (elsewhere + local today)` per group; the last `elsewhere` and the config are
  cached in the agent DB, so the same code runs offline.
  *Done when:* unit tests for online, offline and the day boundary.
  → `app_limits::group_status()`: a pure function (blocked days stay blocked even
  with extra time; negative adjustments floor at 0); `cached_group_elsewhere`
  (dated: a new day starts at 0) is filled from each `remaining_update`. The
  heartbeat loop recomputes `group_status` every app tick and logs when a group
  runs out; predefined group names are localized for the child
  (`i18n::group_name`).
  - Real machine: Games limited to 3 min today with 60 s already used, then
    the test game ran → "app group 'Games' has no time left today" logged at
    exactly 180 s.

### 2C. Enforcement

- [ ] **2.9 Warnings** — notify at the profile's warning thresholds per group
  ("Games: 5 minutes left"), once per threshold per day; localized.
- [ ] **2.10 Closing apps** — at 0: SIGTERM matched processes → grace (~10 s) →
  `cgroup.kill` on the app scope → fallback SIGKILL of the process tree.
  Strictly obey the safety rules in [Enforcement](#enforcement-agent-phase-2).
  *Done when:* integration test with dummy processes in a test cgroup (or a
  container) proves only matched processes die.
- [ ] **2.11 Relaunch handling** — killed-again on next tick + notification,
  rate-limited (no notification spam).
- [ ] **2.12 Blocked groups (0 min)** — apps are closed immediately on start with
  a "not allowed" notification.
- [ ] **2.13 Interaction with the overall limit** — group time also counts toward
  daily screen time (it already does, since it's session time); when the overall
  limit is hit the existing lock logic wins. Verify no double-locking/kill races.
- [ ] **2.14 Extra time for a group** — "+15 min Games today" from the profile
  page (adjustments per group), mirroring existing time adjustments.

### 2D. Verification and release

- [ ] **2.15 Real-machine tests** on every supported desktop: limit hit → warning
  → app closes → relaunch closed → other apps unaffected → overall limit still works.
- [ ] **2.16 Compatibility** — old agent ignores `app_groups` (serde default) and
  simply doesn't enforce; UI shows which agents can enforce (capability).
- [ ] **2.17 Docs + release** (includes Phase 1; version bump commit, tag only
  with the maintainer's go-ahead).

### 2E. New-app approval ("ask a parent") — maintainer's idea, 2026-10-20

Like Google Family Link's install approval. What is and isn't possible on Linux:

- **System-wide installs already need an admin**: pacman/apt/dnf need root;
  GNOME Software / KDE Discover go through PackageKit and system Flatpak through
  polkit, which ask for an *admin* password. A child account that is **not in
  `wheel`/`sudo`** cannot install system software at all.
- **What a child can still add without root:** `flatpak --user` apps, Steam
  games, AppImages / downloaded programs, Minecraft launchers/mods, browser web apps.
  Those can't be stopped at install time without breaking the home directory,
  but they can be stopped **at first launch** — which is what matters.

Design: approval at first launch, using the Phase 2 closing machinery.

- [ ] **2.18 Admin-rights check** — agent reports whether each managed account is
  in `wheel`/`sudo`/`admin` (or has a sudoers rule); the UI warns "this child can
  install software as administrator" with how to fix it.
- [ ] **2.19 Approval mode per profile** — off (default) / "new apps need
  approval". Turning it on approves every app already seen for that child, so
  nothing existing breaks.
- [ ] **2.20 Request flow** — an app not yet approved is closed on start (≤ 5 s,
  same safety rules as 2.10) with a notification "Asked your parent for
  permission"; the agent sends `app_request { local_uid, app_id, name, icon }`
  (once per app until answered); the server stores pending requests.
- [ ] **2.21 Approve / deny UI** — requests badge on the dashboard and profile;
  approve (optionally straight into a group) or deny; the decision goes down in
  the profile config, the child gets a notification.
- [ ] **2.22 Offline behaviour** — while the server is unreachable, unapproved
  apps stay blocked; requests are queued and sent on reconnect.

Limitation to state in the UI/README: only things the agent can identify as an
app are covered (menu entries, Flatpak, Snap, Steam, Minecraft, Wine). A program
started from a terminal counts as the terminal — block the terminal (a group with
0 minutes) if that matters.

**Exit criteria (Phase 2):**
- [ ] A Games group limit is enforced reliably on all supported desktops
- [ ] No unrelated process ever killed in testing (review agent logs)
- [ ] Shipped in a release

---

## Phase 3 — Refinements (pick individually)

- [ ] **3.1 Foreground-only accounting** via `screenguard-tray` reporting the
  focused app over the existing tray↔agent D-Bus link: X11 first, then KWin
  script, then GNOME Shell extension. Agent falls back to v1 when no report.
- [ ] **3.2 Block at launch** with `fanotify` `FAN_OPEN_EXEC_PERM` for blocked
  groups (deny exec for the child's UID) — careful with over-blocking (`java`).
- [ ] **3.3 Web-domain groups** via the DNS web filter (count time per domain
  group, e.g. "YouTube") — separate design doc first.
- [ ] **3.4 Allow-list mode** — superseded by 2E (new-app approval).
- [ ] **3.5 Weekly limits** for groups (in addition to per-day).

---

## Open questions

| # | Question | Answer (with date) |
|---|---|---|
| Q1 | Does Cinnamon (and XFCE) create per-app `app-*.scope` units? If not, how good is the exe-keyed fallback? | 2026-10-20: no evidence they do (upstream GLib doesn't create scopes; only Fedora/RHEL patch it). Design no longer depends on it: GIO env (Cinnamon/MATE) and process tree (XFCE etc.) cover them. |
| Q2 | How do Steam games, Minecraft Java and Proton games appear (scope, exe, cmdline)? | 2026-10-20: Steam — `reaper SteamLaunch AppId=<n> -- …` ancestor (native + Proton). Minecraft — `java` with a Minecraft main class/jar or mod loader (`net.minecraft`, `cpw.mods`, `fabricmc`, `quiltmc`); Prism/MultiMC start `org.prismlauncher.EntryPoint` / `org.multimc.EntryPoint` and pass the main class on stdin, so those entry points are markers too. Flatpak launchers are `flatpak:` ids (their whole sandbox counts as the launcher app; acceptable for v1). Covered by tests in 1.6. |
| Q3 | Which scan interval keeps CPU ≤ 1 % on the slowest machine? | 2026-10-20: 5 s — ≤ 0.11 % of one core even with full reads; ~0.04 % with cached identities (see 0.6). |
| Q4 | Does an old (0.10.x) server tolerate unknown WS message types? (step 1.4) | 2026-10-20: yes — catch-all arm logs at debug since `204a3be`; handler errors never close the socket. |
| Q5 | Should group limits be per profile only, or allow a global default per group? | |
| Q6 | Count apps launched from a terminal as the terminal (inherited scope) or try to attribute to the child process's exe? | 2026-10-20: v1 counts them as the terminal (the launch root) — simple and consistent across desktops; revisit if needed. |

---

## Decision log

| Date | Decision | Why |
|---|---|---|
| 2026-10-20 | Identify apps via systemd cgroup scopes first, exe name last | Scopes are set by the desktop launcher, are hard to spoof by renaming, and separate apps from desktop internals |
| 2026-10-20 | v1 counts "running while session active", foreground later | Works on every desktop without user-session helpers; matches Timekpr PlayTime |
| 2026-10-20 | Cmdlines never sent to the server | Privacy — may contain URLs, file names, tokens |
| 2026-10-20 | Agent syncs per-day totals; server upserts with MAX | Retries/reconnects can't double-count |
| 2026-10-20 | Phase 1 is observe-only | Validate detection quality on real machines before anything gets killed |
| 2026-10-20 | Spike finding: only units named `app-…` under `app.slice` are apps; other units there are user services (`syncthing.service`, `dconf.service`, portals, `dbus-:1.x-…@0.service`) → ignore | Seen on Hyprland and KDE |
| 2026-10-20 | Spike finding: unit formats differ — KDE: `app-<DesktopID>@<uuid>.service`; Hyprland/uwsm: `app-Hyprland-<cmd or id>-<rand>.scope`; D-Bus/xdg activation: `app-<reverse.dns.id>-<pid>.scope`; XDG autostart: `app-<id>@autostart.service`. Parser must handle all; ids are systemd-escaped (`\x2d`) | Per systemd's desktop-environment naming convention, confirmed in samples |
| 2026-10-20 | Resolution chain: unit id → `<id>.desktop` (case-insensitive) → exe → `.desktop` whose `Exec=` binary matches → `scope:<id>` → `exe:<name>`. When several `.desktop` files share a binary, prefer the one whose id equals the binary / has no extra args (seen: `foot` matched `foot-server`) | Same app got two different unit names (Chromium); exe matching merges them |
| 2026-10-20 | **Don't sample every desktop.** Identification is designed from documented signals (systemd convention, Flatpak/Snap scopes, GIO launch env, process tree) with fallbacks so it works on any desktop; real machines only sanity-check it | Maintainer can't provide every desktop/distro combination |
| 2026-10-20 | "Is it an app" = resolves to a visible `.desktop` launcher entry and isn't autostart/session/background — no per-desktop ignore lists | Desktop components lack visible `.desktop` entries, so they drop out on any desktop |
| 2026-10-20 | Autostart units (`@autostart.service`, tray apps like Nextcloud, Bitwarden, screenguard-tray) are tracked but **not counted as usage by default**; enforcement still applies to them if they're in a limited group | Background tray apps would otherwise accumulate "usage" all day |
| 2026-10-20 | Strip a trailing ` (deleted)` from `/proc/<pid>/exe` targets | Binary updated while running (seen: screenguard-tray) |
| 2026-10-20 | The unit is the **first** `.scope`/`.service` below the slice; deeper cgroups belong to it | Only slices nest in systemd; Konsole creates `main.scope`/tab sub-cgroups inside its scope |
| 2026-10-20 | Ignore the `org.chromium.Chromium` self-scope id; use the children's unit or the process tree | Chromium and every Electron app use this id (Bitwarden landed in it) |
| 2026-10-20 | GIO env is checked before the app unit, but only with a live `_PID` ancestor owned by the child | Nested GIO launches inherit the parent app's unit; a stale env must not misattribute |
| 2026-10-20 | Interpreter processes (Electron, Python, Java, …) are identified by the script/archive in `argv` | Wrapper scripts `exec electron /usr/lib/<app>/app.asar` — the exe says nothing |
| 2026-10-20 | Scan every 5 s, identities cached by `(pid, start time)`, `.desktop` catalogue reloaded on dir mtime change | Measured ≤ 0.11 % CPU full, ~0.04 % cached (0.6); start time guards against PID reuse |
| 2026-10-20 | The agent sends `app_usage_sync` to any server, no version gate | Every server version ignores unknown message types (Q4); the payload is small and sent at most once a minute |
| 2026-10-20 | The agent re-requests config when its cached config predates a new `CONFIG_FORMAT` | Otherwise upgraded agents keep config without the new parts until a setting changes (seen on the test machine) |
| 2026-10-20 | Group time is accounted on the agent (union per tick) and remaining group time is computed there from `limit − (other computers + local)` | Summing member apps double-counts overlapping apps; the agent sees exact local time live and the same code works offline |
| 2026-10-20 | Phase 1 + 2 ship in one release | Maintainer wants a full working solution (see apps, group, limit, close) before release |
| 2026-10-20 | Predefined "Games" and "Web browsers" groups from freedesktop categories (`Game`, `WebBrowser`) + `steam:`/`minecraft:` | Parents don't know game/browser names; categories are in every `.desktop` file and auto-include new apps |
| 2026-10-20 | No regex match rules in Phase 2 | Keep it simple; ids, prefixes, categories and exclusions cover the known needs |
| 2026-10-20 | New-app approval = approval at first launch (2E), not install blocking | System installs already need an admin; user-level installs (Flatpak --user, Steam, AppImage) can't be blocked at install, but can at launch |
| 2026-10-20 | `session.slice` isn't excluded, only `background.slice` and non-`app-` user services | uwsm compositor keybinding launches without `uwsm app` stay in the compositor's unit |

---

## Session log

| Date | Session summary | Next |
|---|---|---|
| 2026-10-20 | Concept agreed, plan written (this file) | Phase 0, step 0.1 |
| 2026-10-20 | 0.1 done (scan script). Samples: Hyprland (maintainer's machine), KDE (screenguardTest, sparse). Findings logged in decision log | Richer KDE sample; GNOME + Cinnamon access |
| 2026-10-20 | Switched Phase 0 from sampling desktops to research: signal matrix, desktop-independent algorithm and app filter written (0.2–0.4) | 0.5 sanity check on Hyprland + KDE |
| 2026-10-20 | 0.5 done: full algorithm in `app-scan.py`; Hyprland + KDE (KIO and GIO launches) + simulated GIO/Steam/Minecraft/Wine all correct after 5 algorithm fixes (see Sanity check results) | 0.6 measure cost |
| 2026-10-20 | Trimmed the Hyprland sample (privacy). 0.6 done: Rust benchmark on Hyprland laptop and the KDE VM, 5 s interval with cached identities. **Phase 0 complete** | Phase 1, step 1.1 |
| 2026-10-20 | 1.1 done: `common::apps::AppId` (parse/display/validate, 3 tests) | 1.2 wire types |
| 2026-10-20 | 1.2 done: `AppUsageSync` / `AppUsageEntry` + round-trip test | 1.3 capability, 1.4 old-server check |
| 2026-10-20 | 1.3 done (`app_usage` capability when cgroup v2; real log check moved into 1.19). 1.4 done by code inspection: unknown types ignored since the first server | 1.5 process enumeration |
| 2026-10-20 | 1.5 done: `app_scan::ProcScanner` with (pid, start, comm) cache + fake `/proc` fixture, 5 tests | 1.6 identification |
| 2026-10-20 | 1.6 done: `app_catalog.rs` + `app_ident.rs`, 13 fixture tests; Steam-vs-Wine precedence fix; Prism/MultiMC markers (Q2 answered) | 1.7 localized names + catalogue reload |
| 2026-10-20 | 1.7 done: `name_for(lang)`, `CatalogCache` (mtime signature, 30 s), `display()` for Steam/Snap/Flatpak/Minecraft/Wine | 1.8 accounting loop |
| 2026-10-20 | 1.8 done: `AppAccounting` (fake-clock tests: exact totals, carry, 2× cap) + `AppTracker`; `AppId` now `Ord` | 1.9 storage + wiring |
| 2026-10-20 | 1.9 done: `app_usage_log` table + DB functions (2 tests), tracker wired into `HeartbeatLoop` (5 s), prune > 35 days; clippy clean on new code | 1.10 sync |
| 2026-10-20 | 1.10 done: `send_app_usage_sync` (60 s + on connect with 2-day resend), test | 1.11 config switch |
| 2026-10-20 | 1.11 done: `app_tracking` config (default on), capability follows it. Agent side (1A–1B) complete | 1.12 server migration |
| 2026-10-20 | 1.12–1.14 done: v10 (`app_catalog`, `app_usage`, `agents.app_usage_available`), upgrade from 0.10.17 verified on SQLite + Postgres; `app_usage::ingest` with MAX upsert (tests on both DBs); capability stored and in agents API | 1.15 reports API |
| 2026-10-20 | 1.15 done: `GET /profiles/{id}/app-usage` (day/week/total), tests on SQLite + Postgres | 1.16 web UI |
| 2026-10-20 | 1.16 done: `/profiles/<id>/apps` page (day/week/total), links from the profile page, checked locally with screenshots | 1.17 translations |
| 2026-10-20 | 1.17 done: de/es/fr/pt/pl, profile header wraps on phones; noted 17 untranslated web-filter keys (older gap) | 1.18 optional / 1.19 e2e |
| 2026-10-20 | 1.18 skipped (optional). 1.20 mostly done via `sim-agent.py` (old server ignores the message, new server ingests); real old agent pending | 1.21 docs, then 1.19 with maintainer |
| 2026-10-20 | 1.21 done: README section + corrected privacy statement | 1.19 real-machine e2e (maintainer), 1.22 release |
| 2026-10-20 | Review follow-ups: unit hardening OK for 1.19, ppid refreshed per tick, full tick measured (0.12 % CPU), README desktop claim limited to what was tested | 1.19 (maintainer) |
| 2026-10-20 | 1.19 passed on screenguardTest against a local server: per-app seconds exact, lock = 0 s, server matches agent, UI OK; noted possible screen-time undercount (separate issue) | 1.20 real old agent, 1.22 release |
| 2026-10-20 | Found and fixed a screen-time undercount (~10 %, heartbeat dropped the remainder) — verified on the test machine. Plan revised: release after Phase 2, predefined Games/Web browsers groups via categories, 2E new-app approval | Phase 2.0 |
| 2026-10-20 | 2.0 done: categories agent → server (v11), verified on the test machine | 2.1 matcher, 2.2 v12 groups |
