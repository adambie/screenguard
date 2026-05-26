use anyhow::{Context, Result};
use chrono::{NaiveDate, NaiveTime, Utc};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub type DbPool = Pool<SqliteConnectionManager>;

// ── open / schema ─────────────────────────────────────────────────────────────

pub fn open(path: &str) -> Result<DbPool> {
    if let Some(parent) = std::path::Path::new(path).parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create DB directory: {}", parent.display()))?;
    }
    let manager = SqliteConnectionManager::file(path);
    let pool = Pool::new(manager).context("Failed to create DB pool")?;
    let conn = pool.get().context("Failed to get DB connection")?;
    conn.execute_batch(SCHEMA)?;
    Ok(pool)
}

const SCHEMA: &str = "
PRAGMA journal_mode=WAL;
PRAGMA foreign_keys=ON;

CREATE TABLE IF NOT EXISTS admin_users (
    id              TEXT PRIMARY KEY,
    username        TEXT NOT NULL UNIQUE,
    password_hash   TEXT NOT NULL,
    created_at      INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS agents (
    id              TEXT PRIMARY KEY,
    machine_id      TEXT NOT NULL UNIQUE,
    display_name    TEXT NOT NULL,
    hostname        TEXT NOT NULL,
    timezone        TEXT NOT NULL DEFAULT 'UTC',
    status          TEXT NOT NULL DEFAULT 'pending'
                    CHECK (status IN ('pending','paired','disabled')),
    auth_token_hash TEXT,
    agent_version   TEXT,
    paired_at       INTEGER,
    last_seen_at    INTEGER,
    created_at      INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS user_profiles (
    id              TEXT PRIMARY KEY,
    display_name    TEXT NOT NULL,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS agent_users (
    id              TEXT PRIMARY KEY,
    agent_id        TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    profile_id      TEXT REFERENCES user_profiles(id) ON DELETE SET NULL,
    local_uid       INTEGER NOT NULL,
    local_username  TEXT NOT NULL,
    display_name    TEXT,
    status          TEXT NOT NULL DEFAULT 'unmanaged'
                    CHECK (status IN ('unmanaged','managed','deleted')),
    first_seen_at   INTEGER NOT NULL,
    last_reported_at INTEGER NOT NULL,
    UNIQUE(agent_id, local_uid)
);

CREATE TABLE IF NOT EXISTS schedules (
    id              TEXT PRIMARY KEY,
    profile_id      TEXT NOT NULL REFERENCES user_profiles(id) ON DELETE CASCADE,
    day_of_week     INTEGER NOT NULL CHECK (day_of_week BETWEEN 0 AND 6),
    start_time      TEXT NOT NULL,
    end_time        TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS daily_limits (
    profile_id      TEXT NOT NULL REFERENCES user_profiles(id) ON DELETE CASCADE,
    day_of_week     INTEGER NOT NULL CHECK (day_of_week BETWEEN 0 AND 6),
    allowed_minutes INTEGER NOT NULL CHECK (allowed_minutes > 0),
    PRIMARY KEY (profile_id, day_of_week)
);

CREATE TABLE IF NOT EXISTS time_adjustments (
    id              TEXT PRIMARY KEY,
    profile_id      TEXT NOT NULL REFERENCES user_profiles(id) ON DELETE CASCADE,
    target_date     TEXT NOT NULL,
    adjustment_minutes INTEGER NOT NULL,
    reason          TEXT,
    created_by      TEXT REFERENCES admin_users(id),
    created_at      INTEGER NOT NULL,
    synced_to_agents INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS enforcement_settings (
    profile_id              TEXT PRIMARY KEY REFERENCES user_profiles(id) ON DELETE CASCADE,
    lockout_grace_minutes   INTEGER NOT NULL DEFAULT 5,
    warning_thresholds      TEXT NOT NULL DEFAULT '15,5,1'
);

CREATE TABLE IF NOT EXISTS daily_usage (
    agent_user_id   TEXT NOT NULL REFERENCES agent_users(id) ON DELETE CASCADE,
    date            TEXT NOT NULL,
    used_seconds    INTEGER NOT NULL DEFAULT 0,
    reported_at     INTEGER NOT NULL,
    PRIMARY KEY (agent_user_id, date)
);

CREATE TABLE IF NOT EXISTS config_versions (
    profile_id      TEXT PRIMARY KEY REFERENCES user_profiles(id) ON DELETE CASCADE,
    version         INTEGER NOT NULL DEFAULT 1,
    updated_at      INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS audit_log (
    id              TEXT PRIMARY KEY,
    admin_user_id   TEXT REFERENCES admin_users(id),
    action          TEXT NOT NULL,
    target_type     TEXT,
    target_id       TEXT,
    detail          TEXT,
    created_at      INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_agent_users_agent    ON agent_users(agent_id);
CREATE INDEX IF NOT EXISTS idx_agent_users_profile  ON agent_users(profile_id);
CREATE INDEX IF NOT EXISTS idx_schedules_profile    ON schedules(profile_id);
CREATE INDEX IF NOT EXISTS idx_daily_usage_date     ON daily_usage(date);
CREATE INDEX IF NOT EXISTS idx_adjustments_profile  ON time_adjustments(profile_id, target_date);
";

// ── models ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminUser {
    pub id: Uuid,
    pub username: String,
    pub password_hash: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    pub id: Uuid,
    pub machine_id: String,
    pub display_name: String,
    pub hostname: String,
    pub timezone: String,
    pub status: String,
    pub auth_token_hash: Option<String>,
    pub agent_version: Option<String>,
    pub paired_at: Option<i64>,
    pub last_seen_at: Option<i64>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserProfile {
    pub id: Uuid,
    pub display_name: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentUser {
    pub id: Uuid,
    pub agent_id: Uuid,
    pub profile_id: Option<Uuid>,
    pub local_uid: i64,
    pub local_username: String,
    pub display_name: Option<String>,
    pub status: String,
    pub first_seen_at: i64,
    pub last_reported_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schedule {
    pub id: Uuid,
    pub profile_id: Uuid,
    pub day_of_week: u8,
    pub start_time: String,
    pub end_time: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyLimit {
    pub profile_id: Uuid,
    pub day_of_week: u8,
    pub allowed_minutes: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeAdjustment {
    pub id: Uuid,
    pub profile_id: Uuid,
    pub target_date: String,
    pub adjustment_minutes: i32,
    pub reason: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct EnforcementSettings {
    pub lockout_grace_minutes: i32,
    pub warning_thresholds: Vec<i32>,
}

// ── admin_users ───────────────────────────────────────────────────────────────

pub fn admin_count(pool: &DbPool) -> Result<i64> {
    let conn = pool.get()?;
    Ok(conn.query_row("SELECT COUNT(*) FROM admin_users", [], |r| r.get(0))?)
}

pub fn create_admin(pool: &DbPool, username: &str, password_hash: &str) -> Result<Uuid> {
    let id = Uuid::new_v4();
    let conn = pool.get()?;
    conn.execute(
        "INSERT INTO admin_users (id, username, password_hash, created_at) VALUES (?1,?2,?3,?4)",
        params![id.to_string(), username, password_hash, Utc::now().timestamp()],
    )?;
    Ok(id)
}

pub fn get_admin_by_username(pool: &DbPool, username: &str) -> Result<Option<AdminUser>> {
    let conn = pool.get()?;
    conn.query_row(
        "SELECT id, username, password_hash, created_at FROM admin_users WHERE username=?1",
        params![username],
        |r| Ok(AdminUser {
            id: r.get::<_, String>(0)?.parse().unwrap_or_default(),
            username: r.get(1)?,
            password_hash: r.get(2)?,
            created_at: r.get(3)?,
        }),
    ).optional().map_err(Into::into)
}

// ── agents ────────────────────────────────────────────────────────────────────

pub fn upsert_agent_pending(
    pool: &DbPool,
    machine_id: &str,
    hostname: &str,
    timezone: &str,
    agent_version: &str,
) -> Result<Agent> {
    let conn = pool.get()?;
    let now = Utc::now().timestamp();
    // Return existing agent if already known (re-pairing attempt).
    let existing: Option<String> = conn
        .query_row(
            "SELECT id FROM agents WHERE machine_id=?1",
            params![machine_id],
            |r| r.get(0),
        )
        .optional()?;

    let id = if let Some(id_str) = existing {
        conn.execute(
            "UPDATE agents SET hostname=?1, timezone=?2, agent_version=?3, last_seen_at=?4,
             status=CASE WHEN status='disabled' THEN 'disabled' ELSE 'pending' END
             WHERE machine_id=?5",
            params![hostname, timezone, agent_version, now, machine_id],
        )?;
        id_str.parse().unwrap_or_else(|_| Uuid::new_v4())
    } else {
        let new_id = Uuid::new_v4();
        conn.execute(
            "INSERT INTO agents (id,machine_id,display_name,hostname,timezone,status,agent_version,created_at,last_seen_at)
             VALUES (?1,?2,?3,?4,?5,'pending',?6,?7,?7)",
            params![new_id.to_string(), machine_id, hostname, hostname, timezone, agent_version, now],
        )?;
        new_id
    };
    get_agent_by_id(pool, id)?.context("Agent not found after upsert")
}

pub fn get_agent_by_id(pool: &DbPool, id: Uuid) -> Result<Option<Agent>> {
    let conn = pool.get()?;
    conn.query_row(
        "SELECT id,machine_id,display_name,hostname,timezone,status,auth_token_hash,
                agent_version,paired_at,last_seen_at,created_at
         FROM agents WHERE id=?1",
        params![id.to_string()],
        row_to_agent,
    ).optional().map_err(Into::into)
}

pub fn get_agent_by_machine_id(pool: &DbPool, machine_id: &str) -> Result<Option<Agent>> {
    let conn = pool.get()?;
    conn.query_row(
        "SELECT id,machine_id,display_name,hostname,timezone,status,auth_token_hash,
                agent_version,paired_at,last_seen_at,created_at
         FROM agents WHERE machine_id=?1",
        params![machine_id],
        row_to_agent,
    ).optional().map_err(Into::into)
}

pub fn list_agents(pool: &DbPool) -> Result<Vec<Agent>> {
    let conn = pool.get()?;
    let mut stmt = conn.prepare(
        "SELECT id,machine_id,display_name,hostname,timezone,status,auth_token_hash,
                agent_version,paired_at,last_seen_at,created_at
         FROM agents ORDER BY created_at DESC",
    )?;
    let rows = stmt.query_map([], row_to_agent)?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

pub fn accept_agent(pool: &DbPool, id: Uuid, auth_token_hash: &str) -> Result<()> {
    let conn = pool.get()?;
    let now = Utc::now().timestamp();
    conn.execute(
        "UPDATE agents SET status='paired', auth_token_hash=?1, paired_at=?2 WHERE id=?3",
        params![auth_token_hash, now, id.to_string()],
    )?;
    Ok(())
}

pub fn update_agent_last_seen(pool: &DbPool, id: Uuid) -> Result<()> {
    let conn = pool.get()?;
    conn.execute(
        "UPDATE agents SET last_seen_at=?1 WHERE id=?2",
        params![Utc::now().timestamp(), id.to_string()],
    )?;
    Ok(())
}

pub fn update_agent_fields(
    pool: &DbPool,
    id: Uuid,
    display_name: Option<&str>,
    status: Option<&str>,
) -> Result<()> {
    let conn = pool.get()?;
    if let Some(name) = display_name {
        conn.execute("UPDATE agents SET display_name=?1 WHERE id=?2", params![name, id.to_string()])?;
    }
    if let Some(s) = status {
        conn.execute("UPDATE agents SET status=?1 WHERE id=?2", params![s, id.to_string()])?;
    }
    Ok(())
}

pub fn update_agent_hello(
    pool: &DbPool,
    id: Uuid,
    hostname: &str,
    timezone: &str,
    agent_version: &str,
) -> Result<()> {
    let conn = pool.get()?;
    conn.execute(
        "UPDATE agents SET hostname=?1, timezone=?2, agent_version=?3, last_seen_at=?4 WHERE id=?5",
        params![hostname, timezone, agent_version, Utc::now().timestamp(), id.to_string()],
    )?;
    Ok(())
}

pub fn delete_agent(pool: &DbPool, id: Uuid) -> Result<()> {
    let conn = pool.get()?;
    conn.execute("DELETE FROM agents WHERE id=?1", params![id.to_string()])?;
    Ok(())
}

fn row_to_agent(r: &rusqlite::Row<'_>) -> rusqlite::Result<Agent> {
    Ok(Agent {
        id: r.get::<_, String>(0)?.parse().unwrap_or_default(),
        machine_id: r.get(1)?,
        display_name: r.get(2)?,
        hostname: r.get(3)?,
        timezone: r.get(4)?,
        status: r.get(5)?,
        auth_token_hash: r.get(6)?,
        agent_version: r.get(7)?,
        paired_at: r.get(8)?,
        last_seen_at: r.get(9)?,
        created_at: r.get(10)?,
    })
}

// ── agent_users ───────────────────────────────────────────────────────────────

pub fn upsert_agent_users(
    pool: &DbPool,
    agent_id: Uuid,
    users: &[common::models::LocalUser],
) -> Result<()> {
    let conn = pool.get()?;
    let now = Utc::now().timestamp();
    for u in users {
        let existing: Option<String> = conn
            .query_row(
                "SELECT id FROM agent_users WHERE agent_id=?1 AND local_uid=?2",
                params![agent_id.to_string(), u.local_uid],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(_) = existing {
            conn.execute(
                "UPDATE agent_users SET local_username=?1, display_name=?2, last_reported_at=?3,
                 status=CASE WHEN status='deleted' THEN 'unmanaged' ELSE status END
                 WHERE agent_id=?4 AND local_uid=?5",
                params![u.username, u.display_name, now, agent_id.to_string(), u.local_uid],
            )?;
        } else {
            let id = Uuid::new_v4();
            conn.execute(
                "INSERT INTO agent_users (id,agent_id,local_uid,local_username,display_name,status,first_seen_at,last_reported_at)
                 VALUES (?1,?2,?3,?4,?5,'unmanaged',?6,?6)",
                params![id.to_string(), agent_id.to_string(), u.local_uid, u.username, u.display_name, now],
            )?;
        }
    }
    Ok(())
}

pub fn mark_agent_users_deleted(pool: &DbPool, agent_id: Uuid, uids: &[u32]) -> Result<()> {
    let conn = pool.get()?;
    for uid in uids {
        conn.execute(
            "UPDATE agent_users SET status='deleted' WHERE agent_id=?1 AND local_uid=?2",
            params![agent_id.to_string(), uid],
        )?;
    }
    Ok(())
}

pub fn list_agent_users(pool: &DbPool, agent_id: Uuid) -> Result<Vec<AgentUser>> {
    let conn = pool.get()?;
    let mut stmt = conn.prepare(
        "SELECT id,agent_id,profile_id,local_uid,local_username,display_name,status,first_seen_at,last_reported_at
         FROM agent_users WHERE agent_id=?1 AND status!='deleted' ORDER BY local_uid",
    )?;
    let rows = stmt.query_map(params![agent_id.to_string()], row_to_agent_user)?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

pub fn get_agent_user_by_id(pool: &DbPool, id: Uuid) -> Result<Option<AgentUser>> {
    let conn = pool.get()?;
    conn.query_row(
        "SELECT id,agent_id,profile_id,local_uid,local_username,display_name,status,first_seen_at,last_reported_at
         FROM agent_users WHERE id=?1",
        params![id.to_string()],
        row_to_agent_user,
    ).optional().map_err(Into::into)
}

pub fn get_agent_user(pool: &DbPool, agent_id: Uuid, local_uid: u32) -> Result<Option<AgentUser>> {
    let conn = pool.get()?;
    conn.query_row(
        "SELECT id,agent_id,profile_id,local_uid,local_username,display_name,status,first_seen_at,last_reported_at
         FROM agent_users WHERE agent_id=?1 AND local_uid=?2",
        params![agent_id.to_string(), local_uid],
        row_to_agent_user,
    ).optional().map_err(Into::into)
}

pub fn update_agent_user(
    pool: &DbPool,
    id: Uuid,
    profile_id: Option<Uuid>,
    status: Option<&str>,
) -> Result<()> {
    let conn = pool.get()?;
    let profile_str = profile_id.map(|p| p.to_string());
    conn.execute(
        "UPDATE agent_users SET profile_id=COALESCE(?1, profile_id),
         status=COALESCE(?2, status) WHERE id=?3",
        params![profile_str, status, id.to_string()],
    )?;
    Ok(())
}

/// Returns all agent_user IDs (and their agent_ids) linked to a profile.
pub fn get_agent_users_for_profile(pool: &DbPool, profile_id: Uuid) -> Result<Vec<AgentUser>> {
    let conn = pool.get()?;
    let mut stmt = conn.prepare(
        "SELECT id,agent_id,profile_id,local_uid,local_username,display_name,status,first_seen_at,last_reported_at
         FROM agent_users WHERE profile_id=?1 AND status='managed'",
    )?;
    let rows = stmt.query_map(params![profile_id.to_string()], row_to_agent_user)?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

fn row_to_agent_user(r: &rusqlite::Row<'_>) -> rusqlite::Result<AgentUser> {
    Ok(AgentUser {
        id: r.get::<_, String>(0)?.parse().unwrap_or_default(),
        agent_id: r.get::<_, String>(1)?.parse().unwrap_or_default(),
        profile_id: r.get::<_, Option<String>>(2)?.and_then(|s| s.parse().ok()),
        local_uid: r.get(3)?,
        local_username: r.get(4)?,
        display_name: r.get(5)?,
        status: r.get(6)?,
        first_seen_at: r.get(7)?,
        last_reported_at: r.get(8)?,
    })
}

// ── user_profiles ─────────────────────────────────────────────────────────────

pub fn create_profile(pool: &DbPool, display_name: &str) -> Result<UserProfile> {
    let id = Uuid::new_v4();
    let now = Utc::now().timestamp();
    let conn = pool.get()?;
    conn.execute(
        "INSERT INTO user_profiles (id, display_name, created_at, updated_at) VALUES (?1,?2,?3,?3)",
        params![id.to_string(), display_name, now],
    )?;
    conn.execute(
        "INSERT INTO enforcement_settings (profile_id) VALUES (?1)",
        params![id.to_string()],
    )?;
    conn.execute(
        "INSERT INTO config_versions (profile_id, version, updated_at) VALUES (?1, 1, ?2)",
        params![id.to_string(), now],
    )?;
    Ok(UserProfile { id, display_name: display_name.to_string(), created_at: now, updated_at: now })
}

pub fn list_profiles(pool: &DbPool) -> Result<Vec<UserProfile>> {
    let conn = pool.get()?;
    let mut stmt = conn.prepare(
        "SELECT id, display_name, created_at, updated_at FROM user_profiles ORDER BY display_name",
    )?;
    let rows = stmt.query_map([], |r| Ok(UserProfile {
        id: r.get::<_, String>(0)?.parse().unwrap_or_default(),
        display_name: r.get(1)?,
        created_at: r.get(2)?,
        updated_at: r.get(3)?,
    }))?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

pub fn get_profile(pool: &DbPool, id: Uuid) -> Result<Option<UserProfile>> {
    let conn = pool.get()?;
    conn.query_row(
        "SELECT id, display_name, created_at, updated_at FROM user_profiles WHERE id=?1",
        params![id.to_string()],
        |r| Ok(UserProfile {
            id: r.get::<_, String>(0)?.parse().unwrap_or_default(),
            display_name: r.get(1)?,
            created_at: r.get(2)?,
            updated_at: r.get(3)?,
        }),
    ).optional().map_err(Into::into)
}

pub fn update_profile(pool: &DbPool, id: Uuid, display_name: &str) -> Result<()> {
    let conn = pool.get()?;
    conn.execute(
        "UPDATE user_profiles SET display_name=?1, updated_at=?2 WHERE id=?3",
        params![display_name, Utc::now().timestamp(), id.to_string()],
    )?;
    Ok(())
}

pub fn delete_profile(pool: &DbPool, id: Uuid) -> Result<()> {
    let conn = pool.get()?;
    conn.execute("DELETE FROM user_profiles WHERE id=?1", params![id.to_string()])?;
    Ok(())
}

// ── schedules ─────────────────────────────────────────────────────────────────

pub fn get_schedules(pool: &DbPool, profile_id: Uuid) -> Result<Vec<Schedule>> {
    let conn = pool.get()?;
    let mut stmt = conn.prepare(
        "SELECT id, profile_id, day_of_week, start_time, end_time FROM schedules WHERE profile_id=?1 ORDER BY day_of_week, start_time",
    )?;
    let rows = stmt.query_map(params![profile_id.to_string()], |r| Ok(Schedule {
        id: r.get::<_, String>(0)?.parse().unwrap_or_default(),
        profile_id: r.get::<_, String>(1)?.parse().unwrap_or_default(),
        day_of_week: r.get::<_, i64>(2)? as u8,
        start_time: r.get(3)?,
        end_time: r.get(4)?,
    }))?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

pub fn replace_schedules(pool: &DbPool, profile_id: Uuid, schedules: &[(u8, &str, &str)]) -> Result<()> {
    let conn = pool.get()?;
    conn.execute("DELETE FROM schedules WHERE profile_id=?1", params![profile_id.to_string()])?;
    for (dow, start, end) in schedules {
        conn.execute(
            "INSERT INTO schedules (id, profile_id, day_of_week, start_time, end_time) VALUES (?1,?2,?3,?4,?5)",
            params![Uuid::new_v4().to_string(), profile_id.to_string(), dow, start, end],
        )?;
    }
    Ok(())
}

// ── daily_limits ──────────────────────────────────────────────────────────────

pub fn get_daily_limits(pool: &DbPool, profile_id: Uuid) -> Result<Vec<DailyLimit>> {
    let conn = pool.get()?;
    let mut stmt = conn.prepare(
        "SELECT profile_id, day_of_week, allowed_minutes FROM daily_limits WHERE profile_id=?1 ORDER BY day_of_week",
    )?;
    let rows = stmt.query_map(params![profile_id.to_string()], |r| Ok(DailyLimit {
        profile_id: r.get::<_, String>(0)?.parse().unwrap_or_default(),
        day_of_week: r.get::<_, i64>(1)? as u8,
        allowed_minutes: r.get(2)?,
    }))?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

pub fn replace_daily_limits(pool: &DbPool, profile_id: Uuid, limits: &[(u8, i32)]) -> Result<()> {
    let conn = pool.get()?;
    conn.execute("DELETE FROM daily_limits WHERE profile_id=?1", params![profile_id.to_string()])?;
    for (dow, minutes) in limits {
        conn.execute(
            "INSERT INTO daily_limits (profile_id, day_of_week, allowed_minutes) VALUES (?1,?2,?3)",
            params![profile_id.to_string(), dow, minutes],
        )?;
    }
    Ok(())
}

// ── time_adjustments ──────────────────────────────────────────────────────────

pub fn get_adjustments(pool: &DbPool, profile_id: Uuid, from: Option<&str>, to: Option<&str>) -> Result<Vec<TimeAdjustment>> {
    let conn = pool.get()?;
    let from_val = from.unwrap_or("0000-00-00");
    let to_val = to.unwrap_or("9999-12-31");
    let mut stmt = conn.prepare(
        "SELECT id,profile_id,target_date,adjustment_minutes,reason,created_at
         FROM time_adjustments WHERE profile_id=?1 AND target_date>=?2 AND target_date<=?3
         ORDER BY target_date DESC",
    )?;
    let rows = stmt.query_map(
        params![profile_id.to_string(), from_val, to_val],
        |r| Ok(TimeAdjustment {
            id: r.get::<_, String>(0)?.parse().unwrap_or_default(),
            profile_id: r.get::<_, String>(1)?.parse().unwrap_or_default(),
            target_date: r.get(2)?,
            adjustment_minutes: r.get(3)?,
            reason: r.get(4)?,
            created_at: r.get(5)?,
        }),
    )?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

pub fn latest_adjustment_reason_for_date(pool: &DbPool, profile_id: Uuid, date: &str) -> Result<Option<String>> {
    let conn = pool.get()?;
    let reason: Option<String> = conn.query_row(
        "SELECT reason FROM time_adjustments WHERE profile_id=?1 AND target_date=?2
         ORDER BY created_at DESC LIMIT 1",
        params![profile_id.to_string(), date],
        |r| r.get(0),
    ).optional()?;
    Ok(reason)
}

pub fn sum_adjustments_for_date(pool: &DbPool, profile_id: Uuid, date: &str) -> Result<i32> {
    let conn = pool.get()?;
    Ok(conn.query_row(
        "SELECT COALESCE(SUM(adjustment_minutes),0) FROM time_adjustments WHERE profile_id=?1 AND target_date=?2",
        params![profile_id.to_string(), date],
        |r| r.get(0),
    )?)
}

pub fn create_adjustment(
    pool: &DbPool,
    profile_id: Uuid,
    target_date: &str,
    minutes: i32,
    reason: Option<&str>,
    created_by: Option<Uuid>,
) -> Result<Uuid> {
    let id = Uuid::new_v4();
    let conn = pool.get()?;
    conn.execute(
        "INSERT INTO time_adjustments (id,profile_id,target_date,adjustment_minutes,reason,created_by,created_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7)",
        params![
            id.to_string(), profile_id.to_string(), target_date, minutes, reason,
            created_by.map(|u| u.to_string()), Utc::now().timestamp()
        ],
    )?;
    Ok(id)
}

// ── enforcement_settings ──────────────────────────────────────────────────────

pub fn get_enforcement_settings(pool: &DbPool, profile_id: Uuid) -> Result<EnforcementSettings> {
    let conn = pool.get()?;
    let (grace, thresholds_str): (i32, String) = conn.query_row(
        "SELECT lockout_grace_minutes, warning_thresholds FROM enforcement_settings WHERE profile_id=?1",
        params![profile_id.to_string()],
        |r| Ok((r.get(0)?, r.get(1)?)),
    ).unwrap_or((5, "15,5,1".to_string()));
    let thresholds = thresholds_str.split(',').filter_map(|s| s.trim().parse().ok()).collect();
    Ok(EnforcementSettings { lockout_grace_minutes: grace, warning_thresholds: thresholds })
}

// ── daily_usage ───────────────────────────────────────────────────────────────

pub fn add_usage_seconds(pool: &DbPool, agent_user_id: Uuid, date: &str, seconds: i64) -> Result<()> {
    let conn = pool.get()?;
    conn.execute(
        "INSERT INTO daily_usage (agent_user_id, date, used_seconds, reported_at)
         VALUES (?1,?2,?3,?4)
         ON CONFLICT(agent_user_id, date) DO UPDATE SET used_seconds=used_seconds+?3, reported_at=?4",
        params![agent_user_id.to_string(), date, seconds, Utc::now().timestamp()],
    )?;
    Ok(())
}

pub fn get_used_seconds_for_profile_today(pool: &DbPool, profile_id: Uuid, date: &str) -> Result<i64> {
    let conn = pool.get()?;
    Ok(conn.query_row(
        "SELECT COALESCE(SUM(du.used_seconds),0)
         FROM daily_usage du
         JOIN agent_users au ON au.id=du.agent_user_id
         WHERE au.profile_id=?1 AND du.date=?2",
        params![profile_id.to_string(), date],
        |r| r.get(0),
    )?)
}

pub fn get_usage_by_agent_for_profile(
    pool: &DbPool,
    profile_id: Uuid,
    from: &str,
    to: &str,
) -> Result<Vec<(Uuid, String, String, i64)>> {
    let conn = pool.get()?;
    let mut stmt = conn.prepare(
        "SELECT au.agent_id, au.id, du.date, du.used_seconds
         FROM daily_usage du
         JOIN agent_users au ON au.id=du.agent_user_id
         WHERE au.profile_id=?1 AND du.date>=?2 AND du.date<=?3
         ORDER BY du.date DESC",
    )?;
    let rows = stmt.query_map(params![profile_id.to_string(), from, to], |r| {
        Ok((
            r.get::<_, String>(0)?.parse().unwrap_or_default(),
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, i64>(3)?,
        ))
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

pub fn get_daily_usage_for_profile(
    pool: &DbPool,
    profile_id: Uuid,
    from: &str,
    to: &str,
) -> Result<Vec<(String, i64)>> {
    let conn = pool.get()?;
    let mut stmt = conn.prepare(
        "SELECT du.date, SUM(du.used_seconds)
         FROM daily_usage du
         JOIN agent_users au ON au.id=du.agent_user_id
         WHERE au.profile_id=?1 AND du.date>=?2 AND du.date<=?3
         GROUP BY du.date ORDER BY du.date DESC",
    )?;
    let rows = stmt.query_map(params![profile_id.to_string(), from, to], |r| {
        Ok((r.get(0)?, r.get(1)?))
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

// ── config_versions ───────────────────────────────────────────────────────────

pub fn get_config_version(pool: &DbPool, profile_id: Uuid) -> Result<i64> {
    let conn = pool.get()?;
    Ok(conn.query_row(
        "SELECT version FROM config_versions WHERE profile_id=?1",
        params![profile_id.to_string()],
        |r| r.get(0),
    ).unwrap_or(1))
}

pub fn bump_config_version(pool: &DbPool, profile_id: Uuid) -> Result<i64> {
    let conn = pool.get()?;
    let now = Utc::now().timestamp();
    conn.execute(
        "INSERT INTO config_versions (profile_id, version, updated_at) VALUES (?1, 1, ?2)
         ON CONFLICT(profile_id) DO UPDATE SET version=version+1, updated_at=?2",
        params![profile_id.to_string(), now],
    )?;
    Ok(conn.query_row(
        "SELECT version FROM config_versions WHERE profile_id=?1",
        params![profile_id.to_string()],
        |r| r.get(0),
    )?)
}

// ── audit_log ─────────────────────────────────────────────────────────────────

#[allow(dead_code)]
pub fn audit(
    pool: &DbPool,
    admin_id: Option<Uuid>,
    action: &str,
    target_type: Option<&str>,
    target_id: Option<Uuid>,
    detail: Option<&str>,
) -> Result<()> {
    let conn = pool.get()?;
    conn.execute(
        "INSERT INTO audit_log (id,admin_user_id,action,target_type,target_id,detail,created_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7)",
        params![
            Uuid::new_v4().to_string(),
            admin_id.map(|u| u.to_string()),
            action,
            target_type,
            target_id.map(|u| u.to_string()),
            detail,
            Utc::now().timestamp()
        ],
    )?;
    Ok(())
}

// ── helpers ───────────────────────────────────────────────────────────────────

pub fn pending_agent_count(pool: &DbPool) -> Result<i64> {
    let conn = pool.get()?;
    Ok(conn.query_row("SELECT COUNT(*) FROM agents WHERE status='pending'", [], |r| r.get(0))?)
}

/// Returns the weekday index (0=Mon … 6=Sun) for a given date string "YYYY-MM-DD".
pub fn weekday_for_date(date: &str) -> u8 {
    use chrono::Datelike;
    NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map(|d| d.weekday().num_days_from_monday() as u8)
        .unwrap_or(0)
}

pub fn parse_time(s: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(s, "%H:%M").ok()
}
