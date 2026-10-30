# How ScreenGuard works

The details behind the features in the [README](../README.md): what the agent sends and accepts, how screen time and app time are measured, and what happens when time runs out.

- [What the agent does on your machine](#what-the-agent-does-on-your-machine)
- [Locking](#locking) — screen time, time windows, keeping tasks running
- [App usage](#app-usage) — how apps are recognised and what counts
- [App limits](#app-limits) — closing apps when a group's time is up
- [Blocking new apps](#blocking-new-apps)
- [Web pages](#web-pages)

## What the agent does on your machine

The agent behaves the same whether it reports to a server on your own network or to the [cloud instance](cloud.md) — only the address changes. Agent and protocol both live in this repository, so none of this has to be taken on trust.

**What the agent sends up** — the complete list (`crates/common/src/messages.rs`):

- `agent_hello` — machine ID, hostname, timezone, agent version
- `user_list_update` — UID, username and display name of each local account above `min_uid`
- `heartbeat` — per UID: seconds of active time since the last beat, an idle flag, the number of open sessions
- `usage_sync` — per UID and date: total seconds used
- `app_usage_sync` — per UID, date and app: the app's identifier, its menu name, icon name and menu categories, and total seconds; per app group: total seconds (off with `app_tracking = false`)
- `app_request` — per UID: the identifier, name and icon of an app the child started for the first time (only with "Block new apps" on)

That is all. App usage only names apps the child could start from the desktop menu (by their `.desktop` entry, Flatpak/Snap ID or Steam game ID) — never command lines, window titles, file names or URLs, which stay on the machine. No keystrokes, no screenshots, no file contents. Web page blocking runs entirely on the machine: the queries the local DNS proxy sees are never reported.

### What a server can ask the agent to do

The complete set (`ServerMessage`, same file):

| Message | Effect |
|---|---|
| `config_push` / `config_reload` | update daily limits, time windows, blocked sites, app groups and their limits, known apps — the agent then closes the child's apps that are over a limit or blocked (only those apps' own processes) |
| `remaining_update` | set remaining minutes and allow / warn / lock |
| `lock_now` | lock the given local user |
| `notify_user` | show a desktop notification |
| `pairing_accepted` | finish pairing, store the auth token |
| `unpair` | forget the pairing and restart |
| `fetch_logs` | return the last 50 lines of `journalctl -u screenguard-agent` |
| `update_agent` | run the published installer as root (see below) |

There is no message that runs an arbitrary command, reads a file or opens a shell. `update_agent` is the one privileged operation: it launches `install.sh --update` from the GitHub releases page as root via `systemd-run`, and writes a loud banner to the journal when it does. There is no config switch to turn it off — if that is not acceptable for a machine, self-host, or pin the agent by other means.

The agent always dials out; nothing ever connects to it. That is why cloud mode needs no port forwarding and no VPN, and it is the same outbound WebSocket in both modes (`wss://` with TLS for the cloud endpoint). Local state — pairing, cached rules, usage counters — stays in `/var/lib/screenguard/agent.db`, everything it does is visible in `journalctl -u screenguard-agent`, and `screenguard-agent --reset` cuts it loose from whichever server it is paired with.

### Offline

The agent keeps the last rules it received (for `cache_ttl_hours`, 48 by default) and keeps enforcing them when the server is unreachable; usage is sent when it reconnects.

## Locking

Screen time counts while the child is active at the computer: the session is unlocked and not idle. When the day's time runs out, or the time is outside every allowed window, the agent locks all the child's graphical sessions. Before that it warns on the desktop (15, 5 and 1 minute before by default), in the profile's notification language.

Each profile has two locking modes (**Lock behavior** on the profile page):

- **Default (keep tasks running off):** the session is locked, and after the grace period (5 minutes by default) the graphical session is terminated.
- **Keep tasks running on:** the session is locked and re-locked for as long as access stays blocked, without terminating it. Apps and unsaved work keep running; once time is added, the child continues where they left off.

In both modes **Lock** on the profile page zeroes the remaining time for today.

Time windows are entered in the installation's time zone (Settings → Timezone) and converted to each computer's time zone.

## App usage

Every 5 seconds the agent looks at the child's running processes and works out which app each belongs to — from the systemd unit the desktop starts apps in, the launch information GTK/GNOME-style desktops leave behind, or the process tree — and matches it to the app's menu entry. It is designed for any desktop with cgroup v2 (every current distribution) and has been tested on KDE Plasma and Hyprland; GNOME, Cinnamon, XFCE and others are covered by tests of their documented behaviour.

What counts:

- Time counts while an app is open **and** the child is active (session unlocked, not idle) — the same rule as screen time. Two apps open at the same time each count; there is no foreground-window tracking.
- Only things with a visible menu entry are apps. Desktop components (panels, portals, background services) never show up.
- Apps started automatically at login (tray apps, sync clients) are not counted.
- Programs started from a terminal count as the terminal.
- Steam games show by name (from Steam's own files), Minecraft Java as "Minecraft" (also via Prism Launcher / MultiMC), Windows programs under Wine by their `.exe` name.
- A newly installed app is recognised within about 30 seconds (the agent rereads the menu entries at most that often).

The agent keeps 35 days of app usage locally and sends day totals once a minute. Turn it off per machine with `app_tracking = false` in `agent.toml`; agents too old for app usage are flagged on the profile page.

## App limits

Each app is in at most one group. An app placed in a group by hand stays there; otherwise the first group whose rules match takes it (the ready-made groups use menu categories, Steam and Minecraft). Group time is counted once while any app of the group is open, across all the child's computers, and also counts toward the daily screen-time limit.

When a group's time runs out the child gets warnings, then "*Dota 2 will close in 10 seconds*", and the app is closed: it is asked to quit (SIGTERM to its main process, as at logout — browsers may offer to restore the session next time) and forced after 10 more seconds if it doesn't. Starting it again closes it right away. On a blocked day, and for apps in **Blocked**, the app is closed at once. Only the app's own processes are closed — never other apps, the desktop, or other users' programs.

## Blocking new apps

With **Block new apps** on, an app the child starts for the first time is closed and the server puts it into the **Blocked** group; the child sees "*Your parents were asked to allow …*". Apps started automatically at login are left alone. Apps already used when you switch it on stay as they are — if nothing has been used yet, every app would be blocked, so let a new profile run for a while first. New apps started while the computer is offline are reported when it reconnects.

System-wide installs already need an administrator password, so make sure a child's account is not an administrator: the profile page warns if it is (member of `wheel`/`sudo`, or a sudo rule) and shows how to fix it. What a child can still add without admin rights — Flatpak user installs, Steam games, downloaded programs — is caught at first start. A program started from a terminal counts as the terminal; put terminals in Blocked or a 0-minute group if that matters.

## Web pages

Restricted sites are blocked on the child's computers by a small DNS proxy run by the agent: the child's DNS lookups go to it (per-UID nftables rules), and it answers lookups for a restricted domain — and its subdomains — with NXDOMAIN. Other lookups are passed on to the system resolver and nothing is logged or reported. Known DNS-over-HTTPS providers are blocked so browsers can't go around the proxy. Needs nftables on the computer; the Agents page shows where it is available ("web filter"). Details: [web-filtering-technical.md](web-filtering-technical.md).
