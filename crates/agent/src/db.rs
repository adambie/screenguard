use anyhow::{Context, Result};
use chrono::NaiveDate;
use rusqlite::{Connection, OptionalExtension, params};
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
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&self) -> Result<()> {
        // Ignore errors — columns/tables already exist on fresh DBs created by create_schema.
        let _ = self.conn.execute_batch(
            "ALTER TABLE cached_enforcement ADD COLUMN language TEXT NOT NULL DEFAULT 'en'",
        );
        let _ = self.conn.execute_batch(
            "ALTER TABLE cached_enforcement ADD COLUMN preserve_tasks_on_lock INTEGER NOT NULL DEFAULT 0",
        );
        // Experimental cloud mode: records which cloud account (email) this
        // pairing is bound to. NULL for self-hosted / local-network pairings.
        let _ = self.conn.execute_batch(
            "ALTER TABLE server_connection ADD COLUMN cloud_account TEXT",
        );
        let _ = self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS agent_capabilities (
                capability  TEXT PRIMARY KEY,
                available   INTEGER NOT NULL DEFAULT 0,
                checked_at  INTEGER NOT NULL
            )",
        );
        // Per-app usage (#27). Keyed by date, so a new day starts new rows.
        let _ = self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS app_usage_log (
                local_uid       INTEGER NOT NULL,
                date            TEXT NOT NULL,
                app_id          TEXT NOT NULL,
                display_name    TEXT NOT NULL,
                icon            TEXT,
                categories      TEXT NOT NULL DEFAULT '',
                seconds         INTEGER NOT NULL DEFAULT 0,
                synced          INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (local_uid, date, app_id)
            )",
        );
        // Format of the cached config (see CONFIG_FORMAT); 0 on older DBs.
        let _ = self.conn.execute_batch(
            "ALTER TABLE config_meta ADD COLUMN config_format INTEGER NOT NULL DEFAULT 0",
        );
        // Time per app group (#27 Phase 2): while any member app ran.
        let _ = self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS group_usage_log (
                local_uid INTEGER NOT NULL,
                date      TEXT NOT NULL,
                group_id  TEXT NOT NULL,
                seconds   INTEGER NOT NULL DEFAULT 0,
                synced    INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (local_uid, date, group_id)
            )",
        );
        // Group time on the child's other computers, last reported by the server.
        let _ = self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS cached_group_elsewhere (
                local_uid INTEGER NOT NULL,
                date      TEXT NOT NULL,
                group_id  TEXT NOT NULL,
                seconds   INTEGER NOT NULL,
                PRIMARY KEY (local_uid, group_id)
            )",
        );
        // App groups from the server's config (#27 Phase 2), as JSON per user.
        let _ = self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS cached_app_groups (
                local_uid INTEGER PRIMARY KEY,
                groups    TEXT NOT NULL
            )",
        );
        // App categories (#27 Phase 2) — on DBs created before the column existed.
        let _ = self.conn.execute_batch(
            "ALTER TABLE app_usage_log ADD COLUMN categories TEXT NOT NULL DEFAULT ''",
        );
        let _ = self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS cached_blocked_domains (
                local_uid   INTEGER NOT NULL REFERENCES managed_users(local_uid) ON DELETE CASCADE,
                domain      TEXT NOT NULL,
                PRIMARY KEY (local_uid, domain)
            )",
        );
        Ok(())
    }

    fn create_schema(&self) -> Result<()> {
        self.conn.execute_batch(r#"
            CREATE TABLE IF NOT EXISTS server_connection (
                id              INTEGER PRIMARY KEY CHECK (id = 1),
                server_url      TEXT NOT NULL,
                auth_token      TEXT NOT NULL,
                agent_id        TEXT NOT NULL,
                paired_at       INTEGER NOT NULL,
                last_sync_at    INTEGER,
                cloud_account   TEXT
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
                preserve_tasks_on_lock  INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (local_uid)
            );

            CREATE TABLE IF NOT EXISTS usage_log (
                local_uid       INTEGER NOT NULL,
                date            TEXT NOT NULL,
                used_seconds    INTEGER NOT NULL DEFAULT 0,
                synced          INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (local_uid, date)
            );

            CREATE TABLE IF NOT EXISTS app_usage_log (
                local_uid       INTEGER NOT NULL,
                date            TEXT NOT NULL,
                app_id          TEXT NOT NULL,
                display_name    TEXT NOT NULL,
                icon            TEXT,
                categories      TEXT NOT NULL DEFAULT '',
                seconds         INTEGER NOT NULL DEFAULT 0,
                synced          INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (local_uid, date, app_id)
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

            CREATE TABLE IF NOT EXISTS agent_capabilities (
                capability  TEXT PRIMARY KEY,
                available   INTEGER NOT NULL DEFAULT 0,
                checked_at  INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS cached_blocked_domains (
                local_uid   INTEGER NOT NULL REFERENCES managed_users(local_uid) ON DELETE CASCADE,
                domain      TEXT NOT NULL,
                PRIMARY KEY (local_uid, domain)
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

    /// Wipe everything tied to the current pairing *and* the downloaded policy,
    /// used when the agent is re-bound to a different cloud account (experimental
    /// cloud mode). Unlike [`reset_pairing`], this also drops cached rules and
    /// pending usage, because that data belongs to the old account/tenant and
    /// must not be enforced against — or uploaded into — the new one.
    ///
    /// Hardware capability flags (`agent_capabilities`) are kept: they describe
    /// the machine, not the tenant.
    pub fn wipe_for_rebind(&self) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        // managed_users cascades to cached_schedules / cached_daily_limits /
        // cached_adjustments / cached_enforcement / cached_blocked_domains,
        // but delete explicitly so this does not depend on PRAGMA foreign_keys.
        for table in [
            "cached_blocked_domains",
            "cached_adjustments",
            "cached_daily_limits",
            "cached_schedules",
            "cached_enforcement",
            "managed_users",
            "usage_log",
            "app_usage_log",
            "group_usage_log",
            "cached_app_groups",
            "cached_group_elsewhere",
            "server_remaining",
            "active_sessions",
            "config_meta",
            "server_connection",
        ] {
            tx.execute(&format!("DELETE FROM {table}"), [])?;
        }
        tx.execute(
            "UPDATE agent_state SET mode = 'unpaired', offline_since = NULL WHERE id = 1",
            [],
        )?;
        tx.commit()?;
        Ok(())
    }
}

// ── server_connection ─────────────────────────────────────────────────────────

pub struct ServerConnection {
    pub server_url: String,
    pub auth_token: String,
    pub agent_id: String,
    /// Cloud account (email) this pairing is bound to; `None` for local pairings.
    pub cloud_account: Option<String>,
}

impl Db {
    pub fn get_server_connection(&self) -> Result<Option<ServerConnection>> {
        let mut stmt = self.conn.prepare(
            "SELECT server_url, auth_token, agent_id, cloud_account FROM server_connection WHERE id = 1"
        )?;
        let mut rows = stmt.query([])?;
        if let Some(row) = rows.next()? {
            Ok(Some(ServerConnection {
                server_url: row.get(0)?,
                auth_token: row.get(1)?,
                agent_id: row.get(2)?,
                cloud_account: row.get(3)?,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn save_server_connection(&self, sc: &ServerConnection) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        self.conn.execute(
            "INSERT OR REPLACE INTO server_connection
             (id, server_url, auth_token, agent_id, paired_at, cloud_account)
             VALUES (1, ?1, ?2, ?3, ?4, ?5)",
            params![sc.server_url, sc.auth_token, sc.agent_id, now, sc.cloud_account],
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

/// Bump when the agent starts storing a new part of `UserConfig` (2 = app
/// groups). Config stored in an older format is re-requested on the next hello —
/// otherwise an upgraded agent keeps the old cached config, without the new
/// parts, until some setting changes on the server.
pub const CONFIG_FORMAT: i64 = 2;

impl Db {
    /// The config version to report in `agent_hello`: 0 when the cached config
    /// was stored in an older format, so the server sends it again in full.
    pub fn get_config_version(&self) -> Result<i64> {
        let (version, format): (i64, i64) = self
            .conn
            .query_row("SELECT config_version, config_format FROM config_meta WHERE id = 1", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap_or((0, 0));
        Ok(if format < CONFIG_FORMAT { 0 } else { version })
    }

    pub fn save_config_version(&self, version: i64) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        self.conn.execute(
            "INSERT OR REPLACE INTO config_meta (id, config_version, received_at, config_format) VALUES (1, ?1, ?2, ?3)",
            params![version, now, CONFIG_FORMAT],
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
        tx.execute("DELETE FROM cached_app_groups", [])?;

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
                "INSERT OR REPLACE INTO cached_enforcement
                 (local_uid, lockout_grace_minutes, warning_thresholds, language, preserve_tasks_on_lock)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    u.local_uid,
                    u.lockout_grace_minutes,
                    u.warning_thresholds_minutes
                        .iter()
                        .map(|v| v.to_string())
                        .collect::<Vec<_>>()
                        .join(","),
                    &u.language,
                    u.preserve_tasks_on_lock,
                ],
            )?;

            tx.execute(
                "INSERT OR REPLACE INTO cached_app_groups (local_uid, groups) VALUES (?1, ?2)",
                params![u.local_uid, serde_json::to_string(&u.app_groups)?],
            )?;

            tx.execute(
                "DELETE FROM cached_blocked_domains WHERE local_uid = ?1",
                params![u.local_uid],
            )?;
            for domain in &u.blocked_domains {
                tx.execute(
                    "INSERT OR IGNORE INTO cached_blocked_domains (local_uid, domain) VALUES (?1, ?2)",
                    params![u.local_uid, domain],
                )?;
            }
        }

        tx.commit()?;
        Ok(())
    }

    /// App groups of a managed user from the last config push (empty if none).
    pub fn get_cached_app_groups(&self, uid: u32) -> Result<Vec<common::models::AppGroupConfig>> {
        let json: Option<String> = self
            .conn
            .query_row("SELECT groups FROM cached_app_groups WHERE local_uid = ?1", params![uid], |r| r.get(0))
            .optional()?;
        Ok(json.map(|j| serde_json::from_str(&j)).transpose()?.unwrap_or_default())
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

// ── app_usage_log ─────────────────────────────────────────────────────────────

/// One row of `app_usage_log`: an app's running total for a day.
#[derive(Debug, Clone, PartialEq)]
pub struct AppUsageRow {
    pub local_uid: u32,
    pub date: String,
    pub app_id: String,
    pub display_name: String,
    pub icon: Option<String>,
    /// `;`-separated freedesktop categories.
    pub categories: String,
    pub seconds: u64,
}

impl Db {
    /// Adds seconds to the day's total and flags the row for the next sync; the
    /// name/icon are refreshed (language or `.desktop` may have changed).
    #[allow(clippy::too_many_arguments)]
    pub fn add_app_usage(&self, uid: u32, date: &str, app_id: &str, name: &str, icon: Option<&str>, categories: &str, seconds: u64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO app_usage_log (local_uid, date, app_id, display_name, icon, categories, seconds, synced)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0)
             ON CONFLICT(local_uid, date, app_id) DO UPDATE SET
                seconds = seconds + ?7, display_name = ?4, icon = ?5, categories = ?6, synced = 0",
            params![uid, date, app_id, name, icon, categories, seconds],
        )?;
        Ok(())
    }

    /// Unsynced rows, plus every row from `resend_since` on (YYYY-MM-DD) when
    /// given — used after a reconnect, as rows sent just before a disconnect may
    /// never have arrived. The server keeps the larger total, so resending is safe.
    pub fn get_unsynced_app_usage(&self, resend_since: Option<&str>) -> Result<Vec<AppUsageRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT local_uid, date, app_id, display_name, icon, categories, seconds FROM app_usage_log
             WHERE synced = 0 OR date >= ?1 ORDER BY date, local_uid, app_id",
        )?;
        // '~' sorts after any date: no resend
        let rows = stmt
            .query_map(params![resend_since.unwrap_or("~")], |r| {
                Ok(AppUsageRow {
                    local_uid: r.get(0)?,
                    date: r.get(1)?,
                    app_id: r.get(2)?,
                    display_name: r.get(3)?,
                    icon: r.get(4)?,
                    categories: r.get(5)?,
                    seconds: r.get(6)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Marks a row synced only if it still holds the total that was sent, so
    /// seconds added in between are picked up by the next sync.
    pub fn mark_app_usage_synced(&self, row: &AppUsageRow) -> Result<()> {
        self.conn.execute(
            "UPDATE app_usage_log SET synced = 1
             WHERE local_uid = ?1 AND date = ?2 AND app_id = ?3 AND seconds = ?4",
            params![row.local_uid, row.date, row.app_id, row.seconds],
        )?;
        Ok(())
    }

    #[cfg(test)]
    pub fn get_app_usage_seconds(&self, uid: u32, date: &str, app_id: &str) -> Result<u64> {
        Ok(self
            .conn
            .query_row(
                "SELECT seconds FROM app_usage_log WHERE local_uid = ?1 AND date = ?2 AND app_id = ?3",
                params![uid, date, app_id],
                |r| r.get(0),
            )
            .unwrap_or(0))
    }

    /// Drops days before `before` (YYYY-MM-DD compares as text).
    pub fn prune_app_usage(&self, before: &str) -> Result<usize> {
        Ok(self.conn.execute("DELETE FROM app_usage_log WHERE date < ?1", params![before])?)
    }
}

// ── group_usage_log ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct GroupUsageRow {
    pub local_uid: u32,
    pub date: String,
    pub group_id: String,
    pub seconds: u64,
}

impl Db {
    pub fn add_group_usage(&self, uid: u32, date: &str, group_id: &str, seconds: u64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO group_usage_log (local_uid, date, group_id, seconds, synced) VALUES (?1, ?2, ?3, ?4, 0)
             ON CONFLICT(local_uid, date, group_id) DO UPDATE SET seconds = seconds + ?4, synced = 0",
            params![uid, date, group_id, seconds],
        )?;
        Ok(())
    }

    /// Like `get_unsynced_app_usage`: unsynced rows plus everything since `resend_since`.
    pub fn get_unsynced_group_usage(&self, resend_since: Option<&str>) -> Result<Vec<GroupUsageRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT local_uid, date, group_id, seconds FROM group_usage_log
             WHERE synced = 0 OR date >= ?1 ORDER BY date, local_uid, group_id",
        )?;
        let rows = stmt
            .query_map(params![resend_since.unwrap_or("~")], |r| {
                Ok(GroupUsageRow { local_uid: r.get(0)?, date: r.get(1)?, group_id: r.get(2)?, seconds: r.get(3)? })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn mark_group_usage_synced(&self, row: &GroupUsageRow) -> Result<()> {
        self.conn.execute(
            "UPDATE group_usage_log SET synced = 1
             WHERE local_uid = ?1 AND date = ?2 AND group_id = ?3 AND seconds = ?4",
            params![row.local_uid, row.date, row.group_id, row.seconds],
        )?;
        Ok(())
    }

    /// This machine's time per group for a user on a date.
    pub fn get_group_usage(&self, uid: u32, date: &str) -> Result<std::collections::HashMap<String, u64>> {
        let mut stmt = self.conn.prepare("SELECT group_id, seconds FROM group_usage_log WHERE local_uid = ?1 AND date = ?2")?;
        let rows = stmt
            .query_map(params![uid, date], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<std::collections::HashMap<String, u64>>>()?;
        Ok(rows)
    }

    /// Replaces a user's "time on other computers" with the server's latest numbers.
    pub fn save_group_elsewhere(&self, uid: u32, date: &str, usage: &[common::models::GroupUsage]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM cached_group_elsewhere WHERE local_uid = ?1", params![uid])?;
        for u in usage {
            tx.execute(
                "INSERT INTO cached_group_elsewhere (local_uid, date, group_id, seconds) VALUES (?1, ?2, ?3, ?4)",
                params![uid, date, u.group_id, u.used_seconds],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Last known time per group on other computers, for `date` only (a new day starts at 0).
    pub fn get_group_elsewhere(&self, uid: u32, date: &str) -> Result<std::collections::HashMap<String, u64>> {
        let mut stmt = self.conn.prepare("SELECT group_id, seconds FROM cached_group_elsewhere WHERE local_uid = ?1 AND date = ?2")?;
        let rows = stmt
            .query_map(params![uid, date], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<std::collections::HashMap<String, u64>>>()?;
        Ok(rows)
    }

    pub fn prune_group_usage(&self, before: &str) -> Result<usize> {
        Ok(self.conn.execute("DELETE FROM group_usage_log WHERE date < ?1", params![before])?)
    }
}

// ── usage_log ─────────────────────────────────────────────────────────────────

impl Db {
    pub fn add_usage_seconds(&self, uid: u32, date: &str, seconds: u64) -> Result<()> {
        // Reset synced=0 on every update, not just the initial insert: otherwise
        // a day's row is marked synced=1 after the first reconnect of the day,
        // and any usage added by a *second* disconnect/reconnect that same day
        // is never picked up by get_unsynced_usage() — the server's number gets
        // stuck until local midnight rolls the row over.
        self.conn.execute(
            "INSERT INTO usage_log (local_uid, date, used_seconds, synced)
             VALUES (?1, ?2, ?3, 0)
             ON CONFLICT(local_uid, date) DO UPDATE SET used_seconds = used_seconds + ?3, synced = 0",
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
        let tx = self.conn.unchecked_transaction()?;
        // Remove any stale row where the same session_id was owned by a different uid
        // (can happen if logind reuses the ID after a reboot and the agent missed cleanup).
        tx.execute(
            "DELETE FROM active_sessions WHERE session_id = ?1 AND local_uid != ?2",
            params![session_id, uid],
        )?;
        tx.execute(
            "INSERT INTO active_sessions (local_uid, session_id, started_at, last_heartbeat, is_idle)
             VALUES (?1, ?2, ?3, ?3, ?4)
             ON CONFLICT(local_uid, session_id) DO UPDATE
             SET last_heartbeat = ?3, is_idle = ?4",
            params![uid, session_id, now, is_idle as i32],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Remove any `active_sessions` rows that are not in the live logind session snapshot.
    /// Called once at agent startup to purge rows left over from before a reboot.
    pub fn reconcile_sessions(&self, live: &[(u32, String)]) -> Result<()> {
        let live: std::collections::HashSet<(u32, &str)> =
            live.iter().map(|(uid, sid)| (*uid, sid.as_str())).collect();
        let mut stmt = self.conn.prepare(
            "SELECT local_uid, session_id FROM active_sessions",
        )?;
        let stale: Vec<(u32, String)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        for (uid, sid) in stale {
            if !live.contains(&(uid, sid.as_str())) {
                self.conn.execute(
                    "DELETE FROM active_sessions WHERE local_uid = ?1 AND session_id = ?2",
                    params![uid, sid],
                )?;
            }
        }
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
    pub language: String,
    pub preserve_tasks_on_lock: bool,
}

impl Default for CachedEnforcement {
    fn default() -> Self {
        Self {
            lockout_grace_minutes: 5,
            warning_thresholds: vec![15, 5, 1],
            language: "en".to_string(),
            preserve_tasks_on_lock: false,
        }
    }
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
        let (grace, thresholds_str, language, preserve_tasks): (u32, String, String, bool) = self.conn.query_row(
            "SELECT lockout_grace_minutes, warning_thresholds, language, preserve_tasks_on_lock FROM cached_enforcement WHERE local_uid = ?1",
            params![uid],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        ).unwrap_or((5, "15,5,1".to_string(), "en".to_string(), false));

        let thresholds = thresholds_str
            .split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect();

        Ok(CachedEnforcement {
            lockout_grace_minutes: grace,
            warning_thresholds: thresholds,
            language,
            preserve_tasks_on_lock: preserve_tasks,
        })
    }

    pub fn save_capability(&self, capability: &str, available: bool) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        self.conn.execute(
            "INSERT OR REPLACE INTO agent_capabilities (capability, available, checked_at)
             VALUES (?1, ?2, ?3)",
            params![capability, available as i32, now],
        )?;
        Ok(())
    }

    pub fn get_capability(&self, capability: &str) -> Result<Option<bool>> {
        let val: Option<i32> = self.conn.query_row(
            "SELECT available FROM agent_capabilities WHERE capability = ?1",
            params![capability],
            |row| row.get(0),
        ).optional()?;
        Ok(val.map(|v| v != 0))
    }

    pub fn save_cached_blocked_domains(&self, local_uid: u32, domains: &[String]) -> Result<()> {
        self.conn.execute(
            "DELETE FROM cached_blocked_domains WHERE local_uid = ?1",
            params![local_uid],
        )?;
        for domain in domains {
            self.conn.execute(
                "INSERT OR IGNORE INTO cached_blocked_domains (local_uid, domain) VALUES (?1, ?2)",
                params![local_uid, domain],
            )?;
        }
        Ok(())
    }

    pub fn get_cached_blocked_domains(&self, local_uid: u32) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT domain FROM cached_blocked_domains WHERE local_uid = ?1 ORDER BY domain",
        )?;
        let rows = stmt.query_map(params![local_uid], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::{AgentMode, Db, ServerConnection};
    use common::models::{UserConfig, UserStatus};
    use rusqlite::Connection;
    use uuid::Uuid;

    fn conn(url: &str, cloud_account: Option<&str>) -> ServerConnection {
        ServerConnection {
            server_url: url.to_string(),
            auth_token: "tok".to_string(),
            agent_id: "aid".to_string(),
            cloud_account: cloud_account.map(str::to_string),
        }
    }

    fn user_config(uid: u32, preserve_tasks_on_lock: bool) -> UserConfig {
        UserConfig {
            local_uid: uid,
            profile_id: Uuid::new_v4(),
            status: UserStatus::Managed,
            schedules: Vec::new(),
            daily_limits: Vec::new(),
            adjustments_today: 0,
            adjustment_message: None,
            lockout_grace_minutes: 5,
            preserve_tasks_on_lock,
            warning_thresholds_minutes: vec![15, 5, 1],
            language: "en".to_string(),
            blocked_domains: Vec::new(),
            app_groups: Vec::new(),
        }
    }

    #[test]
    fn cached_enforcement_defaults_preserve_tasks_to_false() {
        let db = Db::open(Some(":memory:")).unwrap();

        assert!(!db.get_cached_enforcement(1000).unwrap().preserve_tasks_on_lock);
    }

    #[test]
    fn migration_defaults_existing_cached_enforcement_to_false() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE cached_enforcement (
                local_uid INTEGER PRIMARY KEY,
                lockout_grace_minutes INTEGER NOT NULL DEFAULT 5,
                warning_thresholds TEXT NOT NULL DEFAULT '15,5,1'
             );
             INSERT INTO cached_enforcement (local_uid) VALUES (1000);",
        ).unwrap();
        let db = Db { conn };

        db.migrate().unwrap();

        assert!(!db.get_cached_enforcement(1000).unwrap().preserve_tasks_on_lock);
    }

    // Regression test for #17: after the first reconnect of the day marks a
    // date's usage_log row synced=1, a *second* round of usage added that same
    // day (e.g. the agent drops offline again, keeps counting, reconnects a
    // second time) must be flagged unsynced again so send_usage_sync picks it
    // up — otherwise the server's displayed remaining-time freezes for the
    // rest of that day.
    #[test]
    fn add_usage_seconds_reflags_unsynced_after_a_sync_cycle() {
        let db = Db::open(Some(":memory:")).unwrap();
        let today = "2026-10-05";

        db.add_usage_seconds(1000, today, 60).unwrap();
        assert_eq!(db.get_unsynced_usage().unwrap(), vec![(1000, today.to_string(), 60)]);

        // First reconnect of the day syncs it.
        db.mark_usage_synced(1000, today).unwrap();
        assert!(db.get_unsynced_usage().unwrap().is_empty());

        // More usage accrues after that — must be unsynced again, not silently
        // folded into the already-synced row.
        db.add_usage_seconds(1000, today, 30).unwrap();
        assert_eq!(db.get_unsynced_usage().unwrap(), vec![(1000, today.to_string(), 90)]);
    }

    #[test]
    fn app_usage_accumulates_syncs_and_prunes() {
        let db = Db::open(Some(":memory:")).unwrap();
        db.add_app_usage(1000, "2026-10-20", "desktop:firefox", "Firefox", Some("firefox"), "", 5).unwrap();
        db.add_app_usage(1000, "2026-10-20", "desktop:firefox", "Firefox", Some("firefox"), "", 5).unwrap();
        db.add_app_usage(1000, "2026-09-01", "steam:570", "Dota 2", None, "", 60).unwrap();
        assert_eq!(db.get_app_usage_seconds(1000, "2026-10-20", "desktop:firefox").unwrap(), 10);

        let unsynced = db.get_unsynced_app_usage(None).unwrap();
        assert_eq!(unsynced.len(), 2);
        let ff = unsynced.iter().find(|r| r.app_id == "desktop:firefox").unwrap().clone();
        assert_eq!((ff.seconds, ff.icon.as_deref()), (10, Some("firefox")));

        // seconds added between reading and marking keep the row unsynced
        db.add_app_usage(1000, "2026-10-20", "desktop:firefox", "Firefox", Some("firefox"), "", 5).unwrap();
        db.mark_app_usage_synced(&ff).unwrap();
        assert_eq!(db.get_unsynced_app_usage(None).unwrap().iter().find(|r| r.app_id == "desktop:firefox").unwrap().seconds, 15);
        let ff = db.get_unsynced_app_usage(None).unwrap().into_iter().find(|r| r.app_id == "desktop:firefox").unwrap();
        db.mark_app_usage_synced(&ff).unwrap();
        assert!(db.get_unsynced_app_usage(None).unwrap().iter().all(|r| r.app_id != "desktop:firefox"));

        // firefox is synced but recent; steam is old but still unsynced
        assert_eq!(db.get_unsynced_app_usage(Some("2026-10-19")).unwrap().len(), 2);
        assert_eq!(db.prune_app_usage("2026-09-15").unwrap(), 1);
        assert_eq!(db.get_app_usage_seconds(1000, "2026-09-01", "steam:570").unwrap(), 0);
    }

    #[test]
    fn group_time_elsewhere_is_cached_per_day() {
        use common::models::GroupUsage;
        let db = Db::open(Some(":memory:")).unwrap();
        db.save_group_elsewhere(1000, "2026-10-20", &[GroupUsage { group_id: "g".into(), used_seconds: 600 }]).unwrap();
        assert_eq!(db.get_group_elsewhere(1000, "2026-10-20").unwrap()["g"], 600);
        assert!(db.get_group_elsewhere(1000, "2026-10-21").unwrap().is_empty()); // a new day
        db.save_group_elsewhere(1000, "2026-10-20", &[]).unwrap(); // replaced
        assert!(db.get_group_elsewhere(1000, "2026-10-20").unwrap().is_empty());
    }

    #[test]
    fn config_from_an_older_agent_is_requested_again() {
        let db = Db::open(Some(":memory:")).unwrap();
        db.save_config_version(7).unwrap();
        assert_eq!(db.get_config_version().unwrap(), 7);
        // what an older agent left behind: version stored, no format
        db.conn.execute("UPDATE config_meta SET config_format = 0", []).unwrap();
        assert_eq!(db.get_config_version().unwrap(), 0);
        db.save_config_version(7).unwrap(); // the full push arrives
        assert_eq!(db.get_config_version().unwrap(), 7);
    }

    #[test]
    fn group_usage_accumulates_syncs_and_prunes() {
        let db = Db::open(Some(":memory:")).unwrap();
        db.add_group_usage(1000, "2026-10-20", "g1", 5).unwrap();
        db.add_group_usage(1000, "2026-10-20", "g1", 5).unwrap();
        db.add_group_usage(1000, "2026-09-01", "g2", 60).unwrap();
        assert_eq!(db.get_group_usage(1000, "2026-10-20").unwrap()["g1"], 10);
        let rows = db.get_unsynced_group_usage(None).unwrap();
        assert_eq!(rows.len(), 2);
        for r in &rows {
            db.mark_group_usage_synced(r).unwrap();
        }
        assert!(db.get_unsynced_group_usage(None).unwrap().is_empty());
        assert_eq!(db.get_unsynced_group_usage(Some("2026-10-19")).unwrap().len(), 1);
        assert_eq!(db.prune_group_usage("2026-09-15").unwrap(), 1);
    }

    #[test]
    fn app_groups_are_cached_from_config_push() {
        use common::apps::{GroupRule, RuleKind};
        use common::models::{AppGroupConfig, DailyLimit};
        let db = Db::open(Some(":memory:")).unwrap();
        let mut cfg = user_config(1000, false);
        cfg.app_groups = vec![AppGroupConfig {
            id: "g1".into(),
            name: "Games".into(),
            builtin: Some("games".into()),
            rules: vec![GroupRule { kind: RuleKind::Category, value: "Game".into(), exclude: false }],
            daily_limits: vec![DailyLimit { day_of_week: 0, allowed_minutes: 60 }],
        }];
        db.apply_config_push(&[cfg]).unwrap();
        let g = db.get_cached_app_groups(1000).unwrap();
        assert_eq!((g.len(), g[0].id.as_str(), g[0].daily_limits[0].allowed_minutes), (1, "g1", 60));
        assert!(db.get_cached_app_groups(1001).unwrap().is_empty());
        // a later push without the user drops its groups
        db.apply_config_push(&[]).unwrap();
        assert!(db.get_cached_app_groups(1000).unwrap().is_empty());
    }

    #[test]
    fn app_usage_table_is_added_to_old_dbs() {
        let conn = Connection::open_in_memory().unwrap();
        let db = Db { conn };
        db.migrate().unwrap();
        db.add_app_usage(1000, "2026-10-20", "desktop:foot", "Foot", None, "", 5).unwrap();
        assert_eq!(db.get_app_usage_seconds(1000, "2026-10-20", "desktop:foot").unwrap(), 5);
    }

    #[test]
    fn server_connection_roundtrips_cloud_account() {
        let db = Db::open(Some(":memory:")).unwrap();

        db.save_server_connection(&conn("wss://api.example/ws", Some("a@b.net"))).unwrap();
        assert_eq!(
            db.get_server_connection().unwrap().unwrap().cloud_account.as_deref(),
            Some("a@b.net"),
        );

        // A local (no-account) pairing stores NULL and reads back as None.
        db.save_server_connection(&conn("ws://lan/ws", None)).unwrap();
        assert_eq!(db.get_server_connection().unwrap().unwrap().cloud_account, None);
    }

    #[test]
    fn migration_adds_cloud_account_to_legacy_server_connection() {
        // A DB created before this column existed.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE server_connection (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                server_url TEXT NOT NULL, auth_token TEXT NOT NULL,
                agent_id TEXT NOT NULL, paired_at INTEGER NOT NULL, last_sync_at INTEGER
             );
             INSERT INTO server_connection (id, server_url, auth_token, agent_id, paired_at)
             VALUES (1, 'ws://lan/ws', 't', 'a', 0);",
        ).unwrap();
        let db = Db { conn };

        db.migrate().unwrap();

        assert_eq!(db.get_server_connection().unwrap().unwrap().cloud_account, None);
    }

    #[test]
    fn wipe_for_rebind_clears_pairing_and_policy_but_keeps_capabilities() {
        let db = Db::open(Some(":memory:")).unwrap();
        db.save_server_connection(&conn("wss://api.example/ws", Some("old@acct.net"))).unwrap();
        db.save_config_version(47).unwrap();
        db.save_capability("web_filter", true).unwrap();
        let mut cfg = user_config(1000, true);
        cfg.blocked_domains = vec!["youtube.com".to_string()];
        db.apply_config_push(&[cfg]).unwrap();
        db.add_usage_seconds(1000, "2026-08-28", 600).unwrap();
        db.upsert_server_remaining(1000, 30, "warn").unwrap();
        db.upsert_session(1000, "s1", false).unwrap();
        db.set_agent_mode(AgentMode::Online).unwrap();

        db.wipe_for_rebind().unwrap();

        assert!(db.get_server_connection().unwrap().is_none());
        assert_eq!(db.get_config_version().unwrap(), 0);
        assert!(db.get_managed_uids().unwrap().is_empty());
        assert!(db.get_cached_blocked_domains(1000).unwrap().is_empty());
        assert_eq!(db.get_usage_seconds(1000, "2026-08-28").unwrap(), 0);
        assert!(db.get_server_remaining(1000).unwrap().is_none());
        assert!(db.get_all_session_ids(1000).unwrap().is_empty());
        assert_eq!(db.get_agent_mode().unwrap(), AgentMode::Unpaired);
        // Hardware capability describes the machine, not the tenant — kept.
        assert_eq!(db.get_capability("web_filter").unwrap(), Some(true));
    }

    #[test]
    fn reset_pairing_leaves_cached_policy_untouched() {
        // Guards the backwards-compat promise: --reset must stay narrow.
        let db = Db::open(Some(":memory:")).unwrap();
        db.save_server_connection(&conn("ws://lan/ws", None)).unwrap();
        db.apply_config_push(&[user_config(1000, true)]).unwrap();
        db.add_usage_seconds(1000, "2026-08-28", 600).unwrap();
        db.set_agent_mode(AgentMode::Online).unwrap();

        db.reset_pairing().unwrap();

        assert!(db.get_server_connection().unwrap().is_none());
        assert_eq!(db.get_agent_mode().unwrap(), AgentMode::Unpaired);
        assert_eq!(db.get_managed_uids().unwrap(), vec![1000]);
        assert_eq!(db.get_usage_seconds(1000, "2026-08-28").unwrap(), 600);
    }

    #[test]
    fn config_push_caches_preserve_tasks_setting() {
        let db = Db::open(Some(":memory:")).unwrap();

        db.apply_config_push(&[user_config(1000, true)]).unwrap();
        assert!(db.get_cached_enforcement(1000).unwrap().preserve_tasks_on_lock);

        db.apply_config_push(&[user_config(1000, false)]).unwrap();
        assert!(!db.get_cached_enforcement(1000).unwrap().preserve_tasks_on_lock);
    }

    #[test]
    fn save_and_get_capability() {
        let db = Db::open(Some(":memory:")).unwrap();
        assert_eq!(db.get_capability("web_filter").unwrap(), None);
        db.save_capability("web_filter", true).unwrap();
        assert_eq!(db.get_capability("web_filter").unwrap(), Some(true));
        db.save_capability("web_filter", false).unwrap();
        assert_eq!(db.get_capability("web_filter").unwrap(), Some(false));
    }

    #[test]
    fn config_push_caches_blocked_domains() {
        let db = Db::open(Some(":memory:")).unwrap();
        let mut cfg = user_config(1000, false);
        cfg.blocked_domains = vec!["youtube.com".to_string(), "tiktok.com".to_string()];
        db.apply_config_push(&[cfg]).unwrap();
        let cached = db.get_cached_blocked_domains(1000).unwrap();
        assert_eq!(cached, vec!["tiktok.com", "youtube.com"]); // ORDER BY domain
    }

    #[test]
    fn config_push_replaces_cached_blocked_domains() {
        let db = Db::open(Some(":memory:")).unwrap();
        let mut cfg = user_config(1000, false);
        cfg.blocked_domains = vec!["youtube.com".to_string()];
        db.apply_config_push(&[cfg]).unwrap();

        let mut cfg2 = user_config(1000, false);
        cfg2.blocked_domains = vec!["discord.com".to_string()];
        db.apply_config_push(&[cfg2]).unwrap();

        let cached = db.get_cached_blocked_domains(1000).unwrap();
        assert_eq!(cached, vec!["discord.com"]);
    }

    #[test]
    fn save_and_get_cached_blocked_domains() {
        let db = Db::open(Some(":memory:")).unwrap();
        let mut cfg = user_config(1000, false);
        cfg.blocked_domains = Vec::new();
        db.apply_config_push(&[cfg]).unwrap();

        db.save_cached_blocked_domains(1000, &["instagram.com".to_string(), "reddit.com".to_string()]).unwrap();
        let cached = db.get_cached_blocked_domains(1000).unwrap();
        assert_eq!(cached, vec!["instagram.com", "reddit.com"]);
    }

    fn all_sessions(db: &Db) -> Vec<(u32, String)> {
        let mut stmt = db.conn.prepare(
            "SELECT local_uid, session_id FROM active_sessions ORDER BY local_uid, session_id",
        ).unwrap();
        stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    #[test]
    fn upsert_session_evicts_stale_owner_for_reused_id() {
        let db = Db::open(Some(":memory:")).unwrap();
        db.upsert_session(1001, "c9", false).unwrap();
        db.upsert_session(1002, "c9", false).unwrap();
        assert_eq!(all_sessions(&db), vec![(1002, "c9".to_string())]);
    }

    #[test]
    fn reconcile_removes_sessions_absent_from_live_snapshot() {
        let db = Db::open(Some(":memory:")).unwrap();
        db.upsert_session(1001, "c2", false).unwrap();
        db.upsert_session(1001, "c7", false).unwrap();

        db.reconcile_sessions(&[(1001, "c2".to_string())]).unwrap();

        assert_eq!(all_sessions(&db), vec![(1001, "c2".to_string())]);
    }

    #[test]
    fn reconcile_keeps_all_sessions_that_are_still_live() {
        let db = Db::open(Some(":memory:")).unwrap();
        db.upsert_session(1001, "c2", false).unwrap();
        db.upsert_session(1001, "c7", false).unwrap();

        db.reconcile_sessions(&[(1001, "c2".to_string()), (1001, "c7".to_string())]).unwrap();

        assert_eq!(all_sessions(&db), vec![
            (1001, "c2".to_string()),
            (1001, "c7".to_string()),
        ]);
    }

    #[test]
    fn reconcile_with_empty_snapshot_clears_all() {
        let db = Db::open(Some(":memory:")).unwrap();
        db.upsert_session(1001, "c2", false).unwrap();

        db.reconcile_sessions(&[]).unwrap();

        assert!(all_sessions(&db).is_empty());
    }
}
