# Feature — per-app usage reports, app groups and group limits

- **Status:** In progress — Phase 0 (detection spike)
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

- **Phase:** 0 — Detection spike
- **Last completed step:** 0.1 (scan script `docs/spikes/app-scan.py`)
- **Next step:** 0.2 — richer KDE sample (open browser, a Flatpak, Steam/Minecraft on
  screenguardTest), then GNOME and Cinnamon machines (need access to them)
- **In progress / half-done:** 0.2 — Hyprland sample done (extra, not in the list),
  KDE sample taken but with only one app open
- **Blockers / open questions:** GNOME + Cinnamon test machines needed; see
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

## Phase 0 — Detection spike (no product code)

Purpose: prove the cgroup approach on real desktops **before** designing tables.
Output is knowledge + a throwaway script, recorded in this doc.

- [x] **0.1 Write a read-only scan script** (`docs/spikes/app-scan.sh` or a small
  Python file, kept out of the build). For a given UID: list processes, their
  `/proc/<pid>/cgroup` path, `exe`, `comm`, short cmdline, and the derived
  candidate `app_id`.
  *Done when:* it runs as root on a test machine and prints one line per process.
- [ ] **0.2 Run it on every desktop we support**, with a child account logged in
  and typical apps open (browser, a Flatpak, a Steam game or Minecraft, a terminal
  app, file manager):
  - [ ] GNOME (Wayland)
  - [ ] KDE Plasma (Wayland) — first sample `docs/spikes/kde-cachyos.txt` (only Alacritty open; rerun with more apps)
  - [ ] Cinnamon (X11)
  - [ ] (optional) XFCE
  - [x] (extra) Hyprland via uwsm — `docs/spikes/hyprland-arch.txt`
  *Done when:* raw output for each is saved under `docs/spikes/` (redact cmdlines).
- [ ] **0.3 Record findings in [Decision log](#decision-log):** which desktops
  create `app-*.scope` per app; scope naming format per desktop; how Steam games,
  Minecraft (Java) and Wine/Proton look; what lands in `session.slice` /
  `background.slice`; what apps launched from a terminal look like (they inherit
  the terminal's scope).
- [ ] **0.4 Decide fallback** for desktops without per-app scopes (ignore list +
  exe keying) and write the initial **ignore list** (desktop shells, panels,
  portals, pipewire, gvfs, …) into the decision log.
- [ ] **0.5 Measure cost:** time one full scan of `/proc` on the slowest test
  machine; pick the scan interval (target ≤ 1 % CPU; expected 5 s).

**Exit criteria (Phase 0):**
- [ ] Identification rules confirmed or adjusted for every supported desktop
- [ ] Fallback strategy + ignore list decided
- [ ] Scan interval decided
- [ ] [Open questions](#open-questions) Q1–Q3 answered

---

## Phase 1 — Per-app usage reports (observe only, no enforcement)

### 1A. Shared types (`crates/common`)

- [ ] **1.1 `AppId` helpers** — `crates/common/src/apps.rs`: the `app_id` string
  format (`desktop:` / `flatpak:` / `snap:` / `steam:` / `minecraft:` / `exe:`),
  parse/validate, max length.
  *Done when:* unit tests cover each prefix and reject malformed ids.
- [ ] **1.2 Wire types** — in `messages.rs`: `MSG_APP_USAGE_SYNC`,
  `AppUsageSync { entries: Vec<AppUsageEntry> }`,
  `AppUsageEntry { local_uid, date, app_id, display_name, icon: Option<String>, seconds_total }`.
  *Done when:* serde round-trip test passes.

### 1B. Agent — scanner and accounting (`crates/agent`)

- [ ] **1.3 Capability** — advertise `"app_usage"` in `agent_hello.capabilities`
  (Linux with cgroup v2 present). *Done when:* server log shows it on connect.
- [ ] **1.4 Check unknown-message handling on old servers** — confirm a 0.10.x
  server ignores an unknown WS message type without dropping the connection.
  Record result in decision log; if it drops, gate sending on server version.
- [ ] **1.5 `app_scan.rs` — process enumeration**: iterate `/proc`, keep PIDs whose
  real UID is a managed child, read cgroup path, `exe` basename, cmdline (local only).
  *Done when:* unit tests run against a fake `/proc` tree fixture (tempdir).
- [ ] **1.6 Identification** — implement the rules from Phase 0 (scope name →
  desktop id, flatpak, snap, Steam AppId, Minecraft Java, Wine `.exe`, exe fallback)
  and the ignore list. *Done when:* table-driven tests using the real samples
  captured in Phase 0 produce the expected `app_id`s.
- [ ] **1.7 `.desktop` lookup** — map desktop ids to `Name=` (+ localized
  `Name[xx]=` if the profile language matches) and `Icon=` from
  `/usr/share/applications`, `~/.local/share/applications`, flatpak/snap export
  dirs. Cache, refresh on miss. *Done when:* tests with fixture `.desktop` files pass.
- [ ] **1.8 Accounting loop** — every scan tick, if the user's session counts
  (reuse `SessionUsageState` logic from `dbus.rs`), add the tick's seconds to each
  running `app_id` once. Handle clock jumps / suspend (cap a tick at 2× interval).
  *Done when:* unit test with a fake clock + fake scan results gives exact totals.
- [ ] **1.9 Local storage** — agent DB table
  `app_usage_log(local_uid, date, app_id, display_name, icon, seconds, PRIMARY KEY(local_uid,date,app_id))`;
  midnight rollover like `usage_log`; prune > 35 days.
  *Done when:* DB tests pass; restarting the agent keeps today's totals.
- [ ] **1.10 Sync** — send today's (and any unsynced past days') totals via
  `MSG_APP_USAGE_SYNC` on connect and every heartbeat N (e.g. every 60 s);
  only when connected; never block the existing heartbeat.
  *Done when:* simulated server receives monotonically growing totals; reconnect
  re-sends totals without double counting server-side.
- [ ] **1.11 Feature switch** — agent config `app_tracking = true|false`
  (default true) so it can be disabled per machine without a rebuild.

### 1C. Server — storage, ingestion, reports (`crates/server`)

- [ ] **1.12 Migration v10** — tables:
  - `app_catalog(app_id PK, display_name, icon, first_seen, last_seen)`
  - `app_usage(agent_user_id FK, date, app_id, seconds, PRIMARY KEY(agent_user_id, date, app_id))`
  Portable SQL only. *Done when:* migration tests on SQLite pass and an
  upgrade from 0.10.17 is verified on SQLite **and** Postgres (docker).
- [ ] **1.13 Ingestion** — handle `MSG_APP_USAGE_SYNC` in `ws.rs`: upsert
  `app_usage` with `seconds = MAX(existing, reported)` (totals are monotonic per
  day), upsert catalog name/icon/last_seen. Reject ids failing `AppId` validation.
  *Done when:* tests cover retry (same total twice), growth, and bad ids.
- [ ] **1.14 Capability storage** — store `"app_usage"` capability per agent
  (like `web_filter_available`), expose in agent API.
- [ ] **1.15 Reports API** — `GET /api/v1/profiles/{id}/app-usage?range=day|week|total&date=YYYY-MM-DD`
  → per app: display name, icon, seconds; summed across all agents of the profile
  (profile-level, like screen time). Also per-day series for the week view.
  *Done when:* API tests with seeded data return correct sums and ordering.

### 1D. Web UI (`webui/`)

- [ ] **1.16 Profile → "App usage" tab/page** — range switch (day / week / total),
  date picker, table sorted by time with bar per app; empty state; notice when no
  agent of the profile has the `app_usage` capability ("update the agent").
- [ ] **1.17 Translations** — all new strings in all 6 languages
  (`en, de, es, fr, pt, pl`).
- [ ] **1.18 Dashboard teaser (optional)** — top 3 apps today per child.

### 1E. Verification and release

- [ ] **1.19 End-to-end on a real test machine** — install the new agent,
  use apps for a while, compare reported minutes with a stopwatch (±1 tick).
- [ ] **1.20 Compatibility checks** — new server + old agent (no app data, no
  errors); old server + new agent (no disconnects, see 1.4); upgrade path keeps
  existing pairings and screen-time numbers identical.
- [ ] **1.21 Docs** — README section "App usage reports" + privacy note
  (cmdlines never leave the machine); update this file.
- [ ] **1.22 Release** — version bump commit, tag only with maintainer go-ahead.

**Exit criteria (Phase 1):**
- [ ] Reports correct on at least two desktops in real use for ≥ 1 week
- [ ] No regression in screen-time accounting (compare daily totals before/after)
- [ ] CPU cost within the Phase 0 target
- [ ] Shipped in a release

---

## Phase 2 — App groups, group limits, enforcement

### 2A. Model and admin UI

- [ ] **2.1 Matching rules in `common`** — `AppMatcher` shared by server and agent
  so they always agree: rule kinds `app_id` (exact), `app_id_prefix`
  (e.g. `steam:` = all Steam games), and `exe_regex` / `cmdline_regex`
  (evaluated **only on the agent**, see privacy rule).
  *Done when:* table-driven tests; regexes are size-limited and pre-compiled.
- [ ] **2.2 Migration v11** —
  `app_groups(id, name, created_at)`,
  `app_group_members(group_id FK, kind, value)`,
  `app_group_limits(profile_id FK, group_id FK, day_of_week, allowed_minutes)` (0 = blocked).
  Groups are global (shared by all children); limits are per profile.
- [ ] **2.3 Groups API + UI** — CRUD groups; add members from the catalog
  (pick seen apps) or as a rule; show "ungrouped apps" to make grouping easy.
- [ ] **2.4 Limits API + UI** — per profile: per group, per weekday minutes, same
  editor style as daily limits; "blocked" toggle = 0.
- [ ] **2.5 Reports by group** — the Phase 1 reports gain a "by group" view.

### 2B. Config delivery and remaining time

- [ ] **2.6 `UserConfig.app_groups`** (`#[serde(default)]`): for each group with a
  limit today: group id, name, rules, `allowed_minutes` per weekday. Bump config
  version when groups/limits change so `config_push` is sent.
- [ ] **2.7 Server-side group remaining** — in `remaining.rs`: per group,
  `allowed − used today across all agents of the profile` (+ future adjustments);
  add `group_remaining: Vec<GroupRemaining>` to `RemainingEntry` (`#[serde(default)]`).
- [ ] **2.8 Agent caches** groups + limits + last server remaining in its DB,
  and evaluates offline from local `app_usage_log` (same pattern as offline
  screen-time evaluation in `enforcement.rs`).
  *Done when:* unit tests for online (server value) and offline (local) paths.

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
- [ ] **2.17 Docs + release.**

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
- [ ] **3.4 Allow-list mode** — "only approved apps may run" for a profile.
- [ ] **3.5 Weekly limits** for groups (in addition to per-day).

---

## Open questions

| # | Question | Answer (with date) |
|---|---|---|
| Q1 | Does Cinnamon (and XFCE) create per-app `app-*.scope` units? If not, how good is the exe-keyed fallback? | |
| Q2 | How do Steam games, Minecraft Java and Proton games appear (scope, exe, cmdline)? | |
| Q3 | Which scan interval keeps CPU ≤ 1 % on the slowest machine? | |
| Q4 | Does an old (0.10.x) server tolerate unknown WS message types? (step 1.4) | |
| Q5 | Should group limits be per profile only, or allow a global default per group? | |
| Q6 | Count apps launched from a terminal as the terminal (inherited scope) or try to attribute to the child process's exe? | Confirmed on KDE + Hyprland that terminal children share the terminal's unit. Decision pending. |

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
| 2026-10-20 | Autostart units (`@autostart.service`, tray apps like Nextcloud, Bitwarden, screenguard-tray) are tracked but **not counted as usage by default**; enforcement still applies to them if they're in a limited group | Background tray apps would otherwise accumulate "usage" all day |
| 2026-10-20 | Strip a trailing ` (deleted)` from `/proc/<pid>/exe` targets | Binary updated while running (seen: screenguard-tray) |

---

## Session log

| Date | Session summary | Next |
|---|---|---|
| 2026-10-20 | Concept agreed, plan written (this file) | Phase 0, step 0.1 |
| 2026-10-20 | 0.1 done (scan script). Samples: Hyprland (maintainer's machine), KDE (screenguardTest, sparse). Findings logged in decision log | Richer KDE sample; GNOME + Cinnamon access |
