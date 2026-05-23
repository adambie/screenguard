# Parental Controller — Full Design Specification

> This document is a complete implementation instruction for the **Parental Controller** system.
> It contains all architectural decisions, data models, message schemas, API surface, and
> behavioral rules needed to implement the system without further design input.

---

## Table of Contents

1. [Overview](#1-overview)
2. [Architecture](#2-architecture)
3. [Tech Stack](#3-tech-stack)
4. [Shared Data Contracts](#4-shared-data-contracts)
5. [Agent (Enforcement Agent)](#5-agent-enforcement-agent)
6. [Server (Management Server)](#6-server-management-server)
7. [Pairing & Discovery](#7-pairing--discovery)
8. [Sync & Conflict Resolution](#8-sync--conflict-resolution)
9. [Enforcement Logic](#9-enforcement-logic)
10. [Bootstrapping / First-Run Flow](#10-bootstrapping--first-run-flow)
11. [Future Considerations](#11-future-considerations)

---

## 1. Overview

**Parental Controller** is a Linux-native parental control system that manages computer usage
time for child user accounts. It consists of three components:

| Component | Role |
|---|---|
| **Enforcement Agent** | Runs on each managed Linux machine as a systemd service. Controls user sessions via DBus/logind. Tracks usage. Enforces time limits. |
| **Management Server** | Central server that stores configuration, aggregates usage across devices, calculates remaining time, and exposes a REST API for the UI. |
| **UI (Web/Mobile)** | Consumes the server's REST API. Not specified in this document — it is fully decoupled from the backend. |

### Key Design Principles

- **Server is the authority** for configuration (schedules, limits, adjustments) and for calculating remaining time.
- **Agent is the authority** for usage data and local user lists.
- **Offline resilience**: both agent and server operate independently when disconnected. Agent enforces cached rules; server accepts config changes and syncs when reconnected.
- **Profile-centric enforcement**: time limits and schedules are defined on logical user profiles, not per-device. A profile can be linked to one or more agent-users across devices. Usage is aggregated across all linked agents.

---

## 2. Architecture

```
┌─────────────────────┐       WSS (JSON)       ┌─────────────────────┐     REST (JSON)    ┌─────────┐
│  Enforcement Agent  │◄───────────────────────►│  Management Server  │◄──────────────────►│   UI    │
│                     │                         │                     │                     │ Web/App │
│  • systemd service  │                         │  • Rules engine     │                     │         │
│  • DBus/logind      │                         │  • Profile mgmt     │                     │  • CRUD │
│  • SQLite cache     │                         │  • Usage aggregation│                     │    rules│
│  • Usage tracking   │                         │  • PostgreSQL       │                     │  • View │
│  • Idle detection   │                         │  • WSS hub          │                     │    usage│
│                     │                         │  • REST API         │                     │         │
└─────────────────────┘                         └─────────────────────┘                     └─────────┘
```

- Agent ↔ Server: persistent **WSS** (WebSocket Secure) connection, JSON messages.
- Server ↔ UI: stateless **REST API** with JWT authentication.
- Agent and server are **always separate processes** (separate systemd units), even when running on the same machine.
- Single-machine mode (agent + server on localhost) is a valid and supported deployment.

---

## 3. Tech Stack

| Component | Technology |
|---|---|
| Agent language | Rust |
| Server language | Rust |
| Agent local DB | SQLite (`rusqlite`) |
| Server DB | PostgreSQL (`sqlx`) |
| Agent DBus | `zbus` (pure Rust, async) |
| Agent systemd | `libsystemd` crate (watchdog/notify) |
| WSS (both sides) | `tokio-tungstenite` |
| Server HTTP/REST | `axum` |
| Server auth | JWT (`jsonwebtoken` crate) |
| Discovery | Avahi/mDNS (`zeroconf` or `mdns-sd` crate) |
| Serialization | `serde` + `serde_json` |

---

## 4. Shared Data Contracts

These types are shared between agent and server. They define the WSS message envelope and
all exchanged payloads.

### 4.1 WSS Message Envelope

All WSS messages use this envelope:

```json
{
  "type": "message_type_name",
  "timestamp": "2026-05-22T13:00:00Z",
  "payload": { ... }
}
```

**Rust type:**

```rust
#[derive(Serialize, Deserialize)]
struct WssMessage {
    #[serde(rename = "type")]
    msg_type: String,
    timestamp: DateTime<Utc>,
    payload: serde_json::Value,
}
```

### 4.2 Message Types — Agent → Server

#### `agent_hello`
Sent on WSS connection establishment after pairing.

```json
{
  "type": "agent_hello",
  "payload": {
    "machine_id": "abc123...",
    "hostname": "bigPcOne",
    "timezone": "Europe/Warsaw",
    "agent_version": "0.1.0",
    "last_config_version": 5
  }
}
```

#### `user_list_update`
Sent on startup and whenever local users change (periodic scan).

```json
{
  "type": "user_list_update",
  "payload": {
    "users": [
      { "local_uid": 1000, "username": "tom", "display_name": "Tom" },
      { "local_uid": 1001, "username": "alice", "display_name": "Alice" }
    ],
    "removed_uids": [1002]
  }
}
```

Root/system users (UID < 1000 typically) are never reported.

#### `heartbeat`
Sent every **10 seconds** while any managed user has an active session.

```json
{
  "type": "heartbeat",
  "payload": {
    "users": [
      {
        "local_uid": 1000,
        "active_seconds_since_last": 10,
        "idle": false,
        "session_count": 1
      }
    ]
  }
}
```

- `active_seconds_since_last`: seconds of **non-idle** usage since the previous heartbeat.
- `idle`: current idle state (from DBus `IdleHint`). When idle, `active_seconds_since_last` is 0.
- Idle time does NOT count against the quota.

#### `usage_sync`
Sent on reconnect after an offline period. Contains accumulated usage during offline period.

```json
{
  "type": "usage_sync",
  "payload": {
    "usage": [
      { "local_uid": 1000, "date": "2026-05-22", "used_seconds": 3600 },
      { "local_uid": 1000, "date": "2026-05-21", "used_seconds": 7200 }
    ]
  }
}
```

#### `pairing_request`
Sent during discovery/pairing phase (before agent is paired).

```json
{
  "type": "pairing_request",
  "payload": {
    "machine_id": "abc123...",
    "hostname": "bigPcOne",
    "pairing_code": "A7X3"
  }
}
```

### 4.3 Message Types — Server → Agent

#### `config_push`
Sent after `agent_hello` if agent's config version is stale, and whenever config changes.

```json
{
  "type": "config_push",
  "payload": {
    "config_version": 6,
    "users": [
      {
        "local_uid": 1000,
        "profile_id": "uuid-...",
        "status": "managed",
        "schedules": [
          { "day_of_week": 0, "start_time": "10:00", "end_time": "12:00" },
          { "day_of_week": 0, "start_time": "16:00", "end_time": "18:00" },
          { "day_of_week": 2, "start_time": "12:00", "end_time": "18:00" },
          { "day_of_week": 5, "start_time": "00:00", "end_time": "23:59" },
          { "day_of_week": 6, "start_time": "00:00", "end_time": "23:59" }
        ],
        "daily_limits": [
          { "day_of_week": 0, "allowed_minutes": 120 },
          { "day_of_week": 1, "allowed_minutes": 120 },
          { "day_of_week": 2, "allowed_minutes": 120 },
          { "day_of_week": 3, "allowed_minutes": 120 },
          { "day_of_week": 4, "allowed_minutes": 120 },
          { "day_of_week": 5, "allowed_minutes": 240 },
          { "day_of_week": 6, "allowed_minutes": 240 }
        ],
        "adjustments_today": 0,
        "lockout_grace_minutes": 5,
        "warning_thresholds_minutes": [15, 5, 1]
      }
    ]
  }
}
```

- `day_of_week`: 0 = Monday, 6 = Sunday.
- `daily_limits`: if absent for a day, that day is unlimited (still bound by schedule).
- `schedules`: if none defined for a user, all times are allowed.
- `adjustments_today`: net sum of all time adjustments for today (minutes, can be negative).

#### `remaining_update`
Sent in response to each heartbeat and after usage_sync. Pushed to **all agents** linked to the same profile.

```json
{
  "type": "remaining_update",
  "payload": {
    "users": [
      {
        "local_uid": 1000,
        "remaining_minutes": 42,
        "limit_today_minutes": 120,
        "used_today_minutes": 78,
        "adjustments_today_minutes": 0,
        "current_window_ends_at": "18:00",
        "next_window_starts_at": null,
        "enforce": "allow"
      }
    ]
  }
}
```

- `enforce`: one of `"allow"`, `"warn"`, `"lock"`.
  - `allow`: user may continue.
  - `warn`: remaining time is within a warning threshold — agent should notify the user.
  - `lock`: remaining time is 0 or user is outside schedule — agent must lock/terminate session.
- `current_window_ends_at`: when the current schedule window closes (null if no schedule / unlimited).
- `next_window_starts_at`: next allowed window today (null if none remain).

#### `pairing_accepted`
Sent when admin accepts a pending agent.

```json
{
  "type": "pairing_accepted",
  "payload": {
    "agent_id": "uuid-...",
    "auth_token": "long-lived-token-..."
  }
}
```

#### `lock_now`
Sent when admin triggers immediate lock for a user.

```json
{
  "type": "lock_now",
  "payload": {
    "local_uid": 1000
  }
}
```

This is implemented server-side as a time adjustment that zeros remaining time, but the
explicit message ensures instant enforcement without waiting for the next heartbeat cycle.

#### `config_reload`
Tells the agent to re-fetch and re-apply its cached config without restarting.

```json
{
  "type": "config_reload",
  "payload": {}
}
```

### 4.4 Shared Rust Types (crate: `parental-controller-common`)

Create a shared crate for types used by both agent and server:

```rust
// Schedules, limits, adjustments
pub struct Schedule {
    pub day_of_week: u8,        // 0=Mon, 6=Sun
    pub start_time: NaiveTime,  // chrono::NaiveTime
    pub end_time: NaiveTime,
}

pub struct DailyLimit {
    pub day_of_week: u8,
    pub allowed_minutes: u32,
}

pub struct UserConfig {
    pub local_uid: u32,
    pub profile_id: Uuid,
    pub status: UserStatus,     // Managed | Unmanaged
    pub schedules: Vec<Schedule>,
    pub daily_limits: Vec<DailyLimit>,
    pub adjustments_today: i32,
    pub lockout_grace_minutes: u32,
    pub warning_thresholds_minutes: Vec<u32>,
}

pub struct LocalUser {
    pub local_uid: u32,
    pub username: String,
    pub display_name: String,
}

pub struct HeartbeatEntry {
    pub local_uid: u32,
    pub active_seconds_since_last: u32,
    pub idle: bool,
    pub session_count: u32,
}

pub struct UsageEntry {
    pub local_uid: u32,
    pub date: NaiveDate,
    pub used_seconds: u64,
}

pub struct RemainingEntry {
    pub local_uid: u32,
    pub remaining_minutes: i32,
    pub limit_today_minutes: Option<u32>,
    pub used_today_minutes: u32,
    pub adjustments_today_minutes: i32,
    pub current_window_ends_at: Option<NaiveTime>,
    pub next_window_starts_at: Option<NaiveTime>,
    pub enforce: EnforceAction,  // Allow | Warn | Lock
}

pub enum EnforceAction {
    Allow,
    Warn,
    Lock,
}

pub enum UserStatus {
    Managed,
    Unmanaged,
}
```

---

## 5. Agent (Enforcement Agent)

### 5.1 Overview

- Binary name: `parental-controller-agent`
- Runs as: root-owned systemd service (`parental-controller-agent.service`)
- Manages: local user sessions via DBus/logind
- Stores: SQLite database at `/var/lib/parental-controller/agent.db`
- Config file: `/etc/parental-controller/agent.toml`

### 5.2 Agent Config File

```toml
# /etc/parental-controller/agent.toml

# If set, agent connects directly to this server instead of using mDNS discovery.
# server_url = "wss://192.168.1.100:8443"

# Heartbeat interval in seconds (default: 10)
heartbeat_interval = 10

# How often to scan for local user changes, in seconds (default: 300)
user_scan_interval = 300

# How long to enforce cached rules when server is unreachable, in hours (default: 48)
cache_ttl_hours = 48

# Minimum UID to consider as a real user (default: 1000)
min_uid = 1000
```

### 5.3 Agent SQLite Schema

```sql
-- Pairing state (single row, id always = 1)
CREATE TABLE server_connection (
    id              INTEGER PRIMARY KEY CHECK (id = 1),
    server_url      TEXT NOT NULL,
    auth_token      TEXT NOT NULL,
    agent_id        TEXT NOT NULL,
    paired_at       INTEGER NOT NULL,      -- unix epoch
    last_sync_at    INTEGER
);

-- Last known config version from server
CREATE TABLE config_meta (
    id              INTEGER PRIMARY KEY CHECK (id = 1),
    config_version  INTEGER NOT NULL DEFAULT 0,
    received_at     INTEGER NOT NULL
);

-- Cached managed users and their config
CREATE TABLE managed_users (
    local_uid       INTEGER PRIMARY KEY,
    local_username  TEXT NOT NULL,
    profile_id      TEXT NOT NULL,
    status          TEXT NOT NULL CHECK (status IN ('managed', 'unmanaged'))
);

CREATE TABLE cached_schedules (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    local_uid       INTEGER NOT NULL REFERENCES managed_users(local_uid) ON DELETE CASCADE,
    day_of_week     INTEGER NOT NULL CHECK (day_of_week BETWEEN 0 AND 6),
    start_time      TEXT NOT NULL,         -- "HH:MM"
    end_time        TEXT NOT NULL           -- "HH:MM"
);

CREATE TABLE cached_daily_limits (
    local_uid       INTEGER NOT NULL REFERENCES managed_users(local_uid) ON DELETE CASCADE,
    day_of_week     INTEGER NOT NULL CHECK (day_of_week BETWEEN 0 AND 6),
    allowed_minutes INTEGER NOT NULL,
    PRIMARY KEY (local_uid, day_of_week)
);

CREATE TABLE cached_adjustments (
    local_uid       INTEGER NOT NULL REFERENCES managed_users(local_uid) ON DELETE CASCADE,
    target_date     TEXT NOT NULL,          -- "YYYY-MM-DD"
    adjustment_minutes INTEGER NOT NULL,
    PRIMARY KEY (local_uid, target_date)
);

CREATE TABLE cached_enforcement (
    local_uid       INTEGER NOT NULL REFERENCES managed_users(local_uid) ON DELETE CASCADE,
    lockout_grace_minutes   INTEGER NOT NULL DEFAULT 5,
    warning_thresholds      TEXT NOT NULL DEFAULT '15,5,1',  -- comma-separated minutes
    PRIMARY KEY (local_uid)
);

-- Usage tracking (agent-authoritative, synced up to server)
CREATE TABLE usage_log (
    local_uid       INTEGER NOT NULL,
    date            TEXT NOT NULL,          -- "YYYY-MM-DD"
    used_seconds    INTEGER NOT NULL DEFAULT 0,
    synced          INTEGER NOT NULL DEFAULT 0,  -- 0=pending, 1=synced
    PRIMARY KEY (local_uid, date)
);

-- Server-provided remaining time (updated from remaining_update messages)
CREATE TABLE server_remaining (
    local_uid           INTEGER PRIMARY KEY,
    remaining_minutes   INTEGER NOT NULL,
    enforce             TEXT NOT NULL CHECK (enforce IN ('allow', 'warn', 'lock')),
    updated_at          INTEGER NOT NULL    -- unix epoch
);

-- Active session tracking (operational, not synced)
CREATE TABLE active_sessions (
    local_uid       INTEGER NOT NULL,
    session_id      TEXT NOT NULL,
    started_at      INTEGER NOT NULL,
    last_heartbeat  INTEGER NOT NULL,
    is_idle         INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (local_uid, session_id)
);

-- Agent operational mode
CREATE TABLE agent_state (
    id              INTEGER PRIMARY KEY CHECK (id = 1),
    mode            TEXT NOT NULL CHECK (mode IN ('unpaired', 'online', 'offline')) DEFAULT 'unpaired',
    offline_since   INTEGER              -- unix epoch, set when connection lost
);
```

### 5.4 Agent Behavior

#### Startup Sequence
1. Read config from `/etc/parental-controller/agent.toml`.
2. Open/create SQLite database.
3. Check `server_connection` table:
   - If empty (unpaired): enter discovery mode (mDNS broadcast or connect to configured URL).
   - If paired: connect WSS to stored `server_url` using `auth_token`.
4. On successful WSS connection: send `agent_hello`, then `user_list_update`.
5. Receive `config_push` if config version is stale.
6. Subscribe to DBus logind signals for session changes and idle state.
7. Begin heartbeat loop.

#### Session Monitoring via DBus
- Connect to `org.freedesktop.login1` Manager interface.
- Listen for signals: `SessionNew`, `SessionRemoved`, `PrepareForSleep`.
- For each session: monitor `IdleHint` property changes.
- Track active sessions in `active_sessions` table.
- Multiple sessions for the same user count time only **once** — if any session is non-idle, the user is considered active.

#### Heartbeat Loop (every 10 seconds)
1. For each managed user with ≥1 active non-idle session:
   - Calculate `active_seconds_since_last` (10s if continuously active, less if became idle mid-interval).
2. Send `heartbeat` message to server.
3. Receive `remaining_update` in response.
4. Store in `server_remaining` table.
5. Evaluate enforcement action (see Section 9).

#### Offline Mode
- Triggered when WSS connection is lost and reconnect fails.
- Set `agent_state.mode = 'offline'`, record `offline_since`.
- Switch to local enforcement:
  - Use cached schedules and limits from SQLite.
  - Track usage locally in `usage_log` (increment `used_seconds`, mark `synced=0`).
  - Calculate remaining time locally: `allowed_minutes + cached_adjustments - (used_seconds / 60)`.
- Attempt reconnection with exponential backoff: 5s, 10s, 30s, 60s, 120s, max 300s.
- On reconnect: send `usage_sync` with all `synced=0` records, then receive corrected `remaining_update`.

#### Enforcement Actions
- **Allow**: no action.
- **Warn**: (future) send desktop notification to the user. For now, log it.
- **Lock**: lock the user's session via DBus `org.freedesktop.login1.Session.Lock()`. If session does not lock within `lockout_grace_minutes`, terminate it via `org.freedesktop.login1.Session.Terminate()`.

#### Local User Scanning
- Every `user_scan_interval` seconds (default 300), read `/etc/passwd` or use `getent passwd`.
- Filter: only UIDs >= `min_uid`, skip `nobody`, skip users with `/usr/sbin/nologin` or `/bin/false` shells.
- Compare with last known list. Send `user_list_update` if changed.

#### Midnight Handling
- At local midnight (00:00), the agent:
  1. Resets daily usage counters for the new day.
  2. Re-evaluates schedules and limits for the new weekday.
  3. If the new day has no schedule window currently open, lock active sessions.
  4. If online, the server's `remaining_update` will reflect the new day automatically.

#### Cache TTL
- If `offline_since` exceeds `cache_ttl_hours` (default 48h):
  - Continue enforcing but log warnings.
  - Do NOT go fully permissive — maintain last known rules.
  - (Rationale: failing open defeats the purpose; stale rules are better than none.)

### 5.5 Systemd Unit

```ini
# /etc/systemd/system/parental-controller-agent.service
[Unit]
Description=Parental Controller Enforcement Agent
After=network-online.target dbus.service
Wants=network-online.target

[Service]
Type=notify
ExecStart=/usr/local/bin/parental-controller-agent
Restart=always
RestartSec=5
WatchdogSec=60

# Hardening
ProtectSystem=strict
ReadWritePaths=/var/lib/parental-controller
NoNewPrivileges=true
PrivateTmp=true
ProtectHome=read-only
ProtectKernelModules=true
ProtectKernelTunables=true

[Install]
WantedBy=multi-user.target
```

---

## 6. Server (Management Server)

### 6.1 Overview

- Binary name: `parental-controller-server`
- Runs as: systemd service (`parental-controller-server.service`)
- Exposes: WSS endpoint for agents, REST API for UI
- Stores: PostgreSQL database
- Config file: `/etc/parental-controller/server.toml`

### 6.2 Server Config File

```toml
# /etc/parental-controller/server.toml

# HTTP/WSS listen address
listen_addr = "0.0.0.0"
listen_port = 8443

# TLS (required for WSS)
tls_cert = "/etc/parental-controller/cert.pem"
tls_key = "/etc/parental-controller/key.pem"

# PostgreSQL connection
database_url = "postgresql://parental:password@localhost/parental_controller"

# JWT secret (generated on first run if not set)
# jwt_secret = "auto-generated-on-first-run"

# JWT token expiry in hours (default: 24)
jwt_expiry_hours = 24

# Enable mDNS discovery advertisement (default: true)
enable_mdns = true

# mDNS service type
mdns_service_type = "_parental-controller._tcp"
```

### 6.3 Server PostgreSQL Schema

```sql
-- Admin users (for REST API / UI authentication)
CREATE TABLE admin_users (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    username        TEXT NOT NULL UNIQUE,
    password_hash   TEXT NOT NULL,         -- bcrypt or argon2
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Registered agents (devices)
CREATE TABLE agents (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    machine_id      TEXT NOT NULL UNIQUE,
    display_name    TEXT NOT NULL,          -- defaults to hostname, admin can rename
    hostname        TEXT NOT NULL,
    timezone        TEXT NOT NULL DEFAULT 'UTC',
    status          TEXT NOT NULL CHECK (status IN ('pending', 'paired', 'disabled'))
                    DEFAULT 'pending',
    auth_token_hash TEXT,                  -- hashed long-lived token for WSS auth
    agent_version   TEXT,
    paired_at       TIMESTAMPTZ,
    last_seen_at    TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Logical user profiles (the entity that schedules/limits attach to)
CREATE TABLE user_profiles (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    display_name    TEXT NOT NULL,          -- "Tom"
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Local users reported by agents, linked to profiles
CREATE TABLE agent_users (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agent_id        UUID NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    profile_id      UUID REFERENCES user_profiles(id) ON DELETE SET NULL,
    local_uid       INTEGER NOT NULL,
    local_username  TEXT NOT NULL,
    display_name    TEXT,
    status          TEXT NOT NULL CHECK (status IN ('unmanaged', 'managed', 'deleted'))
                    DEFAULT 'unmanaged',
    first_seen_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_reported_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(agent_id, local_uid)
);

-- Allowed time windows (attached to profile, not agent_user)
CREATE TABLE schedules (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    profile_id      UUID NOT NULL REFERENCES user_profiles(id) ON DELETE CASCADE,
    day_of_week     SMALLINT NOT NULL CHECK (day_of_week BETWEEN 0 AND 6),
    start_time      TIME NOT NULL,
    end_time        TIME NOT NULL,
    CHECK (start_time < end_time)
);

-- Daily time limits (attached to profile)
CREATE TABLE daily_limits (
    profile_id      UUID NOT NULL REFERENCES user_profiles(id) ON DELETE CASCADE,
    day_of_week     SMALLINT NOT NULL CHECK (day_of_week BETWEEN 0 AND 6),
    allowed_minutes INTEGER NOT NULL CHECK (allowed_minutes > 0),
    PRIMARY KEY (profile_id, day_of_week)
);

-- Ad-hoc time adjustments (attached to profile, for a specific date)
CREATE TABLE time_adjustments (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    profile_id      UUID NOT NULL REFERENCES user_profiles(id) ON DELETE CASCADE,
    target_date     DATE NOT NULL,
    adjustment_minutes INTEGER NOT NULL,   -- positive=grant, negative=remove
    reason          TEXT,
    created_by      UUID REFERENCES admin_users(id),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    synced_to_agents BOOLEAN NOT NULL DEFAULT false
);

-- Enforcement settings per profile
CREATE TABLE enforcement_settings (
    profile_id              UUID PRIMARY KEY REFERENCES user_profiles(id) ON DELETE CASCADE,
    lockout_grace_minutes   INTEGER NOT NULL DEFAULT 5,
    warning_thresholds      TEXT NOT NULL DEFAULT '15,5,1'   -- comma-separated minutes
);

-- Daily usage per agent_user (granular: per device)
CREATE TABLE daily_usage (
    agent_user_id   UUID NOT NULL REFERENCES agent_users(id) ON DELETE CASCADE,
    date            DATE NOT NULL,
    used_seconds    INTEGER NOT NULL DEFAULT 0,
    reported_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (agent_user_id, date)
);

-- Config version tracker per profile (bumped on any config change)
CREATE TABLE config_versions (
    profile_id      UUID PRIMARY KEY REFERENCES user_profiles(id) ON DELETE CASCADE,
    version         INTEGER NOT NULL DEFAULT 1,
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Audit log (append-only)
CREATE TABLE audit_log (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    admin_user_id   UUID REFERENCES admin_users(id),
    action          TEXT NOT NULL,
    target_type     TEXT,                  -- 'profile', 'agent', 'agent_user'
    target_id       UUID,
    detail          JSONB,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Indexes
CREATE INDEX idx_agent_users_agent ON agent_users(agent_id);
CREATE INDEX idx_agent_users_profile ON agent_users(profile_id);
CREATE INDEX idx_schedules_profile ON schedules(profile_id);
CREATE INDEX idx_daily_usage_date ON daily_usage(date);
CREATE INDEX idx_time_adjustments_profile_date ON time_adjustments(profile_id, target_date);
CREATE INDEX idx_audit_log_created ON audit_log(created_at);
```

### 6.4 Server Behavior

#### Startup Sequence
1. Read config from `/etc/parental-controller/server.toml`.
2. Connect to PostgreSQL, run migrations if needed.
3. Check if any `admin_users` exist:
   - If none: set a flag that forces admin creation on first REST API call (see Section 10).
4. Start WSS listener on configured port.
5. Start REST API on same port (different path prefix).
6. If `enable_mdns=true`: advertise `_parental-controller._tcp` service via Avahi/mDNS.
7. Load all active WSS connections from paired agents.

#### Agent Connection Handling
1. Agent connects via WSS with `Authorization: Bearer <auth_token>` header.
2. Server validates token hash against `agents.auth_token_hash`.
3. On valid connection: update `agents.last_seen_at`, mark agent as online in memory.
4. Receive `agent_hello`: check `last_config_version`. If stale, send `config_push`.
5. Receive `user_list_update`: upsert `agent_users`, mark removed UIDs as `deleted`.
6. Enter heartbeat loop: receive heartbeats, aggregate usage, send `remaining_update`.
7. On WSS disconnect: mark agent as offline in memory, update `last_seen_at`.

#### Remaining Time Calculation
When a heartbeat arrives for a managed user:

```
profile = agent_user.profile
today = current date in agent's timezone

1. used_today = SUM(daily_usage.used_seconds) for all agent_users linked to this profile, for today
   + heartbeat.active_seconds_since_last (just received)
   → convert to minutes

2. limit_today = daily_limits for profile, for today's day_of_week
   → if not defined: unlimited (use a sentinel like 1440)

3. adjustments_today = SUM(time_adjustments.adjustment_minutes) for profile, for today

4. remaining = limit_today + adjustments_today - used_today_minutes
   → cap at 0 minimum

5. Check schedule: is current time (in agent's timezone) within any schedule window?
   → If no schedules defined: always in window
   → If schedules exist but current time is outside all windows: enforce = "lock"
   → If inside a window: cap remaining by minutes until window ends

6. Determine enforce action:
   → remaining <= 0: "lock"
   → remaining <= min(warning_thresholds): "warn"
   → else: "allow"
```

Update `daily_usage` with the new seconds, then send `remaining_update` to **all connected agents** that have agent_users linked to this profile.

#### Config Change Propagation
When admin changes schedules/limits/adjustments via REST API:
1. Update the database.
2. Bump `config_versions.version` for the affected profile.
3. Insert into `audit_log`.
4. For each online agent that has agent_users linked to this profile:
   - Send `config_push` with the new config.
   - Send `remaining_update` with recalculated remaining time.
5. For offline agents: changes will be synced when they reconnect (via `agent_hello` config version check).

### 6.5 REST API

Base path: `/api/v1`

All endpoints except `/api/v1/auth/*` require JWT in `Authorization: Bearer <token>` header.

#### Authentication

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/auth/setup` | Create initial admin account (only works if no admin exists) |
| `POST` | `/auth/login` | Login, returns JWT |

**POST `/auth/setup`**
```json
// Request
{ "username": "admin", "password": "secure-password" }
// Response 201
{ "message": "Admin account created" }
// Response 409 (if admin already exists)
{ "error": "Admin account already exists" }
```

**POST `/auth/login`**
```json
// Request
{ "username": "admin", "password": "secure-password" }
// Response 200
{ "token": "eyJ...", "expires_at": "2026-05-23T13:00:00Z" }
```

#### Agents

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/agents` | List all agents (with status, last_seen) |
| `GET` | `/agents/:id` | Get agent details |
| `PATCH` | `/agents/:id` | Update agent (display_name, status) |
| `POST` | `/agents/:id/accept` | Accept a pending agent |
| `DELETE` | `/agents/:id` | Remove agent |

**GET `/agents`**
```json
// Response 200
{
  "agents": [
    {
      "id": "uuid",
      "machine_id": "abc123",
      "display_name": "Big PC One",
      "hostname": "bigPcOne",
      "status": "paired",
      "online": true,
      "last_seen_at": "2026-05-22T13:50:00Z",
      "agent_version": "0.1.0",
      "user_count": 3
    }
  ]
}
```

**POST `/agents/:id/accept`**
```json
// Response 200
{ "message": "Agent accepted", "agent_id": "uuid" }
```

#### User Profiles

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/profiles` | List all profiles |
| `POST` | `/profiles` | Create a profile |
| `GET` | `/profiles/:id` | Get profile with schedules, limits, linked agent_users |
| `PATCH` | `/profiles/:id` | Update profile (display_name) |
| `DELETE` | `/profiles/:id` | Delete profile |

#### Agent Users (per-agent local users)

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/agents/:id/users` | List local users for an agent |
| `PATCH` | `/agent-users/:id` | Update agent_user (link to profile, set status) |

**PATCH `/agent-users/:id`** — Link to profile:
```json
// Request
{ "profile_id": "uuid", "status": "managed" }
// Response 200
{ "message": "User linked to profile" }
```

#### Schedules

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/profiles/:id/schedules` | List schedules for a profile |
| `PUT` | `/profiles/:id/schedules` | Replace all schedules for a profile |

**PUT `/profiles/:id/schedules`** — Full replacement (simpler than individual CRUD):
```json
// Request
{
  "schedules": [
    { "day_of_week": 0, "start_time": "10:00", "end_time": "12:00" },
    { "day_of_week": 0, "start_time": "16:00", "end_time": "18:00" }
  ]
}
// Response 200
{ "message": "Schedules updated", "config_version": 7 }
```

#### Daily Limits

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/profiles/:id/daily-limits` | Get daily limits |
| `PUT` | `/profiles/:id/daily-limits` | Replace all daily limits |

**PUT `/profiles/:id/daily-limits`**
```json
// Request
{
  "limits": [
    { "day_of_week": 0, "allowed_minutes": 120 },
    { "day_of_week": 5, "allowed_minutes": 240 },
    { "day_of_week": 6, "allowed_minutes": 240 }
  ]
}
// Response 200
{ "message": "Daily limits updated", "config_version": 8 }
```

Days not included have no limit (unlimited, still bound by schedule).

#### Time Adjustments

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/profiles/:id/adjustments` | List adjustments (filterable by date) |
| `POST` | `/profiles/:id/adjustments` | Add a time adjustment |
| `POST` | `/profiles/:id/lock-now` | Lock now (zero remaining time for today) |

**POST `/profiles/:id/adjustments`**
```json
// Request
{ "target_date": "2026-05-22", "adjustment_minutes": 30, "reason": "Good behavior" }
// Response 201
{ "id": "uuid", "new_remaining_minutes": 72 }
```

**POST `/profiles/:id/lock-now`**
```json
// Response 200
{ "message": "Lock command sent", "adjustment_id": "uuid" }
```
Internally: inserts a negative adjustment to zero remaining time, then sends `lock_now` to all connected agents with agent_users linked to this profile.

#### Usage & Dashboard

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/profiles/:id/usage` | Usage history (query params: `from`, `to`) |
| `GET` | `/profiles/:id/status` | Current status (remaining time, enforce state, per-agent breakdown) |
| `GET` | `/dashboard` | Overview of all profiles with today's status |

**GET `/profiles/:id/status`**
```json
// Response 200
{
  "profile": {
    "id": "uuid",
    "display_name": "Tom",
    "today": {
      "date": "2026-05-22",
      "day_of_week": 3,
      "limit_minutes": 120,
      "used_minutes": 78,
      "adjustments_minutes": 0,
      "remaining_minutes": 42,
      "enforce": "allow",
      "current_window": { "start": "16:00", "end": "18:00" },
      "next_window": null
    },
    "agents": [
      {
        "agent_id": "uuid",
        "agent_name": "Big PC One",
        "local_username": "tom",
        "online": true,
        "last_seen_at": "2026-05-22T13:50:00Z",
        "used_today_minutes": 48
      },
      {
        "agent_id": "uuid",
        "agent_name": "Toms Laptop",
        "local_username": "tom",
        "online": false,
        "last_seen_at": "2026-05-22T10:30:00Z",
        "used_today_minutes": 30
      }
    ]
  }
}
```

**GET `/profiles/:id/usage`**
```json
// Query: ?from=2026-05-15&to=2026-05-22
// Response 200
{
  "usage": [
    { "date": "2026-05-22", "used_minutes": 78, "limit_minutes": 120, "adjustments_minutes": 0 },
    { "date": "2026-05-21", "used_minutes": 115, "limit_minutes": 120, "adjustments_minutes": 0 },
    { "date": "2026-05-20", "used_minutes": 240, "limit_minutes": 240, "adjustments_minutes": 30 }
  ],
  "by_agent": [
    {
      "agent_name": "Big PC One",
      "daily": [
        { "date": "2026-05-22", "used_minutes": 48 },
        { "date": "2026-05-21", "used_minutes": 60 }
      ]
    }
  ]
}
```

**GET `/dashboard`**
```json
// Response 200
{
  "profiles": [
    {
      "id": "uuid",
      "display_name": "Tom",
      "remaining_minutes": 42,
      "limit_minutes": 120,
      "used_minutes": 78,
      "enforce": "allow",
      "agents_online": 1,
      "agents_total": 2
    },
    {
      "id": "uuid",
      "display_name": "Alice",
      "remaining_minutes": 180,
      "limit_minutes": 240,
      "used_minutes": 60,
      "enforce": "allow",
      "agents_online": 1,
      "agents_total": 1
    }
  ],
  "pending_agents": 1
}
```

### 6.6 Systemd Unit

```ini
# /etc/systemd/system/parental-controller-server.service
[Unit]
Description=Parental Controller Management Server
After=network-online.target postgresql.service
Wants=network-online.target

[Service]
Type=notify
ExecStart=/usr/local/bin/parental-controller-server
Restart=always
RestartSec=5
WatchdogSec=60
EnvironmentFile=/etc/parental-controller/server.env

ProtectSystem=strict
ReadWritePaths=/var/lib/parental-controller
NoNewPrivileges=true
PrivateTmp=true

[Install]
WantedBy=multi-user.target
```

---

## 7. Pairing & Discovery

### 7.1 Discovery Methods

**mDNS (default, LAN):**
- Server advertises: `_parental-controller._tcp.local` on configured port.
- Agent browses for this service type.
- Works on same subnet only.

**Direct URL (configured):**
- Agent config specifies `server_url = "wss://192.168.1.100:8443"`.
- No discovery needed; agent connects directly.

Both methods lead to the same pairing flow.

### 7.2 Pairing Flow

```
Agent (unpaired)                          Server                          Admin (UI)
     │                                       │                               │
     ├── discovers server via mDNS ─────────►│                               │
     │   or connects to configured URL       │                               │
     │                                       │                               │
     ├── WSS connect (no auth token) ───────►│                               │
     ├── pairing_request ──────────────────►│                               │
     │   { machine_id, hostname,            │                               │
     │     pairing_code: "A7X3" }           │                               │
     │                                       ├── stores as pending agent     │
     │                                       │                               │
     │                                       │◄── GET /agents ───────────────┤
     │                                       │    (shows pending agent       │
     │                                       │     with pairing code)        │
     │                                       │                               │
     │                                       │◄── POST /agents/:id/accept ──┤
     │                                       │    (admin sees code "A7X3"    │
     │                                       │     displayed on agent's      │
     │                                       │     terminal, confirms match) │
     │                                       │                               │
     │◄── pairing_accepted ─────────────────┤                               │
     │    { agent_id, auth_token }          │                               │
     │                                       │                               │
     ├── stores token in SQLite              │                               │
     ├── reconnects with auth ──────────────►│                               │
     ├── agent_hello ──────────────────────►│                               │
     │   (normal operation begins)           │                               │
```

- The **pairing code** is a short random string (4-6 alphanumeric chars) displayed on the agent's terminal/journal. The admin sees it in the UI next to the pending agent and confirms the match. This prevents rogue agents from pairing.
- The **auth_token** is a long-lived random token (256-bit). Server stores its hash; agent stores the raw token. Used for all future WSS connections.
- Agent will **not** accept pairing with a different server unless an admin resets its state via CLI: `parental-controller-agent --reset`.

---

## 8. Sync & Conflict Resolution

### 8.1 Data Ownership

| Data | Authority | Direction | On reconnect |
|------|-----------|-----------|--------------|
| Schedules, limits, adjustments | **Server** | Server → Agent | Agent replaces its cache with server's version |
| Usage data | **Agent** | Agent → Server | Server accepts and aggregates |
| Local user list | **Agent** | Agent → Server | Server updates its records |
| Remaining time | **Server** (calculated) | Server → Agent | Server sends corrected remaining |

### 8.2 Config Versioning

- Each profile has a `config_versions` row with an integer `version`.
- Any change to schedules, daily_limits, time_adjustments, or enforcement_settings bumps the version.
- Agent stores `last_applied_config_version` in `config_meta`.
- On `agent_hello`, if agent's version < server's version → full `config_push`.
- On config change while agent is online → immediate `config_push`.

### 8.3 Reconnect Sequence

1. Agent reconnects WSS with auth token.
2. Agent sends `agent_hello` (includes `last_config_version`).
3. Agent sends `usage_sync` with all `synced=0` usage records.
4. Server ingests usage, recalculates remaining for all affected profiles.
5. Server sends `config_push` if version is stale.
6. Server sends `remaining_update` with corrected values.
7. Agent marks synced usage records as `synced=1`.
8. Agent replaces cached config with server's config.
9. Normal heartbeat loop resumes.

---

## 9. Enforcement Logic

### 9.1 Core Algorithm (runs on agent, uses server-provided data when online)

```
fn evaluate_enforcement(user, now, timezone) -> EnforceAction:

    // 1. Get remaining time
    if online:
        remaining = server_remaining[user].remaining_minutes
        enforce = server_remaining[user].enforce
        return enforce  // trust server's decision
    else:
        // Offline: calculate locally
        today = now.date()
        weekday = today.weekday()  // 0=Mon

        // 2. Check schedule
        windows = cached_schedules.filter(user, weekday)
        if windows.is_empty():
            in_window = true
            window_remaining = unlimited
        else:
            current_window = windows.find(|w| w.start <= now.time() <= w.end)
            if current_window.is_none():
                return Lock  // outside all windows
            window_remaining = current_window.end - now.time()

        // 3. Check daily limit
        limit = cached_daily_limits.get(user, weekday)
        if limit.is_none():
            time_remaining = unlimited
        else:
            used = usage_log.get(user, today).used_seconds / 60
            adjustments = cached_adjustments.get(user, today).adjustment_minutes
            time_remaining = limit.allowed_minutes + adjustments - used

        // 4. Effective remaining
        remaining = min(time_remaining, window_remaining)

        // 5. Determine action
        if remaining <= 0:
            return Lock
        if remaining <= min(warning_thresholds):
            return Warn
        return Allow
```

### 9.2 Lock Execution

1. Send desktop notification: "Your session will be locked in {grace} minutes." (future; log for now).
2. Wait `lockout_grace_minutes`.
3. Call DBus: `org.freedesktop.login1.Session.Lock()` on all sessions for the user.
4. If session still active after 30 seconds: call `org.freedesktop.login1.Session.Terminate()`.

### 9.3 Unlock / Session Resume

- When a new schedule window opens or a time adjustment grants more time:
  - If user is currently locked: the lock persists (the user can log back in).
  - The agent does **not** automatically unlock — the user must re-authenticate at the login screen.
  - This is intentional: it prevents a child from simply waiting for the next window without re-entering their password.

---

## 10. Bootstrapping / First-Run Flow

### 10.1 Server First Run

1. Server starts, connects to PostgreSQL.
2. Runs migrations: creates all tables.
3. Detects no rows in `admin_users`.
4. Logs: "No admin account configured. Create one via POST /api/v1/auth/setup".
5. The `/auth/setup` endpoint is only accessible when `admin_users` is empty.
6. Admin hits `/auth/setup` with username + password → account created.
7. All subsequent requests require JWT from `/auth/login`.

### 10.2 Agent First Run

1. Agent starts, creates SQLite DB and tables.
2. No `server_connection` row → enters discovery mode.
3. Scans for mDNS service `_parental-controller._tcp` (or uses configured URL).
4. Connects to discovered/configured server via WSS (without auth).
5. Generates a pairing code, displays it in terminal/journal.
6. Sends `pairing_request` to server.
7. Waits for `pairing_accepted`.
8. Stores auth token and agent_id in SQLite.
9. Reconnects with auth, begins normal operation.

### 10.3 First User Setup

1. Agent sends `user_list_update` with detected local users.
2. Admin sees new agent_users in the UI (all status: "unmanaged").
3. Admin creates a user profile (e.g., "Tom").
4. Admin links agent_users to the profile (e.g., tom@bigPcOne → profile "Tom").
5. Admin sets schedules and daily limits for the profile.
6. Server sends `config_push` to the agent.
7. Agent begins enforcing.

---

## 11. Future Considerations

These are **not in scope** for the initial implementation but are noted here to avoid
design decisions that would block them:

1. **Desktop notifications**: Agent sends warnings to the user's desktop session before lockout. Requires detecting the user's notification bus (D-Bus session bus per user). The `warning_thresholds` field is already in the data model for this.

2. **Network/content filtering**: DNS proxy or nftables rules per user. Would be a separate enforcement module on the agent.

3. **Per-agent schedule overrides**: A profile-level schedule is the default, but an admin might want to allow different hours on the desktop vs laptop. Would require an override table with `(agent_user_id, day_of_week, start_time, end_time)` that takes precedence over profile-level schedules.

4. **Mobile app**: The REST API is designed to be consumed by any client. A Flutter or React Native app can use it directly.

5. **Multi-admin**: The `admin_users` table already supports multiple admins. Role-based access (e.g., read-only parent) could be added later.

6. **Agent auto-update**: Server could host agent binaries and push update notifications via WSS.

7. **Encrypted local DB**: Agent's SQLite could be encrypted (via SQLCipher) to prevent a child from reading cached rules. Low priority — if the child has root they can intercept DBus calls anyway.

---

## Project Structure (recommended)

```
parental-controller/
├── Cargo.toml                          # Workspace root
├── crates/
│   ├── common/                         # Shared types, message definitions
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── messages.rs             # WSS message types
│   │       ├── models.rs              # Shared domain types
│   │       └── protocol.rs            # Message envelope, serialization
│   ├── agent/                          # Enforcement agent binary
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── main.rs
│   │       ├── config.rs              # Agent config file parsing
│   │       ├── db.rs                  # SQLite operations
│   │       ├── dbus.rs                # logind session monitoring
│   │       ├── discovery.rs           # mDNS browsing
│   │       ├── enforcement.rs         # Lock/warn/allow logic
│   │       ├── heartbeat.rs           # Heartbeat loop
│   │       ├── pairing.rs             # Pairing flow
│   │       ├── users.rs              # Local user scanning
│   │       └── ws_client.rs          # WSS client connection
│   └── server/                         # Management server binary
│       ├── Cargo.toml
│       └── src/
│           ├── main.rs
│           ├── config.rs              # Server config file parsing
│           ├── db.rs                  # PostgreSQL operations
│           ├── api/                   # REST API handlers
│           │   ├── mod.rs
│           │   ├── auth.rs
│           │   ├── agents.rs
│           │   ├── profiles.rs
│           │   ├── schedules.rs
│           │   ├── limits.rs
│           │   ├── adjustments.rs
│           │   ├── usage.rs
│           │   └── dashboard.rs
│           ├── ws_hub.rs              # WSS connection manager
│           ├── ws_handler.rs          # WSS message handler
│           ├── engine.rs             # Remaining time calculation
│           ├── discovery.rs           # mDNS advertisement
│           ├── auth.rs               # JWT, password hashing
│           └── audit.rs             # Audit logging
├── migrations/                         # PostgreSQL migrations (sqlx)
└── README.md
```

---

*End of specification.*
