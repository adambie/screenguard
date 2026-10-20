# ScreenGuard

Parental-control screen-time manager for Linux. A lightweight server+agent system that enforces daily screen-time limits and schedules on managed machines.

**Features**

- Daily time limits and weekly schedules per user profile
- Hard enforcement — locks all graphical sessions when time runs out
- Optional session preservation keeps applications and unsaved work running behind the lock screen
- Desktop notifications: warns at 15 / 10 / 5 / 1 minute before lockout
- Admin can send arbitrary text messages to any managed user's desktop
- Time adjustments (add or remove minutes) with optional reason shown to the user
- App usage reports — which apps each child used and for how long (day / week / total)
- App groups with daily limits — e.g. 1 hour of games on school days; ready-made "Games" and "Web browsers" groups fill themselves
- New-app approval — a new app is closed on start until a parent allows it
- mDNS auto-discovery — agents find the server on the local network without manual configuration
- Hosted cloud accounts — manage your machines without running a server yourself
- Web UI for administration
- Static binaries, no runtime dependencies

## Cloud accounts

**There is now a hosted ScreenGuard server at [screenguard.cc](https://screenguard.cc)** — a multi-account instance, so you can try ScreenGuard out without setting up a server of your own.

Create an account there, then install the agent on the machine you want to manage and point it at that account:

```bash
curl -fsSL https://github.com/adambie/screenguard/releases/latest/download/install.sh \
  | sudo bash -s -- --cloud-account=you@example.com
```

The machine shows up as *pending* under your account; approve it and it is managed. That is the whole setup — no server host to keep running, no fixed IP, no firewall rule.

The mobile app signs in to the same account, so the devices are reachable from anywhere — no VPN back to the home network and no port forwarding. You can change a limit or lock a machine while you are out.

Each account is isolated: its own profiles, devices, and usage history. Self-hosting on the LAN works exactly as before and is unaffected — see [Cloud mode](#cloud-mode) for the agent-side configuration details.

The agent is the same binary either way, and it sends and accepts exactly the same things — [What the agent does on your machine](#what-the-agent-does-on-your-machine) lists all of them.

## Mobile app

A Flutter Android app lives in the [screenguard-mobile](https://github.com/adambie/screenguard-mobile) repository. It lets you manage profiles, devices, schedules, and daily limits from your phone — no browser needed.

**Download:** grab `screenguard-android-<version>.apk` from the [latest mobile release](https://github.com/adambie/screenguard-mobile/releases/latest) and install it (enable *Install unknown apps* in Android settings first).

The app connects directly to the same REST API as the web UI. No extra setup needed on the server.

<p align="center">
  <img src="docs/screenshots/mobile-profiles.png" width="30%" alt="Profiles dashboard" />
  &nbsp;&nbsp;
  <img src="docs/screenshots/mobile-profile-detail.png" width="30%" alt="Profile detail with usage chart" />
  &nbsp;&nbsp;
  <img src="docs/screenshots/mobile-devices.png" width="30%" alt="Devices list" />
</p>

See the [screenguard-mobile repo](https://github.com/adambie/screenguard-mobile) for full details and build instructions.

## What the agent does on your machine

The agent behaves identically whether it reports to a server on your own LAN or to the cloud instance — only the address it connects to changes. Agent and protocol both live in this repository, so none of the following has to be taken on trust.

**What the agent sends up** — the complete uplink (`crates/common/src/messages.rs`):

- `agent_hello` — machine ID, hostname, timezone, agent version
- `user_list_update` — UID, username and display name of each local account above `min_uid`
- `heartbeat` — per UID: seconds of active time since the last beat, an idle flag, the number of open sessions
- `usage_sync` — per UID and date: total seconds used
- `app_usage_sync` — per UID, date and app: the app's identifier, its menu name, icon name and menu categories, and total seconds; per app group: total seconds (see [App usage reports](#app-usage-reports); off with `app_tracking = false`)
- `app_request` — per UID: the identifier, name and icon of an app waiting for a parent's approval (only with new-app approval on)

That is all of it. App usage only names apps the child could start from the desktop menu (by their `.desktop` entry, Flatpak/Snap ID or Steam game ID) — never command lines, window titles, file names or URLs, which stay on the machine. No keystrokes, no screenshots, no file contents. Optional domain blocking runs entirely on the machine: a local DNS proxy answers blocked lookups with NXDOMAIN. The blocklist is pushed down from the server; the queries the proxy sees are never reported back up.

**What a server can ask the agent to do** — the complete set (`ServerMessage`, same file):

| Message | Effect |
|---|---|
| `config_push` / `config_reload` | update schedules, daily limits, blocked domains, app groups and their limits, approved apps — the agent then closes the managed child's apps that are over a limit or not approved (only those apps' own processes) |
| `remaining_update` | set remaining minutes and allow / warn / lock |
| `lock_now` | lock the given local user |
| `notify_user` | show a desktop notification |
| `pairing_accepted` | finish pairing, store the auth token |
| `unpair` | forget the pairing and restart |
| `fetch_logs` | return the last 50 lines of `journalctl -u screenguard-agent` |
| `update_agent` | run the published installer as root (see below) |

There is no message that runs an arbitrary command, reads a file, or opens a shell. `update_agent` is the one privileged operation: it launches `install.sh --update` from the GitHub releases page as root via `systemd-run`, and writes a loud banner to the journal when it does. There is currently no config switch to turn it off — if that is not acceptable for a machine, self-host, or pin the agent by other means.

The agent always dials out; nothing ever connects to it. That is why cloud mode needs no port forwarding and no VPN, and it is the same outbound WebSocket in both modes (`wss://` with TLS for the cloud endpoint). Local state — pairing, cached rules, usage counters — stays in `/var/lib/screenguard/agent.db`, everything it does is visible in `journalctl -u screenguard-agent`, and `screenguard-agent --reset` cuts it loose from whichever server it is paired with.

## Architecture

```
┌─────────────────────────────────┐        ┌──────────────────────────┐
│  Server machine                 │        │  Managed machine (child) │
│                                 │        │                          │
│  screenguard-server  (REST API) │◄──WS───│  screenguard-agent       │
│  screenguard web UI  (Flask)    │        │  (enforces limits,       │
│                                 │        │   sends notifications)   │
└─────────────────────────────────┘        └──────────────────────────┘
         ▲
         │ REST
    Android app
 (screenguard-mobile)
```

The server and agent can run on the same machine or on separate machines. The agent connects to the server over a persistent WebSocket connection. mDNS (Avahi/Bonjour) is used for automatic discovery on the local network.

## Requirements

- Linux with systemd
- x86_64 or aarch64 CPU
- Managed machines: a graphical session manager that supports `loginctl terminate-session` (GNOME, KDE, etc.)
- Server machine: any Linux with systemd; does not need a graphical desktop

## Install

```bash
curl -fsSL https://github.com/adambie/screenguard/releases/latest/download/install.sh | sudo bash
```

The installer will ask whether to install the **agent**, the **server**, or **both**, then configure and start the appropriate systemd services. For an agent, it asks how to reach the server: mDNS auto-discovery, a fixed URL, or a cloud account (see [Cloud mode](#cloud-mode)).

### Update

```bash
curl -fsSL https://github.com/adambie/screenguard/releases/latest/download/install.sh | sudo bash -s -- --update
```

This downloads the latest binaries, replaces them, and restarts the services. Configs are not touched.

### Uninstall

```bash
curl -fsSL https://github.com/adambie/screenguard/releases/latest/download/install.sh | sudo bash -s -- --uninstall
```

You will be asked separately whether to remove the config directory and the database.

## Running the server in Docker

Two compose files are included. The **agent always runs natively** on the managed (child) machine — it interacts with the desktop session via DBus and cannot run inside a container.

### SQLite (default) — `docker-compose.yml`

Uses host networking so mDNS auto-discovery works — agents on the LAN find the server without any manual configuration. The database is stored in a named Docker volume (`screenguard-data`) and survives restarts and image updates.

```bash
# 1. Get the compose file (images are pulled from ghcr.io — no clone or local build needed)
curl -fsSLO https://raw.githubusercontent.com/adambie/screenguard/main/docker-compose.yml

# 2. Edit docker-compose.yml and replace "change-me-in-production" with strong secrets

# 3. Start
docker compose up -d

# 4. Open the admin UI
#    http://<server-ip>:5000
```

To pin a specific release instead of always tracking `latest`, change the image tags in `docker-compose.yml`:

```yaml
image: ghcr.io/adambie/screenguard-server:v0.10.12
image: ghcr.io/adambie/screenguard-webui:v0.10.12
```

Images are published to GHCR for `linux/amd64` and `linux/arm64` on every release. To build from source instead, use the commented `build:` lines in the compose file.

**Changing the server port:** with host networking a `ports:` mapping has no effect, so set `SCREENGUARD_SERVER_PORT` on the server (e.g. `"8089"`) and point the webui's `SERVER_URL` at the same port (`http://127.0.0.1:8089`). Agents using mDNS pick up the new port automatically; agents with a manual `server_url` need it updated.

### Postgres — `docker-compose.postgres.yml`

Spins up a Postgres container alongside the server. Useful if you want a proper relational backend or plan to move the database off the server host later. Uses bridge networking — **mDNS auto-discovery does not work**, so agents must be pointed at the server manually:

```toml
# /etc/screenguard/agent.toml on each managed machine
server_url = "ws://<server-host-ip>:8080"
```

```bash
# Download docker-compose.postgres.yml, then edit it and replace "change-me-in-production" with strong secrets
docker compose -f docker-compose.postgres.yml up -d
```

The `DATABASE_URL` environment variable controls which database the server connects to. When set to a `postgres://` URL the server uses Postgres; when set to a `sqlite:` path (or left unset) it falls back to SQLite.

### Bridge networking (SQLite)

If host networking is not an option for the default compose (rootless Podman, non-Linux Docker host, strict isolation), `docker-compose.yml` contains a commented-out bridge-mode configuration. The same mDNS caveat applies — you must configure the server address manually on every managed machine.

### Updating

```bash
docker compose pull
docker compose up -d
```

## Firewall

If the server and agents run on **different machines**, the agents need to reach the server's port (default **8080**) over TCP.

**ufw (Ubuntu/Debian)**

```bash
sudo ufw allow 8080/tcp
```

**firewalld (Fedora/RHEL)**

```bash
sudo firewall-cmd --permanent --add-port=8080/tcp
sudo firewall-cmd --reload
```

**mDNS auto-discovery** uses UDP multicast on port 5353. This works out of the box on most home/office networks. If agents fail to find the server automatically, use a fixed URL instead (the installer offers this as an option), or ensure mDNS/multicast traffic is allowed between the machines.

## First-run setup

1. **Server**: after install, open the web UI. On first visit you will be prompted to create an admin account.

2. **Create a profile**: in the web UI, go to Profiles → New profile. Set schedules and daily limits.

3. **Pair an agent**: the installer prints the agent's pairing code when it finishes. To show it again on the managed machine:
   ```
   sudo screenguard-agent --show-pairing-code
   # agents older than 0.10.17:
   sudo journalctl -u screenguard-agent | grep 'PAIRING CODE' | tail -1
   ```
   In the web UI go to Agents, enter that code next to the machine and click Accept. The code proves you control the machine, so it is never shown in the web UI itself. It is valid for 5 minutes (the agent then issues a new one), and 5 wrong attempts make the agent start over with a fresh code.

4. **Assign users**: on the agent detail page, assign each local user account to a profile.

## Locking modes

Each profile has two locking modes. The setting defaults to **off**, preserving ScreenGuard's existing behavior.

- **Default (preserve tasks off):** when access is blocked, ScreenGuard locks the login session and terminates the graphical session after the configured grace period.
- **Preserve tasks on:** when access is blocked, ScreenGuard locks and continues re-locking the login session without terminating it. Applications and unsaved work remain running. Once access is restored, the user returns to the existing session.

In both modes, **Lock now** zeroes the user's remaining allowance. The preserve-tasks setting changes only what happens to the graphical session while access remains blocked.

## App usage reports

Each profile has an **App usage** page (button on the profile page) showing which apps were used and for how long — per day, per week (Monday–Sunday) or in total, summed over all the child's computers.

How it works: every 5 seconds the agent looks at the child's running processes and works out which app each belongs to — from the systemd unit the desktop starts apps in, the launch information GTK/GNOME-style desktops leave behind, or the process tree — and matches it to the app's menu entry. It is designed to work on any desktop with cgroup v2 (every current distribution) and has been tested on KDE Plasma and Hyprland; GNOME, Cinnamon, XFCE and others are covered by tests of their documented behaviour.

What counts:

- Time counts while an app is open **and** the child is active at the computer (session unlocked, not idle) — the same rule as screen time. Two apps open at the same time each count; there is no foreground-window tracking.
- Only things with a visible menu entry are apps. Desktop components (panels, portals, background services) never show up.
- Apps started automatically at login (tray apps, sync clients) are not counted.
- Programs started from a terminal count as the terminal.
- Steam games show by name (from Steam's own files), Minecraft Java as "Minecraft" (also via Prism Launcher / MultiMC), Windows programs under Wine by their `.exe` name.

The agent keeps 35 days of app usage locally and sends day totals to the server once a minute. Turn it off per machine with `app_tracking = false` in `agent.toml`. Agents older than this feature simply don't report app usage; the page tells you to update them.

### App groups and limits

The **Apps** page lists every app seen on the children's computers. Put apps into groups — two are ready-made: **Games** (every app whose menu entry says it's a game, all Steam games, Minecraft) and **Web browsers** (every browser). They pick up newly installed games and browsers by themselves; take a single app out of a group or add one in with one click, or create your own groups (e.g. "School").

On a child's profile, **App limits** sets daily time per group and weekday (`1:30`, `90`, empty = no limit, `0` = blocked that day), and **+15 / +30 / +60** gives extra time for today. Group time is counted once while any app of the group is open, across all the child's computers, and also counts toward the daily screen-time limit.

When a group's time runs out the child gets warnings at the profile's thresholds, then "*Dota 2 will close in 10 seconds*" and the app is closed (a polite close first, forced if it doesn't quit). Starting it again closes it right away. On a blocked day the app is closed at once. Only the app's own processes are closed — never other apps, the desktop, or other users' programs.

### New-app approval

Turn on **New apps → Ask before new apps** on a profile and every app the child hasn't used before is closed on start; the child sees "*Your parents were asked to allow …*" and a request appears on the dashboard and the profile. **Allow** (optionally straight into a group) or **Deny** — the child is told either way. Apps already used when you turn it on stay allowed. Requests made while the computer is offline are delivered when it reconnects.

System-wide installs already need an administrator password, so make sure a child's account is not an administrator: the profile page warns if it is (member of `wheel`/`sudo` or a sudo rule) and shows how to fix it. What a child can still add without admin rights — Flatpak user installs, Steam games, downloaded programs — is caught by new-app approval at first start. A program started from a terminal counts as the terminal; give terminals a group with 0 minutes if that matters.

## Configuration

### Server — `/etc/screenguard/server.toml`

```toml
listen_addr      = "0.0.0.0"
listen_port      = 8080
db_path          = "/var/lib/screenguard/server.db"  # SQLite path (ignored when database_url is set)
# database_url   = "postgres://user:pass@host:5432/screenguard"  # use Postgres instead of SQLite
enable_mdns      = true
jwt_expiry_hours = 24
```

### Agent — `/etc/screenguard/agent.toml`

```toml
# Leave commented out to use mDNS auto-discovery:
# server_url = "http://192.168.1.100:8080"

heartbeat_interval  = 10    # seconds
user_scan_interval  = 300   # seconds
cache_ttl_hours     = 48
min_uid             = 1000  # ignore system accounts below this UID
app_tracking        = true  # per-app usage reports; false = don't scan, store or send them
```

### Cloud mode

> The hosted service is young — its endpoint and this enrolment flow may still
> change. Self-hosted / LAN use is unaffected: skip this section entirely and the
> agent behaves exactly as documented above.

Instead of running your own server on the LAN, an agent can pair with the hosted
service and report to a cloud account — see [Cloud accounts](#cloud-accounts) for
the overview. You sign up at `screenguard.cc`; the agent connects to
`api.screenguard.cc`.

**At install time.** Pass the account to the installer and it writes the config
for you:

```bash
curl -fsSL https://github.com/adambie/screenguard/releases/latest/download/install.sh \
  | sudo bash -s -- --cloud-account=you@example.com
```

The interactive installer offers the same thing: choose **Agent only**, then pick
**Cloud account** when it asks how to reach the server. Either way it creates
`/etc/screenguard/agent.toml` with `cloud_account` set. (If `agent.toml` already
exists it is not modified — add the line by hand, see below.)

**On an already-installed agent.** Add the account to the config and restart:

```toml
# /etc/screenguard/agent.toml
cloud_account = "you@example.com"
# cloud_url  = "wss://api.screenguard.cc/ws"   # override the endpoint if needed
```

```bash
sudo systemctl restart screenguard-agent
```

You can also pass it once on the command line (`screenguard-agent
--cloud-account you@example.com`), but the systemd unit carries no flags, so
without the config line the agent falls back to its saved binding on the next
restart — persisting it in `agent.toml` is the supported way.

- The account is an **email that identifies your cloud account**, not a server
  address. Use `--server-url wss://host/ws` (or `cloud_url`) to point at a
  different / self-run cloud endpoint.
- With a cloud account set, mDNS discovery and `server_url` are ignored.
- The machine then shows up as *pending* under that account and has to be
  approved there before it does anything.
- **Starting with a _different_ cloud account re-pairs from scratch.** If the
  agent was already paired (to a LAN server or another cloud account) and you
  start it with a different `--cloud-account`, it clears the old pairing *and*
  all downloaded rules and usage counters, then pairs anew. **The machine is
  unmanaged until the new account approves it**, and today's screen-time counter
  is reset to zero. Restarting with the *same* account, or with no account, does
  **not** wipe anything. To leave cloud mode, or to move to a new server IP, run
  `screenguard-agent --reset` (see "Agent reset" below).
- An unknown / mistyped account gives no error — the agent just stays *pending*
  forever. This is intentional (it stops the service being used to test which
  emails have accounts).

Environment variable overrides:

| Variable | Description |
|---|---|
| `SCREENGUARD_SERVER_CONFIG` | Path to server config file |
| `SCREENGUARD_AGENT_CONFIG` | Path to agent config file |
| `SCREENGUARD_SERVER_DB` | SQLite database path (default: `/var/lib/screenguard/server.db`) |
| `DATABASE_URL` | Full database URL — overrides `db_path`. Use `postgres://…` for Postgres or `sqlite:/path` for SQLite |
| `SCREENGUARD_SERVER_JWT_SECRET` | JWT signing secret |
| `SCREENGUARD_SERVER_PORT` | Listen port — overrides `listen_port` (default: `8080`) |

## Web UI

The web UI is a small Flask app included in the `webui/` directory. It is **not** installed as a service — it is intended for local administration while the server binary handles all agent communication.

```bash
cd webui
SERVER_URL=http://localhost:8080 uv run --with flask --with requests python app.py
```

## Logs

```bash
journalctl -u screenguard-server -f
journalctl -u screenguard-agent -f
```

## Agent reset (re-pair)

If an agent needs to be re-paired (e.g. after moving to a different server):

```bash
screenguard-agent --reset
sudo systemctl restart screenguard-agent
```

## License

GPL-3.0 — see [LICENSE](LICENSE).
