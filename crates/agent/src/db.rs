use anyhow::{Context, Result};
use chrono::NaiveDate;
use rusqlite::{Connection, params};
use std::path::Path;

const DB_PATH: &str = "/var/lib/screenguard/agent.db";
const DB_PATH_ENV: &str = "PARENTAL_AGENT_DB";

pub struct Db {
    conn: Connection,
}

// ── connection & schema ────────────────────────────────────────────────────────

impl Db {
    pub fn open(path: Option<&str>) -> Result<Self> {
        let env_path = std::env::var(DB_PATH_ENV).ok();
        let path = path
            .or(env_path.as_deref())
            .unwrap_or(DB_PATH);
        if let Some(parent) = Path::new(path).parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create DB directory: {}", parent.display()))?;
        }
        let conn = Connection::open(path)
            .with_context(|| format!("Failed to open SQLite DB at: {path}"))?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        let db = Self { conn };
        db.create_schema()?;
        Ok(db)
    }

    fn create_schema(&self) -> Result<()> {
        self.conn.execute_batch(r#"
            CREATE TABLE IF NOT EXISTS server_connection (
                id              INTEGER PRIMARY KEY CHECK (id = 1),
                server_url      TEXT NOT NULL,
                auth_token      TEXT NOT NULL,
                agent_id        TEXT NOT NULL,
                paired_at       INTEGER NOT NULL,
                last_sync_at    INTEGER
            );

            CREATE TABLE IF NOT EXISTS config_meta (
                id              INTEGER PRIMARY KEY CHECK (id = 1),
                config_version  INTEGER NOT NULL DEFAULT 0,
                received_at     INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS managed_users (
                local_uid       INTEGER PRIMARY KEY,
                local_username  TEXT NOT NULL,
                profile_id      TEXT NOT NULL,
                status          TEXT NOT NULL CHECK (status IN ('managed', 'unmanaged'))
            );

            CREATE TABLE IF NOT EXISTS cached_schedules (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                local_uid       INTEGER NOT NULL REFERENCES managed_users(local_uid) ON DELETE CASCADE,
                day_of_week     INTEGER NOT NULL CHECK (day_of_week BETWEEN 0 AND 6),
                start_time      TEXT NOT NULL,
                end_time        TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS cached_daily_limits (
                local_uid       INTEGER NOT NULL REFERENCES managed_users(local_uid) ON DELETE CASCADE,
                day_of_week     INTEGER NOT NULL CHECK (day_of_week BETWEEN 0 AND 6),
                allowed_minutes INTEGER NOT NULL,
                PRIMARY KEY (local_uid, day_of_week)
            );

            CREATE TABLE IF NOT EXISTS cached_adjustments (
                local_uid          INTEGER NOT NULL REFERENCES managed_users(local_uid) ON DELETE CASCADE,
                target_date        TEXT NOT NULL,
                adjustment_minutes INTEGER NOT NULL,
                PRIMARY KEY (local_uid, target_date)
            );

            CREATE TABLE IF NOT EXISTS cached_enforcement (
                local_uid               INTEGER NOT NULL REFERENCES managed_users(local_uid) ON DELETE CASCADE,
                lockout_grace_minutes   INTEGER NOT NULL DEFAULT 5,
                warning_thresholds      TEXT NOT NULL DEFAULT '15,5,1',
                PRIMARY KEY (local_uid)
            );

            CREATE TABLE IF NOT EXISTS usage_log (
                local_uid       INTEGER NOT NULL,
                date            TEXT NOT NULL,
                used_seconds    INTEGER NOT NULL DEFAULT 0,
                synced          INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (local_uid, date)
            );

            CREATE TABLE IF NOT EXISTS server_remaining (
                local_uid           INTEGER PRIMARY KEY,
                remaining_minutes   INTEGER NOT NULL,
                enforce             TEXT NOT NULL CHECK (enforce IN ('allow', 'warn', 'lock')),
                updated_at          INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS active_sessions (
                local_uid       INTEGER NOT NULL,
                session_id      TEXT NOT NULL,
                started_at      INTEGER NOT NULL,
                last_heartbeat  INTEGER NOT NULL,
                is_idle         INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (local_uid, session_id)
            );

            CREATE TABLE IF NOT EXISTS agent_state (
                id              INTEGER PRIMARY KEY CHECK (id = 1),
                mode            TEXT NOT NULL CHECK (mode IN ('unpaired', 'online', 'offline')) DEFAULT 'unpaired',
                offline_since   INTEGER
            );

            INSERT OR IGNORE INTO agent_state (id, mode) VALUES (1, 'unpaired');
        "#)?;
        Ok(())
    }
}

// ── reset ─────────────────────────────────────────────────────────────────────

impl Db {
    pub fn reset_pairing(&self) -> Result<()> {
        self.conn.execute("DELETE FROM server_connection", [])?;
        self.set_agent_mode(AgentMode::Unpaired)?;
        Ok(())
    }
}

// ── server_connection ─────────────────────────────────────────────────────────

pub struct ServerConnection {
    pub server_url: String,
    pub auth_token: String,
    pub agent_id: String,
}

impl Db {
    pub fn get_server_connection(&self) -> Result<Option<ServerConnection>> {
        let mut stmt = self.conn.prepare(
            "SELECT server_url, auth_token, agent_id FROM server_connection WHERE id = 1"
        )?;
        let mut rows = stmt.query([])?;
        if let Some(row) = rows.next()? {
            Ok(Some(ServerConnection {
                server_url: row.get(0)?,
                auth_token: row.get(1)?,
                agent_id: row.get(2)?,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn save_server_connection(&self, sc: &ServerConnection) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        self.conn.execute(
            "INSERT OR REPLACE INTO server_connection (id, server_url, auth_token, agent_id, paired_at)
             VALUES (1, ?1, ?2, ?3, ?4)",
            params![sc.server_url, sc.auth_token, sc.agent_id, now],
        )?;
        Ok(())
    }

    pub fn update_last_sync(&self) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        self.conn.execute(
            "UPDATE server_connection SET last_sync_at = ?1 WHERE id = 1",
            params![now],
        )?;
        Ok(())
    }
}

// ── config_meta ───────────────────────────────────────────────────────────────

impl Db {
    pub fn get_config_version(&self) -> Result<i64> {
        let version: i64 = self.conn.query_row(
            "SELECT config_version FROM config_meta WHERE id = 1",
            [],
            |row| row.get(0),
        ).unwrap_or(0);
        Ok(version)
    }

    pub fn save_config_version(&self, version: i64) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        self.conn.execute(
            "INSERT OR REPLACE INTO config_meta (id, config_version, received_at) VALUES (1, ?1, ?2)",
            params![version, now],
        )?;
        Ok(())
    }
}

// ── managed_users + cached config ─────────────────────────────────────────────

impl Db {
    /// Replace all managed users and their cached config atomically.
    pub fn apply_config_push(
        &self,
        users: &[common::models::UserConfig],
    ) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;

        tx.execute("DELETE FROM managed_users", [])?;

        for u in users {
            let status = match u.status {
                common::models::UserStatus::Managed => "managed",
                common::models::UserStatus::Unmanaged => "unmanaged",
            };
            tx.execute(
                "INSERT INTO managed_users (local_uid, local_username, profile_id, status)
                 VALUES (?1, ?2, ?3, ?4)",
                params![u.local_uid, "", u.profile_id.to_string(), status],
            )?;

            for s in &u.schedules {
                tx.execute(
                    "INSERT INTO cached_schedules (local_uid, day_of_week, start_time, end_time)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![
                        u.local_uid,
                        s.day_of_week,
                        s.start_time.format("%H:%M").to_string(),
                        s.end_time.format("%H:%M").to_string(),
                    ],
                )?;
            }

            for l in &u.daily_limits {
                tx.execute(
                    "INSERT INTO cached_daily_limits (local_uid, day_of_week, allowed_minutes)
                     VALUES (?1, ?2, ?3)",
                    params![u.local_uid, l.day_of_week, l.allowed_minutes],
                )?;
            }

            let today = chrono::Local::now().date_naive().to_string();
            if u.adjustments_today != 0 {
                tx.execute(
                    "INSERT OR REPLACE INTO cached_adjustments (local_uid, target_date, adjustment_minutes)
                     VALUES (?1, ?2, ?3)",
                    params![u.local_uid, today, u.adjustments_today],
                )?;
            }

            tx.execute(
                "INSERT OR REPLACE INTO cached_enforcement (local_uid, lockout_grace_minutes, warning_thresholds)
                 VALUES (?1, ?2, ?3)",
                params![
                    u.local_uid,
                    u.lockout_grace_minutes,
                    u.warning_thresholds_minutes
                        .iter()
                        .map(|v| v.to_string())
                        .collect::<Vec<_>>()
                        .join(","),
                ],
            )?;
        }

        tx.commit()?;
        Ok(())
    }

    pub fn get_managed_uids(&self) -> Result<Vec<u32>> {
        let mut stmt = self.conn.prepare(
            "SELECT local_uid FROM managed_users WHERE status = 'managed'"
        )?;
        let uids = stmt
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<u32>>>()?;
        Ok(uids)
    }
}

// ── usage_log ─────────────────────────────────────────────────────────────────

impl Db {
    pub fn add_usage_seconds(&self, uid: u32, date: &str, seconds: u64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO usage_log (local_uid, date, used_seconds, synced)
             VALUES (?1, ?2, ?3, 0)
             ON CONFLICT(local_uid, date) DO UPDATE SET used_seconds = used_seconds + ?3",
            params![uid, date, seconds],
        )?;
        Ok(())
    }

    pub fn get_usage_seconds(&self, uid: u32, date: &str) -> Result<u64> {
        let secs: u64 = self.conn.query_row(
            "SELECT used_seconds FROM usage_log WHERE local_uid = ?1 AND date = ?2",
            params![uid, date],
            |row| row.get(0),
        ).unwrap_or(0);
        Ok(secs)
    }

    pub fn get_unsynced_usage(&self) -> Result<Vec<(u32, String, u64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT local_uid, date, used_seconds FROM usage_log WHERE synced = 0"
        )?;
        let rows = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .collect::<rusqlite::Result<Vec<(u32, String, u64)>>>()?;
        Ok(rows)
    }

    pub fn mark_usage_synced(&self, uid: u32, date: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE usage_log SET synced = 1 WHERE local_uid = ?1 AND date = ?2",
            params![uid, date],
        )?;
        Ok(())
    }

    pub fn reset_usage_for_date(&self, uid: u32, date: &NaiveDate) -> Result<()> {
        self.conn.execute(
            "UPDATE usage_log SET used_seconds = 0, synced = 0
             WHERE local_uid = ?1 AND date = ?2",
            params![uid, date.to_string()],
        )?;
        Ok(())
    }
}

// ── server_remaining ──────────────────────────────────────────────────────────

#[allow(dead_code)]
pub struct ServerRemaining {
    pub remaining_minutes: i32,
    pub enforce: String,
    pub updated_at: i64,
}

impl Db {
    pub fn upsert_server_remaining(&self, uid: u32, remaining_minutes: i32, enforce: &str) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        self.conn.execute(
            "INSERT OR REPLACE INTO server_remaining (local_uid, remaining_minutes, enforce, updated_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![uid, remaining_minutes, enforce, now],
        )?;
        Ok(())
    }

    pub fn get_server_remaining(&self, uid: u32) -> Result<Option<ServerRemaining>> {
        let mut stmt = self.conn.prepare(
            "SELECT remaining_minutes, enforce, updated_at FROM server_remaining WHERE local_uid = ?1"
        )?;
        let mut rows = stmt.query(params![uid])?;
        if let Some(row) = rows.next()? {
            Ok(Some(ServerRemaining {
                remaining_minutes: row.get(0)?,
                enforce: row.get(1)?,
                updated_at: row.get(2)?,
            }))
        } else {
            Ok(None)
        }
    }
}

// ── active_sessions ───────────────────────────────────────────────────────────

impl Db {
    pub fn upsert_session(&self, uid: u32, session_id: &str, is_idle: bool) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        self.conn.execute(
            "INSERT INTO active_sessions (local_uid, session_id, started_at, last_heartbeat, is_idle)
             VALUES (?1, ?2, ?3, ?3, ?4)
             ON CONFLICT(local_uid, session_id) DO UPDATE
             SET last_heartbeat = ?3, is_idle = ?4",
            params![uid, session_id, now, is_idle as i32],
        )?;
        Ok(())
    }

    pub fn remove_session(&self, uid: u32, session_id: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM active_sessions WHERE local_uid = ?1 AND session_id = ?2",
            params![uid, session_id],
        )?;
        Ok(())
    }

    /// Returns session_ids for non-idle sessions of a user.
    #[allow(dead_code)]
    pub fn get_active_session_ids(&self, uid: u32) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT session_id FROM active_sessions WHERE local_uid = ?1 AND is_idle = 0"
        )?;
        let ids = stmt
            .query_map(params![uid], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(ids)
    }

    /// Returns all session_ids for a user (idle or not), used for locking.
    pub fn get_all_session_ids(&self, uid: u32) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT session_id FROM active_sessions WHERE local_uid = ?1"
        )?;
        let ids = stmt
            .query_map(params![uid], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(ids)
    }

    pub fn count_active_sessions(&self, uid: u32) -> Result<u32> {
        let count: u32 = self.conn.query_row(
            "SELECT COUNT(*) FROM active_sessions WHERE local_uid = ?1 AND is_idle = 0",
            params![uid],
            |row| row.get(0),
        )?;
        Ok(count)
    }
}

// ── agent_state ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentMode {
    Unpaired,
    Online,
    Offline,
}

impl Db {
    pub fn get_agent_mode(&self) -> Result<AgentMode> {
        let mode: String = self.conn.query_row(
            "SELECT mode FROM agent_state WHERE id = 1",
            [],
            |row| row.get(0),
        )?;
        Ok(match mode.as_str() {
            "online" => AgentMode::Online,
            "offline" => AgentMode::Offline,
            _ => AgentMode::Unpaired,
        })
    }

    pub fn set_agent_mode(&self, mode: AgentMode) -> Result<()> {
        let (mode_str, offline_since) = match mode {
            AgentMode::Unpaired => ("unpaired", None),
            AgentMode::Online => ("online", None),
            AgentMode::Offline => ("offline", Some(chrono::Utc::now().timestamp())),
        };
        self.conn.execute(
            "UPDATE agent_state SET mode = ?1, offline_since = ?2 WHERE id = 1",
            params![mode_str, offline_since],
        )?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn get_offline_since(&self) -> Result<Option<i64>> {
        let val: Option<i64> = self.conn.query_row(
            "SELECT offline_since FROM agent_state WHERE id = 1",
            [],
            |row| row.get(0),
        )?;
        Ok(val)
    }
}

// ── cached schedules/limits for offline enforcement ───────────────────────────

pub struct CachedSchedule {
    pub day_of_week: u8,
    pub start_time: String,
    pub end_time: String,
}

pub struct CachedLimit {
    pub day_of_week: u8,
    pub allowed_minutes: u32,
}

pub struct CachedEnforcement {
    pub lockout_grace_minutes: u32,
    pub warning_thresholds: Vec<u32>,
}

impl Db {
    pub fn get_cached_schedules(&self, uid: u32) -> Result<Vec<CachedSchedule>> {
        let mut stmt = self.conn.prepare(
            "SELECT day_of_week, start_time, end_time FROM cached_schedules WHERE local_uid = ?1"
        )?;
        let rows = stmt
            .query_map(params![uid], |row| {
                Ok(CachedSchedule {
                    day_of_week: row.get::<_, u8>(0)?,
                    start_time: row.get(1)?,
                    end_time: row.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn get_cached_daily_limits(&self, uid: u32) -> Result<Vec<CachedLimit>> {
        let mut stmt = self.conn.prepare(
            "SELECT day_of_week, allowed_minutes FROM cached_daily_limits WHERE local_uid = ?1"
        )?;
        let rows = stmt
            .query_map(params![uid], |row| {
                Ok(CachedLimit {
                    day_of_week: row.get::<_, u8>(0)?,
                    allowed_minutes: row.get(1)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn get_cached_adjustment(&self, uid: u32, date: &str) -> Result<i32> {
        let val: i32 = self.conn.query_row(
            "SELECT adjustment_minutes FROM cached_adjustments WHERE local_uid = ?1 AND target_date = ?2",
            params![uid, date],
            |row| row.get(0),
        ).unwrap_or(0);
        Ok(val)
    }

    pub fn get_cached_enforcement(&self, uid: u32) -> Result<CachedEnforcement> {
        let (grace, thresholds_str): (u32, String) = self.conn.query_row(
            "SELECT lockout_grace_minutes, warning_thresholds FROM cached_enforcement WHERE local_uid = ?1",
            params![uid],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap_or((5, "15,5,1".to_string()));

        let thresholds = thresholds_str
            .split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect();

        Ok(CachedEnforcement {
            lockout_grace_minutes: grace,
            warning_thresholds: thresholds,
        })
    }
}
