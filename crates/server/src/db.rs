use anyhow::{Context, Result};
use chrono::{NaiveDate, NaiveTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

pub type DbPool = sqlx::AnyPool;

const CURRENT_VERSION: i32 = 11;

// ── open / schema ─────────────────────────────────────────────────────────────

pub async fn open(cfg: &crate::config::ServerConfig) -> Result<DbPool> {
    sqlx::any::install_default_drivers();

    let (url, is_sqlite) = if let Some(db_url) = &cfg.database_url {
        let is_sqlite = db_url.starts_with("sqlite");
        (db_url.clone(), is_sqlite)
    } else {
        if let Some(parent) = std::path::Path::new(&cfg.db_path).parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create DB directory: {}", parent.display()))?;
        }
        (format!("sqlite:{}?mode=rwc", cfg.db_path), true)
    };

    let pool = sqlx::AnyPool::connect(&url)
        .await
        .context("Failed to open database")?;

    if is_sqlite {
        sqlx::query("PRAGMA journal_mode=WAL").execute(&pool).await?;
        sqlx::query("PRAGMA foreign_keys=ON").execute(&pool).await?;
    }

    create_tables(&pool).await?;
    run_migrations(&pool, is_sqlite).await?;

    Ok(pool)
}

async fn create_tables(pool: &DbPool) -> Result<()> {
    let stmts = [
        "CREATE TABLE IF NOT EXISTS admin_users (
            id              TEXT NOT NULL PRIMARY KEY,
            username        TEXT NOT NULL UNIQUE,
            password_hash   TEXT NOT NULL,
            timezone        TEXT NOT NULL DEFAULT 'UTC',
            must_change_password INTEGER NOT NULL DEFAULT 0,
            created_at      INTEGER NOT NULL
        )",
        "CREATE TABLE IF NOT EXISTS agents (
            id                   TEXT NOT NULL PRIMARY KEY,
            machine_id           TEXT NOT NULL UNIQUE,
            display_name         TEXT NOT NULL,
            hostname             TEXT NOT NULL,
            timezone             TEXT NOT NULL DEFAULT 'UTC',
            status               TEXT NOT NULL DEFAULT 'pending'
                                 CHECK (status IN ('pending','paired','disabled','pending_delete')),
            auth_token_hash      TEXT,
            token_verified       INTEGER NOT NULL DEFAULT 0,
            agent_version        TEXT,
            paired_at            INTEGER,
            last_seen_at         INTEGER,
            created_at           INTEGER NOT NULL,
            web_filter_available INTEGER,
            app_usage_available  INTEGER
        )",
        "CREATE TABLE IF NOT EXISTS user_profiles (
            id              TEXT NOT NULL PRIMARY KEY,
            display_name    TEXT NOT NULL,
            language        TEXT NOT NULL DEFAULT 'en',
            created_at      INTEGER NOT NULL,
            updated_at      INTEGER NOT NULL
        )",
        "CREATE TABLE IF NOT EXISTS agent_users (
            id               TEXT NOT NULL PRIMARY KEY,
            agent_id         TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
            profile_id       TEXT REFERENCES user_profiles(id) ON DELETE SET NULL,
            local_uid        INTEGER NOT NULL,
            local_username   TEXT NOT NULL,
            display_name     TEXT,
            status           TEXT NOT NULL DEFAULT 'unmanaged'
                             CHECK (status IN ('unmanaged','managed','deleted')),
            first_seen_at    INTEGER NOT NULL,
            last_reported_at INTEGER NOT NULL,
            UNIQUE(agent_id, local_uid)
        )",
        "CREATE TABLE IF NOT EXISTS schedules (
            id          TEXT NOT NULL PRIMARY KEY,
            profile_id  TEXT NOT NULL REFERENCES user_profiles(id) ON DELETE CASCADE,
            day_of_week INTEGER NOT NULL CHECK (day_of_week BETWEEN 0 AND 6),
            start_time  TEXT NOT NULL,
            end_time    TEXT NOT NULL
        )",
        "CREATE TABLE IF NOT EXISTS daily_limits (
            profile_id      TEXT NOT NULL REFERENCES user_profiles(id) ON DELETE CASCADE,
            day_of_week     INTEGER NOT NULL CHECK (day_of_week BETWEEN 0 AND 6),
            allowed_minutes INTEGER NOT NULL CHECK (allowed_minutes >= 0),
            PRIMARY KEY (profile_id, day_of_week)
        )",
        "CREATE TABLE IF NOT EXISTS time_adjustments (
            id                 TEXT NOT NULL PRIMARY KEY,
            profile_id         TEXT NOT NULL REFERENCES user_profiles(id) ON DELETE CASCADE,
            target_date        TEXT NOT NULL,
            adjustment_minutes INTEGER NOT NULL,
            reason             TEXT,
            created_by         TEXT REFERENCES admin_users(id),
            created_at         INTEGER NOT NULL,
            synced_to_agents   INTEGER NOT NULL DEFAULT 0
        )",
        "CREATE TABLE IF NOT EXISTS enforcement_settings (
            profile_id              TEXT NOT NULL PRIMARY KEY REFERENCES user_profiles(id) ON DELETE CASCADE,
            lockout_grace_minutes   INTEGER NOT NULL DEFAULT 5,
            warning_thresholds      TEXT NOT NULL DEFAULT '15,5,1',
            preserve_tasks_on_lock  INTEGER NOT NULL DEFAULT 0
        )",
        "CREATE TABLE IF NOT EXISTS daily_usage (
            agent_user_id TEXT NOT NULL REFERENCES agent_users(id) ON DELETE CASCADE,
            date          TEXT NOT NULL,
            used_seconds  INTEGER NOT NULL DEFAULT 0,
            reported_at   INTEGER NOT NULL,
            PRIMARY KEY (agent_user_id, date)
        )",
        "CREATE TABLE IF NOT EXISTS config_versions (
            profile_id TEXT NOT NULL PRIMARY KEY REFERENCES user_profiles(id) ON DELETE CASCADE,
            version    INTEGER NOT NULL DEFAULT 1,
            updated_at INTEGER NOT NULL
        )",
        "CREATE TABLE IF NOT EXISTS audit_log (
            id            TEXT NOT NULL PRIMARY KEY,
            admin_user_id TEXT REFERENCES admin_users(id),
            action        TEXT NOT NULL,
            target_type   TEXT,
            target_id     TEXT,
            detail        TEXT,
            created_at    INTEGER NOT NULL
        )",
        "CREATE TABLE IF NOT EXISTS blocked_domains (
            id         TEXT NOT NULL PRIMARY KEY,
            profile_id TEXT NOT NULL REFERENCES user_profiles(id) ON DELETE CASCADE,
            domain     TEXT NOT NULL,
            enabled    INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL,
            UNIQUE (profile_id, domain)
        )",
"CREATE INDEX IF NOT EXISTS idx_agent_users_agent    ON agent_users(agent_id)",
        "CREATE INDEX IF NOT EXISTS idx_agent_users_profile  ON agent_users(profile_id)",
        "CREATE INDEX IF NOT EXISTS idx_schedules_profile    ON schedules(profile_id)",
        "CREATE INDEX IF NOT EXISTS idx_daily_usage_date     ON daily_usage(date)",
        "CREATE INDEX IF NOT EXISTS idx_adjustments_profile  ON time_adjustments(profile_id, target_date)",
        "CREATE INDEX IF NOT EXISTS idx_blocked_domains_profile ON blocked_domains(profile_id)",
    ];
    for stmt in stmts.iter().chain(APP_USAGE_DDL.iter()) {
        sqlx::query(stmt).execute(pool).await?;
    }
    Ok(())
}

/// Per-app usage tables (#27): created by `create_tables` on fresh installs and
/// by migration v10 on upgrades.
const APP_USAGE_DDL: [&str; 3] = [
    "CREATE TABLE IF NOT EXISTS app_catalog (
        app_id       TEXT NOT NULL PRIMARY KEY,
        display_name TEXT NOT NULL,
        icon         TEXT,
        categories   TEXT NOT NULL DEFAULT '',
        first_seen   INTEGER NOT NULL,
        last_seen    INTEGER NOT NULL
    )",
    "CREATE TABLE IF NOT EXISTS app_usage (
        agent_user_id TEXT NOT NULL REFERENCES agent_users(id) ON DELETE CASCADE,
        date          TEXT NOT NULL,
        app_id        TEXT NOT NULL,
        seconds       INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY (agent_user_id, date, app_id)
    )",
    "CREATE INDEX IF NOT EXISTS idx_app_usage_date ON app_usage(date)",
];

async fn run_migrations(pool: &DbPool, is_sqlite: bool) -> Result<()> {
    let mut v = get_schema_version(pool, is_sqlite).await?;

    // ── SQLite-only migrations (v1–v6) ────────────────────────────────────────
    // These use SQLite-specific DDL (sqlite_master, PRAGMA, table recreation).
    // Postgres always provisions with the final create_tables() DDL and skips these.
    if is_sqlite {
        if v == 0 {
            // Distinguish a fresh database from a very-old pre-migration one.
            let admin_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM admin_users")
                .fetch_one(pool)
                .await?;
            if admin_count == 0 {
                v = CURRENT_VERSION;
                set_schema_version(pool, v).await?;
            }
        }
        if v < 1 { apply_v1(pool).await?; v = 1; set_schema_version(pool, v).await?; }
        if v < 2 { apply_v2(pool).await?; v = 2; set_schema_version(pool, v).await?; }
        if v < 3 { apply_v3(pool).await?; v = 3; set_schema_version(pool, v).await?; }
        if v < 4 { apply_v4(pool).await?; v = 4; set_schema_version(pool, v).await?; }
        if v < 5 { apply_v5(pool).await?; v = 5; set_schema_version(pool, v).await?; }
        if v < 6 { apply_v6(pool).await?; v = 6; set_schema_version(pool, v).await?; }
    } else if v == 0 {
        // Postgres fresh install: create_tables() already built the final schema,
        // so mark as current and skip the portable migrations below.
        v = CURRENT_VERSION;
        set_schema_version(pool, v).await?;
    } else if v < 6 {
        // Existing Postgres database: the SQLite-only migrations don't apply, but
        // the portable ones below must still run (bumping straight to
        // CURRENT_VERSION here would silently skip them).
        v = 6;
        set_schema_version(pool, v).await?;
    }

    // ── Portable migrations (v7+) — run on BOTH SQLite and Postgres ───────────
    // Write in standard SQL only: no PRAGMA, no sqlite_master, no table recreation.
    // Add new entries here; increment CURRENT_VERSION at the top of this file.
    //
    // v7 is intentionally unused: a pre-release fork build of multi-admin (#14)
    // shipped a v7 (is_owner) that was dropped before merge. Skipping straight
    // to v8 means such databases still receive the real migration.
    if v < 8 { apply_v8(pool).await?; v = 8; set_schema_version(pool, v).await?; }
    if v < 9 { apply_v9(pool).await?; v = 9; set_schema_version(pool, v).await?; }
    if v < 10 { apply_v10(pool).await?; v = 10; set_schema_version(pool, v).await?; }
    if v < 11 { apply_v11(pool).await?; v = 11; set_schema_version(pool, v).await?; }

    Ok(())
}

async fn get_schema_version(pool: &DbPool, is_sqlite: bool) -> Result<i32> {
    // Try reading the version table that was introduced with the sqlx migration.
    let existing: Option<i32> = sqlx::query_scalar("SELECT version FROM _schema_version LIMIT 1")
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();

    if let Some(v) = existing {
        return Ok(v);
    }

    // First run after switching to sqlx — bootstrap from PRAGMA user_version (SQLite only).
    sqlx::query("CREATE TABLE IF NOT EXISTS _schema_version (version INTEGER NOT NULL DEFAULT 0)")
        .execute(pool)
        .await?;

    let pragma_v: i32 = if is_sqlite {
        sqlx::query("PRAGMA user_version")
            .fetch_one(pool)
            .await
            .map(|r| r.get::<i32, _>(0))
            .unwrap_or(0)
    } else {
        0
    };

    sqlx::query("INSERT INTO _schema_version (version) VALUES ($1)")
        .bind(pragma_v)
        .execute(pool)
        .await?;

    Ok(pragma_v)
}

async fn set_schema_version(pool: &DbPool, v: i32) -> Result<()> {
    sqlx::query("UPDATE _schema_version SET version = $1")
        .bind(v)
        .execute(pool)
        .await?;
    Ok(())
}

// Migration 1: add 'pending_delete' to the agents status CHECK constraint.
pub(crate) async fn apply_v1(pool: &DbPool) -> Result<()> {
    let schema: Option<String> = sqlx::query_scalar(
        "SELECT sql FROM sqlite_master WHERE type='table' AND name='agents'",
    )
    .fetch_optional(pool)
    .await?;

    if schema.as_deref().map_or(true, |s| s.contains("pending_delete")) {
        return Ok(());
    }

    sqlx::query("PRAGMA foreign_keys = OFF").execute(pool).await?;
    sqlx::query("CREATE TABLE agents_v1 (
        id              TEXT PRIMARY KEY,
        machine_id      TEXT NOT NULL UNIQUE,
        display_name    TEXT NOT NULL,
        hostname        TEXT NOT NULL,
        timezone        TEXT NOT NULL DEFAULT 'UTC',
        status          TEXT NOT NULL DEFAULT 'pending'
                        CHECK (status IN ('pending','paired','disabled','pending_delete')),
        auth_token_hash TEXT,
        agent_version   TEXT,
        paired_at       INTEGER,
        last_seen_at    INTEGER,
        created_at      INTEGER NOT NULL,
        web_filter_available INTEGER
    )").execute(pool).await?;
    sqlx::query(
        "INSERT INTO agents_v1 (id,machine_id,display_name,hostname,timezone,status,
         auth_token_hash,agent_version,paired_at,last_seen_at,created_at)
         SELECT id,machine_id,display_name,hostname,timezone,status,
         auth_token_hash,agent_version,paired_at,last_seen_at,created_at FROM agents",
    )
    .execute(pool)
    .await?;
    sqlx::query("DROP TABLE agents").execute(pool).await?;
    sqlx::query("ALTER TABLE agents_v1 RENAME TO agents").execute(pool).await?;
    sqlx::query("PRAGMA foreign_keys = ON").execute(pool).await?;
    tracing::info!("DB migration v1 applied (pending_delete status)");
    Ok(())
}

// Migration 2: add language column to user_profiles.
pub(crate) async fn apply_v2(pool: &DbPool) -> Result<()> {
    sqlx::query(
        "ALTER TABLE user_profiles ADD COLUMN language TEXT NOT NULL DEFAULT 'en'",
    )
    .execute(pool)
    .await
    .ok(); // Ignore error if column already exists (idempotent)
    tracing::info!("DB migration v2 applied (profile language)");
    Ok(())
}

// Migration 3: add timezone column to admin_users.
pub(crate) async fn apply_v3(pool: &DbPool) -> Result<()> {
    sqlx::query(
        "ALTER TABLE admin_users ADD COLUMN timezone TEXT NOT NULL DEFAULT 'UTC'",
    )
    .execute(pool)
    .await
    .ok();
    tracing::info!("DB migration v3 applied (admin timezone)");
    Ok(())
}

// Migration 4: allow allowed_minutes = 0 (blocked day).
pub(crate) async fn apply_v4(pool: &DbPool) -> Result<()> {
    let schema: Option<String> = sqlx::query_scalar(
        "SELECT sql FROM sqlite_master WHERE type='table' AND name='daily_limits'",
    )
    .fetch_optional(pool)
    .await?;

    // Old schema had CHECK (allowed_minutes > 0); new allows >= 0.
    if schema.as_deref().map_or(true, |s| s.contains(">= 0") || !s.contains("> 0")) {
        return Ok(());
    }

    sqlx::query("PRAGMA foreign_keys = OFF").execute(pool).await?;
    sqlx::query("CREATE TABLE daily_limits_v4 (
        profile_id      TEXT NOT NULL REFERENCES user_profiles(id) ON DELETE CASCADE,
        day_of_week     INTEGER NOT NULL CHECK (day_of_week BETWEEN 0 AND 6),
        allowed_minutes INTEGER NOT NULL CHECK (allowed_minutes >= 0),
        PRIMARY KEY (profile_id, day_of_week)
    )")
    .execute(pool)
    .await?;
    sqlx::query("INSERT INTO daily_limits_v4 SELECT * FROM daily_limits")
        .execute(pool)
        .await?;
    sqlx::query("DROP TABLE daily_limits").execute(pool).await?;
    sqlx::query("ALTER TABLE daily_limits_v4 RENAME TO daily_limits")
        .execute(pool)
        .await?;
    sqlx::query("PRAGMA foreign_keys = ON").execute(pool).await?;
    tracing::info!("DB migration v4 applied (allow zero daily_limits)");
    Ok(())
}

// Migration 5: add preserve_tasks_on_lock to enforcement_settings.
pub(crate) async fn apply_v5(pool: &DbPool) -> Result<()> {
    sqlx::query(
        "ALTER TABLE enforcement_settings ADD COLUMN preserve_tasks_on_lock INTEGER NOT NULL DEFAULT 0",
    )
    .execute(pool)
    .await
    .ok();
    tracing::info!("DB migration v5 applied (preserve tasks on lock)");
    Ok(())
}

// Migration 6: add web_filter_available to agents and create blocked_domains table.
pub(crate) async fn apply_v6(pool: &DbPool) -> Result<()> {
    sqlx::query("ALTER TABLE agents ADD COLUMN web_filter_available INTEGER")
        .execute(pool)
        .await
        .ok();
    sqlx::query("CREATE TABLE IF NOT EXISTS blocked_domains (
        id         TEXT NOT NULL PRIMARY KEY,
        profile_id TEXT NOT NULL REFERENCES user_profiles(id) ON DELETE CASCADE,
        domain     TEXT NOT NULL,
        enabled    INTEGER NOT NULL DEFAULT 0,
        created_at INTEGER NOT NULL,
        UNIQUE (profile_id, domain)
    )")
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_blocked_domains_profile ON blocked_domains(profile_id)",
    )
    .execute(pool)
    .await?;
    tracing::info!("DB migration v6 applied (web filter)");
    Ok(())
}

// Migration 8: multi-admin support.
// - must_change_password: set when an admin creates/resets another account, so the
//   initial password chosen by someone else must be replaced on first login.
// - The timezone is installation-wide (schedules are interpreted in it). It still
//   lives on admin_users, but every row is kept identical; sync existing rows to
//   the original admin's value.
pub(crate) async fn apply_v8(pool: &DbPool) -> Result<()> {
    sqlx::query("ALTER TABLE admin_users ADD COLUMN must_change_password INTEGER NOT NULL DEFAULT 0")
        .execute(pool)
        .await?;
    sqlx::query(
        "UPDATE admin_users SET timezone =
         (SELECT timezone FROM admin_users ORDER BY created_at ASC LIMIT 1)",
    )
    .execute(pool)
    .await?;
    tracing::info!("DB migration v8 applied (multi-admin)");
    Ok(())
}

// Migration 9: agent auth tokens are now verified on connect. Older servers
// never checked them, and a double "accept" could overwrite the stored hash
// with a token the agent never received — so existing pairings start
// unverified and adopt the token they present on their first connect
// (see `verify_agent_token`) instead of being locked out.
pub(crate) async fn apply_v9(pool: &DbPool) -> Result<()> {
    sqlx::query("ALTER TABLE agents ADD COLUMN token_verified INTEGER NOT NULL DEFAULT 0")
        .execute(pool)
        .await?;
    tracing::info!("DB migration v9 applied (agent token verification)");
    Ok(())
}

// Migration 10: per-app usage reports (#27). Day totals per agent user and app,
// a catalogue of names/icons, and the agents' `app_usage` capability.
pub(crate) async fn apply_v10(pool: &DbPool) -> Result<()> {
    for stmt in APP_USAGE_DDL {
        sqlx::query(stmt).execute(pool).await?;
    }
    sqlx::query("ALTER TABLE agents ADD COLUMN app_usage_available INTEGER")
        .execute(pool)
        .await?;
    tracing::info!("DB migration v10 applied (per-app usage)");
    Ok(())
}

// Migration 11: app categories (`Game`, `WebBrowser`, …) for predefined groups.
// `create_tables` runs first and already builds the final `app_catalog` on a
// database that didn't have it, so only add the column where it's missing.
pub(crate) async fn apply_v11(pool: &DbPool) -> Result<()> {
    let has_column = sqlx::query("SELECT categories FROM app_catalog LIMIT 1").fetch_optional(pool).await.is_ok();
    if !has_column {
        sqlx::query("ALTER TABLE app_catalog ADD COLUMN categories TEXT NOT NULL DEFAULT ''")
            .execute(pool)
            .await?;
    }
    tracing::info!("DB migration v11 applied (app categories)");
    Ok(())
}

// ── models ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminUser {
    pub id: Uuid,
    pub username: String,
    pub password_hash: String,
    pub created_at: i64,
    pub timezone: String,
    pub must_change_password: bool,
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
    pub web_filter_available: Option<bool>,
    pub app_usage_available: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockedDomain {
    pub id: Uuid,
    pub profile_id: Uuid,
    pub domain: String,
    pub enabled: bool,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserProfile {
    pub id: Uuid,
    pub display_name: String,
    pub language: String,
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
    pub preserve_tasks_on_lock: bool,
    pub warning_thresholds: Vec<i32>,
}

// ── admin_users ───────────────────────────────────────────────────────────────

pub async fn admin_count(pool: &DbPool) -> Result<i64> {
    Ok(sqlx::query_scalar("SELECT COUNT(*) FROM admin_users")
        .fetch_one(pool)
        .await?)
}

pub async fn create_admin(
    pool: &DbPool,
    username: &str,
    password_hash: &str,
    must_change_password: bool,
) -> Result<Uuid> {
    let id = Uuid::new_v4();
    // New admins inherit the installation-wide timezone.
    let timezone = get_admin_timezone(pool).await?;
    sqlx::query(
        "INSERT INTO admin_users (id, username, password_hash, timezone, must_change_password, created_at)
         VALUES ($1,$2,$3,$4,$5,$6)",
    )
    .bind(id.to_string())
    .bind(username)
    .bind(password_hash)
    .bind(timezone)
    .bind(must_change_password as i32)
    .bind(Utc::now().timestamp())
    .execute(pool)
    .await?;
    Ok(id)
}

fn row_to_admin_user(r: sqlx::any::AnyRow) -> AdminUser {
    AdminUser {
        id: r.get::<String, _>("id").parse().unwrap_or_default(),
        username: r.get("username"),
        password_hash: r.get("password_hash"),
        created_at: r.get("created_at"),
        timezone: r.get("timezone"),
        must_change_password: r.get::<i32, _>("must_change_password") != 0,
    }
}

pub async fn get_admin_by_username(pool: &DbPool, username: &str) -> Result<Option<AdminUser>> {
    let row = sqlx::query(
        "SELECT id, username, password_hash, created_at, timezone, must_change_password
         FROM admin_users WHERE username=$1",
    )
    .bind(username)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(row_to_admin_user))
}

pub async fn get_admin_user_by_id(pool: &DbPool, id: Uuid) -> Result<Option<AdminUser>> {
    let row = sqlx::query(
        "SELECT id, username, password_hash, created_at, timezone, must_change_password
         FROM admin_users WHERE id=$1",
    )
    .bind(id.to_string())
    .fetch_optional(pool)
    .await?;
    Ok(row.map(row_to_admin_user))
}

pub async fn list_admins(pool: &DbPool) -> Result<Vec<AdminUser>> {
    let rows = sqlx::query(
        "SELECT id, username, password_hash, created_at, timezone, must_change_password
         FROM admin_users ORDER BY created_at ASC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(row_to_admin_user).collect())
}

#[derive(Debug, PartialEq, Eq)]
pub enum DeleteAdmin {
    Deleted,
    NotFound,
    /// Refused: it is the only admin left. With zero admins /auth/setup would
    /// reopen to anyone on the network.
    LastAdmin,
}

/// Deletes an admin account, never the last one — even when two admins delete
/// each other at the same moment. A count-then-delete (or a DELETE with a COUNT
/// subquery) races on Postgres, so first touch every admin row inside the
/// transaction: that takes row locks on Postgres and the write lock on SQLite,
/// serialising concurrent deletes so the second one sees the first's result.
pub async fn delete_admin(pool: &DbPool, id: Uuid) -> Result<DeleteAdmin> {
    let mut tx = pool.begin().await?;
    sqlx::query("UPDATE admin_users SET username = username")
        .execute(&mut *tx)
        .await?;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM admin_users")
        .fetch_one(&mut *tx)
        .await?;
    let exists: Option<String> = sqlx::query_scalar("SELECT id FROM admin_users WHERE id=$1")
        .bind(id.to_string())
        .fetch_optional(&mut *tx)
        .await?;
    let outcome = if exists.is_none() {
        DeleteAdmin::NotFound
    } else if count <= 1 {
        DeleteAdmin::LastAdmin
    } else {
        sqlx::query("DELETE FROM admin_users WHERE id=$1")
            .bind(id.to_string())
            .execute(&mut *tx)
            .await?;
        DeleteAdmin::Deleted
    };
    tx.commit().await?;
    Ok(outcome)
}

/// Sets a new password. `must_change_password` is true when someone else chose
/// it (admin reset) and false when the account owner set it themselves.
pub async fn update_admin_password(
    pool: &DbPool,
    id: Uuid,
    password_hash: &str,
    must_change_password: bool,
) -> Result<()> {
    sqlx::query("UPDATE admin_users SET password_hash=$1, must_change_password=$2 WHERE id=$3")
        .bind(password_hash)
        .bind(must_change_password as i32)
        .bind(id.to_string())
        .execute(pool)
        .await?;
    Ok(())
}

/// The installation-wide timezone schedules are interpreted in. Every admin row
/// carries the same value; read the original admin's for determinism.
pub async fn get_admin_timezone(pool: &DbPool) -> Result<String> {
    let tz: Option<String> =
        sqlx::query_scalar("SELECT timezone FROM admin_users ORDER BY created_at ASC LIMIT 1")
        .fetch_optional(pool)
        .await?;
    Ok(tz.unwrap_or_else(|| "UTC".to_string()))
}

/// Sets the installation-wide timezone (on every admin row, see `get_admin_timezone`).
pub async fn set_admin_timezone(pool: &DbPool, timezone: &str) -> Result<()> {
    sqlx::query("UPDATE admin_users SET timezone=$1")
        .bind(timezone)
        .execute(pool)
        .await?;
    Ok(())
}

// ── agents ────────────────────────────────────────────────────────────────────

pub async fn upsert_agent_pending(
    pool: &DbPool,
    machine_id: &str,
    hostname: &str,
    timezone: &str,
    agent_version: &str,
) -> Result<Agent> {
    let now = Utc::now().timestamp();
    let existing: Option<String> =
        sqlx::query_scalar("SELECT id FROM agents WHERE machine_id=$1")
            .bind(machine_id)
            .fetch_optional(pool)
            .await?;

    let id = if let Some(id_str) = existing {
        sqlx::query(
            "UPDATE agents SET hostname=$1, timezone=$2, agent_version=$3, last_seen_at=$4,
             status=CASE WHEN status IN ('disabled','paired') THEN status ELSE 'pending' END
             WHERE machine_id=$5",
        )
        .bind(hostname)
        .bind(timezone)
        .bind(agent_version)
        .bind(now)
        .bind(machine_id)
        .execute(pool)
        .await?;
        id_str.parse().unwrap_or_else(|_| Uuid::new_v4())
    } else {
        let new_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO agents
             (id,machine_id,display_name,hostname,timezone,status,agent_version,created_at,last_seen_at)
             VALUES ($1,$2,$3,$4,$5,'pending',$6,$7,$7)",
        )
        .bind(new_id.to_string())
        .bind(machine_id)
        .bind(hostname)
        .bind(hostname)
        .bind(timezone)
        .bind(agent_version)
        .bind(now)
        .execute(pool)
        .await?;
        new_id
    };
    get_agent_by_id(pool, id).await?.context("Agent not found after upsert")
}

pub async fn get_agent_by_id(pool: &DbPool, id: Uuid) -> Result<Option<Agent>> {
    let row = sqlx::query(
        "SELECT id,machine_id,display_name,hostname,timezone,status,auth_token_hash,
                agent_version,paired_at,last_seen_at,created_at,web_filter_available,
                app_usage_available
         FROM agents WHERE id=$1",
    )
    .bind(id.to_string())
    .fetch_optional(pool)
    .await?;
    Ok(row.map(row_to_agent))
}

pub async fn get_agent_by_machine_id(pool: &DbPool, machine_id: &str) -> Result<Option<Agent>> {
    let row = sqlx::query(
        "SELECT id,machine_id,display_name,hostname,timezone,status,auth_token_hash,
                agent_version,paired_at,last_seen_at,created_at,web_filter_available,
                app_usage_available
         FROM agents WHERE machine_id=$1",
    )
    .bind(machine_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(row_to_agent))
}

pub async fn list_agents(pool: &DbPool) -> Result<Vec<Agent>> {
    let rows = sqlx::query(
        "SELECT id,machine_id,display_name,hostname,timezone,status,auth_token_hash,
                agent_version,paired_at,last_seen_at,created_at,web_filter_available,
                app_usage_available
         FROM agents ORDER BY created_at DESC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|r| row_to_agent(r)).collect())
}

pub async fn accept_agent(pool: &DbPool, id: Uuid, auth_token_hash: &str) -> Result<()> {
    let now = Utc::now().timestamp();
    sqlx::query(
        "UPDATE agents SET status='paired', auth_token_hash=$1, token_verified=1, paired_at=$2 WHERE id=$3",
    )
    .bind(auth_token_hash)
    .bind(now)
    .bind(id.to_string())
    .execute(pool)
    .await?;
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
pub enum TokenCheck {
    Valid,
    /// First connect of a pairing made before tokens were verified: the
    /// presented token has been adopted as this agent's token.
    Adopted,
    /// Pre-verification pairing that presented no token: let through as before,
    /// but nothing is adopted — an empty token must never become the credential.
    UnverifiedNoToken,
    Invalid,
}

/// Checks the bearer token (as its hash; `None` = no token sent) an agent
/// presented when connecting.
pub async fn verify_agent_token(pool: &DbPool, agent_id: Uuid, token_hash: Option<&str>) -> Result<TokenCheck> {
    let row = sqlx::query("SELECT auth_token_hash, token_verified FROM agents WHERE id=$1")
        .bind(agent_id.to_string())
        .fetch_optional(pool)
        .await?;
    let Some(row) = row else { return Ok(TokenCheck::Invalid) };
    let stored: Option<String> = row.get("auth_token_hash");
    let verified = row.get::<i32, _>("token_verified") != 0;

    let Some(token_hash) = token_hash else {
        return Ok(if verified { TokenCheck::Invalid } else { TokenCheck::UnverifiedNoToken });
    };
    if stored.as_deref() == Some(token_hash) {
        if !verified {
            sqlx::query("UPDATE agents SET token_verified=1 WHERE id=$1")
                .bind(agent_id.to_string())
                .execute(pool)
                .await?;
        }
        return Ok(TokenCheck::Valid);
    }
    if verified {
        return Ok(TokenCheck::Invalid);
    }
    // Legacy pairing: adopt atomically, so only the first presenter wins.
    let adopted = sqlx::query(
        "UPDATE agents SET auth_token_hash=$1, token_verified=1 WHERE id=$2 AND token_verified=0",
    )
    .bind(token_hash)
    .bind(agent_id.to_string())
    .execute(pool)
    .await?
    .rows_affected()
        > 0;
    Ok(if adopted { TokenCheck::Adopted } else { TokenCheck::Invalid })
}

pub async fn update_agent_last_seen(pool: &DbPool, id: Uuid) -> Result<()> {
    sqlx::query("UPDATE agents SET last_seen_at=$1 WHERE id=$2")
        .bind(Utc::now().timestamp())
        .bind(id.to_string())
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn update_agent_fields(
    pool: &DbPool,
    id: Uuid,
    display_name: Option<&str>,
    status: Option<&str>,
) -> Result<()> {
    if let Some(name) = display_name {
        sqlx::query("UPDATE agents SET display_name=$1 WHERE id=$2")
            .bind(name)
            .bind(id.to_string())
            .execute(pool)
            .await?;
    }
    if let Some(s) = status {
        sqlx::query("UPDATE agents SET status=$1 WHERE id=$2")
            .bind(s)
            .bind(id.to_string())
            .execute(pool)
            .await?;
    }
    Ok(())
}

pub async fn update_agent_hello(
    pool: &DbPool,
    id: Uuid,
    hostname: &str,
    timezone: &str,
    agent_version: &str,
) -> Result<()> {
    sqlx::query(
        "UPDATE agents SET hostname=$1, timezone=$2, agent_version=$3, last_seen_at=$4 WHERE id=$5",
    )
    .bind(hostname)
    .bind(timezone)
    .bind(agent_version)
    .bind(Utc::now().timestamp())
    .bind(id.to_string())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn mark_agent_pending_delete(pool: &DbPool, id: Uuid) -> Result<()> {
    sqlx::query("UPDATE agents SET status='pending_delete' WHERE id=$1")
        .bind(id.to_string())
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn restore_agent(pool: &DbPool, id: Uuid) -> Result<()> {
    sqlx::query(
        "UPDATE agents SET status='paired' WHERE id=$1 AND status='pending_delete'",
    )
    .bind(id.to_string())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete_agent(pool: &DbPool, id: Uuid) -> Result<()> {
    sqlx::query("DELETE FROM agents WHERE id=$1")
        .bind(id.to_string())
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn update_agent_app_usage(pool: &DbPool, id: Uuid, available: bool) -> Result<()> {
    sqlx::query("UPDATE agents SET app_usage_available=$1 WHERE id=$2")
        .bind(available as i32)
        .bind(id.to_string())
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn update_agent_web_filter(pool: &DbPool, id: Uuid, available: bool) -> Result<()> {
    sqlx::query("UPDATE agents SET web_filter_available=$1 WHERE id=$2")
        .bind(available as i32)
        .bind(id.to_string())
        .execute(pool)
        .await?;
    Ok(())
}

fn row_to_agent(r: sqlx::any::AnyRow) -> Agent {
    let wf: Option<i32> = r.get("web_filter_available");
    let au: Option<i32> = r.get("app_usage_available");
    Agent {
        id: r.get::<String, _>("id").parse().unwrap_or_default(),
        machine_id: r.get("machine_id"),
        display_name: r.get("display_name"),
        hostname: r.get("hostname"),
        timezone: r.get("timezone"),
        status: r.get("status"),
        auth_token_hash: r.get("auth_token_hash"),
        agent_version: r.get("agent_version"),
        paired_at: r.get("paired_at"),
        last_seen_at: r.get("last_seen_at"),
        created_at: r.get("created_at"),
        web_filter_available: wf.map(|v| v != 0),
        app_usage_available: au.map(|v| v != 0),
    }
}

// ── agent_users ───────────────────────────────────────────────────────────────

pub async fn upsert_agent_users(
    pool: &DbPool,
    agent_id: Uuid,
    users: &[common::models::LocalUser],
) -> Result<()> {
    let now = Utc::now().timestamp();
    for u in users {
        let existing: Option<String> = sqlx::query_scalar(
            "SELECT id FROM agent_users WHERE agent_id=$1 AND local_uid=$2",
        )
        .bind(agent_id.to_string())
        .bind(u.local_uid as i64)
        .fetch_optional(pool)
        .await?;

        if existing.is_some() {
            sqlx::query(
                "UPDATE agent_users SET local_username=$1, display_name=$2, last_reported_at=$3,
                 status=CASE WHEN status='deleted' THEN 'unmanaged' ELSE status END
                 WHERE agent_id=$4 AND local_uid=$5",
            )
            .bind(&u.username)
            .bind(&u.display_name)
            .bind(now)
            .bind(agent_id.to_string())
            .bind(u.local_uid as i64)
            .execute(pool)
            .await?;
        } else {
            sqlx::query(
                "INSERT INTO agent_users
                 (id,agent_id,local_uid,local_username,display_name,status,first_seen_at,last_reported_at)
                 VALUES ($1,$2,$3,$4,$5,'unmanaged',$6,$6)",
            )
            .bind(Uuid::new_v4().to_string())
            .bind(agent_id.to_string())
            .bind(u.local_uid as i64)
            .bind(&u.username)
            .bind(&u.display_name)
            .bind(now)
            .execute(pool)
            .await?;
        }
    }
    Ok(())
}

pub async fn mark_agent_users_deleted(
    pool: &DbPool,
    agent_id: Uuid,
    uids: &[u32],
) -> Result<()> {
    for uid in uids {
        sqlx::query(
            "UPDATE agent_users SET status='deleted' WHERE agent_id=$1 AND local_uid=$2",
        )
        .bind(agent_id.to_string())
        .bind(*uid as i64)
        .execute(pool)
        .await?;
    }
    Ok(())
}

pub async fn list_agent_users(pool: &DbPool, agent_id: Uuid) -> Result<Vec<AgentUser>> {
    let rows = sqlx::query(
        "SELECT id,agent_id,profile_id,local_uid,local_username,display_name,status,
                first_seen_at,last_reported_at
         FROM agent_users WHERE agent_id=$1 AND status!='deleted' ORDER BY local_uid",
    )
    .bind(agent_id.to_string())
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(row_to_agent_user).collect())
}

pub async fn get_agent_user_by_id(pool: &DbPool, id: Uuid) -> Result<Option<AgentUser>> {
    let row = sqlx::query(
        "SELECT id,agent_id,profile_id,local_uid,local_username,display_name,status,
                first_seen_at,last_reported_at
         FROM agent_users WHERE id=$1",
    )
    .bind(id.to_string())
    .fetch_optional(pool)
    .await?;
    Ok(row.map(row_to_agent_user))
}

pub async fn get_agent_user(
    pool: &DbPool,
    agent_id: Uuid,
    local_uid: u32,
) -> Result<Option<AgentUser>> {
    let row = sqlx::query(
        "SELECT id,agent_id,profile_id,local_uid,local_username,display_name,status,
                first_seen_at,last_reported_at
         FROM agent_users WHERE agent_id=$1 AND local_uid=$2",
    )
    .bind(agent_id.to_string())
    .bind(local_uid as i64)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(row_to_agent_user))
}

pub async fn update_agent_user(
    pool: &DbPool,
    id: Uuid,
    profile_id: Option<Uuid>,
    status: Option<&str>,
) -> Result<()> {
    let profile_str = profile_id.map(|p| p.to_string());
    sqlx::query(
        "UPDATE agent_users SET profile_id=COALESCE($1, profile_id),
         status=COALESCE($2, status) WHERE id=$3",
    )
    .bind(profile_str)
    .bind(status)
    .bind(id.to_string())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_agent_users_for_profile(
    pool: &DbPool,
    profile_id: Uuid,
) -> Result<Vec<AgentUser>> {
    let rows = sqlx::query(
        "SELECT id,agent_id,profile_id,local_uid,local_username,display_name,status,
                first_seen_at,last_reported_at
         FROM agent_users WHERE profile_id=$1 AND status='managed'",
    )
    .bind(profile_id.to_string())
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(row_to_agent_user).collect())
}

fn row_to_agent_user(r: sqlx::any::AnyRow) -> AgentUser {
    AgentUser {
        id: r.get::<String, _>("id").parse().unwrap_or_default(),
        agent_id: r.get::<String, _>("agent_id").parse().unwrap_or_default(),
        // Explicit Option<String> so NULL decodes as None rather than InvalidColumnType;
        // .and_then flattens Option<Option<String>> → Option<Uuid>.
        profile_id: r
            .get::<Option<String>, _>("profile_id")
            .and_then(|s| s.parse().ok()),
        local_uid: r.get("local_uid"),
        local_username: r.get("local_username"),
        display_name: r.get("display_name"),
        status: r.get("status"),
        first_seen_at: r.get("first_seen_at"),
        last_reported_at: r.get("last_reported_at"),
    }
}

// ── user_profiles ─────────────────────────────────────────────────────────────

const DEFAULT_BLOCKED_DOMAINS: &[&str] = &[
    "youtube.com",
    "tiktok.com",
    "instagram.com",
    "facebook.com",
    "twitter.com",
    "x.com",
    "twitch.tv",
    "discord.com",
    "reddit.com",
    "snapchat.com",
    "roblox.com",
];

pub async fn create_profile(pool: &DbPool, display_name: &str) -> Result<UserProfile> {
    let id = Uuid::new_v4();
    let now = Utc::now().timestamp();
    sqlx::query(
        "INSERT INTO user_profiles (id, display_name, created_at, updated_at) VALUES ($1,$2,$3,$3)",
    )
    .bind(id.to_string())
    .bind(display_name)
    .bind(now)
    .execute(pool)
    .await?;
    sqlx::query("INSERT INTO enforcement_settings (profile_id) VALUES ($1)")
        .bind(id.to_string())
        .execute(pool)
        .await?;
    sqlx::query(
        "INSERT INTO config_versions (profile_id, version, updated_at) VALUES ($1, 1, $2)",
    )
    .bind(id.to_string())
    .bind(now)
    .execute(pool)
    .await?;
    for domain in DEFAULT_BLOCKED_DOMAINS {
        sqlx::query(
            "INSERT INTO blocked_domains (id, profile_id, domain, enabled, created_at)
             VALUES ($1, $2, $3, 0, $4)
             ON CONFLICT DO NOTHING",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(id.to_string())
        .bind(*domain)
        .bind(now)
        .execute(pool)
        .await?;
    }
    Ok(UserProfile {
        id,
        display_name: display_name.to_string(),
        language: "en".to_string(),
        created_at: now,
        updated_at: now,
    })
}

pub async fn list_profiles(pool: &DbPool) -> Result<Vec<UserProfile>> {
    let rows = sqlx::query(
        "SELECT id, display_name, language, created_at, updated_at
         FROM user_profiles ORDER BY display_name",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(row_to_profile).collect())
}

pub async fn get_profile(pool: &DbPool, id: Uuid) -> Result<Option<UserProfile>> {
    let row = sqlx::query(
        "SELECT id, display_name, language, created_at, updated_at
         FROM user_profiles WHERE id=$1",
    )
    .bind(id.to_string())
    .fetch_optional(pool)
    .await?;
    Ok(row.map(row_to_profile))
}

pub async fn update_profile_language(pool: &DbPool, id: Uuid, language: &str) -> Result<()> {
    sqlx::query("UPDATE user_profiles SET language=$1, updated_at=$2 WHERE id=$3")
        .bind(language)
        .bind(Utc::now().timestamp())
        .bind(id.to_string())
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn update_profile(pool: &DbPool, id: Uuid, display_name: &str) -> Result<()> {
    sqlx::query("UPDATE user_profiles SET display_name=$1, updated_at=$2 WHERE id=$3")
        .bind(display_name)
        .bind(Utc::now().timestamp())
        .bind(id.to_string())
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn delete_profile(pool: &DbPool, id: Uuid) -> Result<()> {
    sqlx::query("DELETE FROM user_profiles WHERE id=$1")
        .bind(id.to_string())
        .execute(pool)
        .await?;
    Ok(())
}

fn row_to_profile(r: sqlx::any::AnyRow) -> UserProfile {
    UserProfile {
        id: r.get::<String, _>("id").parse().unwrap_or_default(),
        display_name: r.get("display_name"),
        language: r.get("language"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }
}

// ── schedules ─────────────────────────────────────────────────────────────────

pub async fn get_schedules(pool: &DbPool, profile_id: Uuid) -> Result<Vec<Schedule>> {
    let rows = sqlx::query(
        "SELECT id, profile_id, day_of_week, start_time, end_time
         FROM schedules WHERE profile_id=$1 ORDER BY day_of_week, start_time",
    )
    .bind(profile_id.to_string())
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| Schedule {
            id: r.get::<String, _>("id").parse().unwrap_or_default(),
            profile_id: r.get::<String, _>("profile_id").parse().unwrap_or_default(),
            day_of_week: r.get::<i64, _>("day_of_week") as u8,
            start_time: r.get("start_time"),
            end_time: r.get("end_time"),
        })
        .collect())
}

pub async fn replace_schedules(
    pool: &DbPool,
    profile_id: Uuid,
    schedules: &[(u8, &str, &str)],
) -> Result<()> {
    sqlx::query("DELETE FROM schedules WHERE profile_id=$1")
        .bind(profile_id.to_string())
        .execute(pool)
        .await?;
    for (dow, start, end) in schedules {
        sqlx::query(
            "INSERT INTO schedules (id, profile_id, day_of_week, start_time, end_time)
             VALUES ($1,$2,$3,$4,$5)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(profile_id.to_string())
        .bind(*dow as i32)
        .bind(*start)
        .bind(*end)
        .execute(pool)
        .await?;
    }
    Ok(())
}

// ── daily_limits ──────────────────────────────────────────────────────────────

pub async fn get_daily_limits(pool: &DbPool, profile_id: Uuid) -> Result<Vec<DailyLimit>> {
    let rows = sqlx::query(
        "SELECT profile_id, day_of_week, allowed_minutes
         FROM daily_limits WHERE profile_id=$1 ORDER BY day_of_week",
    )
    .bind(profile_id.to_string())
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| DailyLimit {
            profile_id: r.get::<String, _>("profile_id").parse().unwrap_or_default(),
            day_of_week: r.get::<i64, _>("day_of_week") as u8,
            allowed_minutes: r.get("allowed_minutes"),
        })
        .collect())
}

pub async fn replace_daily_limits(
    pool: &DbPool,
    profile_id: Uuid,
    limits: &[(u8, i32)],
) -> Result<()> {
    sqlx::query("DELETE FROM daily_limits WHERE profile_id=$1")
        .bind(profile_id.to_string())
        .execute(pool)
        .await?;
    for (dow, minutes) in limits {
        sqlx::query(
            "INSERT INTO daily_limits (profile_id, day_of_week, allowed_minutes) VALUES ($1,$2,$3)",
        )
        .bind(profile_id.to_string())
        .bind(*dow as i32)
        .bind(*minutes)
        .execute(pool)
        .await?;
    }
    Ok(())
}

// ── time_adjustments ──────────────────────────────────────────────────────────

pub async fn get_adjustments(
    pool: &DbPool,
    profile_id: Uuid,
    from: Option<&str>,
    to: Option<&str>,
) -> Result<Vec<TimeAdjustment>> {
    let from_val = from.unwrap_or("0000-00-00");
    let to_val = to.unwrap_or("9999-12-31");
    let rows = sqlx::query(
        "SELECT id,profile_id,target_date,adjustment_minutes,reason,created_at
         FROM time_adjustments
         WHERE profile_id=$1 AND target_date>=$2 AND target_date<=$3
         ORDER BY target_date DESC",
    )
    .bind(profile_id.to_string())
    .bind(from_val)
    .bind(to_val)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| TimeAdjustment {
            id: r.get::<String, _>("id").parse().unwrap_or_default(),
            profile_id: r.get::<String, _>("profile_id").parse().unwrap_or_default(),
            target_date: r.get("target_date"),
            adjustment_minutes: r.get("adjustment_minutes"),
            reason: r.get("reason"),
            created_at: r.get("created_at"),
        })
        .collect())
}

pub async fn latest_adjustment_reason_for_date(
    pool: &DbPool,
    profile_id: Uuid,
    date: &str,
) -> Result<Option<String>> {
    let reason: Option<Option<String>> = sqlx::query_scalar(
        "SELECT reason FROM time_adjustments
         WHERE profile_id=$1 AND target_date=$2
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(profile_id.to_string())
    .bind(date)
    .fetch_optional(pool)
    .await?;
    Ok(reason.flatten())
}

pub async fn sum_adjustments_for_date(
    pool: &DbPool,
    profile_id: Uuid,
    date: &str,
) -> Result<i32> {
    let sum: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(adjustment_minutes),0)
         FROM time_adjustments WHERE profile_id=$1 AND target_date=$2",
    )
    .bind(profile_id.to_string())
    .bind(date)
    .fetch_one(pool)
    .await?;
    Ok(sum as i32)
}

pub async fn create_adjustment(
    pool: &DbPool,
    profile_id: Uuid,
    target_date: &str,
    minutes: i32,
    reason: Option<&str>,
    created_by: Option<Uuid>,
) -> Result<Uuid> {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO time_adjustments
         (id,profile_id,target_date,adjustment_minutes,reason,created_by,created_at)
         VALUES ($1,$2,$3,$4,$5,$6,$7)",
    )
    .bind(id.to_string())
    .bind(profile_id.to_string())
    .bind(target_date)
    .bind(minutes)
    .bind(reason)
    .bind(created_by.map(|u| u.to_string()))
    .bind(Utc::now().timestamp())
    .execute(pool)
    .await?;
    Ok(id)
}

// ── enforcement_settings ──────────────────────────────────────────────────────

pub async fn get_enforcement_settings(
    pool: &DbPool,
    profile_id: Uuid,
) -> Result<EnforcementSettings> {
    let row = sqlx::query(
        "SELECT lockout_grace_minutes, warning_thresholds, preserve_tasks_on_lock
         FROM enforcement_settings WHERE profile_id=$1",
    )
    .bind(profile_id.to_string())
    .fetch_optional(pool)
    .await?;

    let (grace, thresholds_str, preserve_i32) = match row {
        Some(r) => (
            r.get::<i32, _>("lockout_grace_minutes"),
            r.get::<String, _>("warning_thresholds"),
            r.get::<i32, _>("preserve_tasks_on_lock"),
        ),
        None => (5, "15,5,1".to_string(), 0),
    };

    let thresholds = thresholds_str
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();

    Ok(EnforcementSettings {
        lockout_grace_minutes: grace,
        preserve_tasks_on_lock: preserve_i32 != 0,
        warning_thresholds: thresholds,
    })
}

pub async fn set_preserve_tasks_on_lock(
    pool: &DbPool,
    profile_id: Uuid,
    preserve: bool,
) -> Result<()> {
    sqlx::query(
        "UPDATE enforcement_settings SET preserve_tasks_on_lock=$1 WHERE profile_id=$2",
    )
    .bind(preserve as i32)
    .bind(profile_id.to_string())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn set_lockout_grace_minutes(
    pool: &DbPool,
    profile_id: Uuid,
    minutes: u32,
) -> Result<()> {
    sqlx::query(
        "UPDATE enforcement_settings SET lockout_grace_minutes=$1 WHERE profile_id=$2",
    )
    .bind(minutes as i32)
    .bind(profile_id.to_string())
    .execute(pool)
    .await?;
    Ok(())
}

// ── daily_usage ───────────────────────────────────────────────────────────────

pub async fn add_usage_seconds(
    pool: &DbPool,
    agent_user_id: Uuid,
    date: &str,
    seconds: i64,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO daily_usage (agent_user_id, date, used_seconds, reported_at)
         VALUES ($1,$2,$3,$4)
         ON CONFLICT(agent_user_id, date)
         DO UPDATE SET used_seconds=daily_usage.used_seconds+EXCLUDED.used_seconds,
                       reported_at=EXCLUDED.reported_at",
    )
    .bind(agent_user_id.to_string())
    .bind(date)
    .bind(seconds)
    .bind(Utc::now().timestamp())
    .execute(pool)
    .await?;
    Ok(())
}

/// Reconciles a usage sync (MSG_USAGE_SYNC): the agent's own cumulative total for
/// the day, sent as a snapshot rather than a delta. Unlike `add_usage_seconds`
/// (used for genuine small per-heartbeat deltas, which must keep adding), this
/// only ever advances the stored total up to the largest value it's seen — so a
/// resend of the same (or an older, out-of-order) total is a safe no-op instead of
/// silently double-counting the whole day. See #13.
///
/// The max is computed in the SQL itself (a portable CASE, not MAX()/GREATEST(),
/// which differ between SQLite and Postgres) so the whole upsert stays atomic.
pub async fn reconcile_usage_seconds(
    pool: &DbPool,
    agent_user_id: Uuid,
    date: &str,
    seconds: i64,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO daily_usage (agent_user_id, date, used_seconds, reported_at)
         VALUES ($1,$2,$3,$4)
         ON CONFLICT(agent_user_id, date)
         DO UPDATE SET used_seconds=CASE
                           WHEN EXCLUDED.used_seconds > daily_usage.used_seconds
                           THEN EXCLUDED.used_seconds
                           ELSE daily_usage.used_seconds
                       END,
                       reported_at=EXCLUDED.reported_at",
    )
    .bind(agent_user_id.to_string())
    .bind(date)
    .bind(seconds)
    .bind(Utc::now().timestamp())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_used_seconds_for_profile_today(
    pool: &DbPool,
    profile_id: Uuid,
    date: &str,
) -> Result<i64> {
    let sum: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(du.used_seconds),0)
         FROM daily_usage du
         JOIN agent_users au ON au.id=du.agent_user_id
         WHERE au.profile_id=$1 AND du.date=$2",
    )
    .bind(profile_id.to_string())
    .bind(date)
    .fetch_one(pool)
    .await?;
    Ok(sum)
}

pub async fn get_usage_by_agent_for_profile(
    pool: &DbPool,
    profile_id: Uuid,
    from: &str,
    to: &str,
) -> Result<Vec<(Uuid, String, String, i64)>> {
    let rows = sqlx::query(
        "SELECT au.agent_id, au.id, du.date, du.used_seconds
         FROM daily_usage du
         JOIN agent_users au ON au.id=du.agent_user_id
         WHERE au.profile_id=$1 AND du.date>=$2 AND du.date<=$3
         ORDER BY du.date DESC",
    )
    .bind(profile_id.to_string())
    .bind(from)
    .bind(to)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            (
                r.get::<String, _>("agent_id").parse().unwrap_or_default(),
                r.get::<String, _>("id"),
                r.get::<String, _>("date"),
                r.get::<i64, _>("used_seconds"),
            )
        })
        .collect())
}

pub async fn get_daily_usage_for_profile(
    pool: &DbPool,
    profile_id: Uuid,
    from: &str,
    to: &str,
) -> Result<Vec<(String, i64)>> {
    let rows = sqlx::query(
        "SELECT du.date, COALESCE(SUM(du.used_seconds),0) as total
         FROM daily_usage du
         JOIN agent_users au ON au.id=du.agent_user_id
         WHERE au.profile_id=$1 AND du.date>=$2 AND du.date<=$3
         GROUP BY du.date ORDER BY du.date DESC",
    )
    .bind(profile_id.to_string())
    .bind(from)
    .bind(to)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| (r.get::<String, _>("date"), r.get::<i64, _>("total")))
        .collect())
}

// ── blocked_domains ───────────────────────────────────────────────────────────

pub async fn get_blocked_domains(pool: &DbPool, profile_id: Uuid) -> Result<Vec<BlockedDomain>> {
    let rows = sqlx::query(
        "SELECT id, profile_id, domain, enabled, created_at
         FROM blocked_domains WHERE profile_id=$1 ORDER BY domain",
    )
    .bind(profile_id.to_string())
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| BlockedDomain {
            id: r.get::<String, _>("id").parse().unwrap_or_default(),
            profile_id: r.get::<String, _>("profile_id").parse().unwrap_or_default(),
            domain: r.get("domain"),
            enabled: r.get::<i32, _>("enabled") != 0,
            created_at: r.get("created_at"),
        })
        .collect())
}

pub async fn get_enabled_blocked_domains(
    pool: &DbPool,
    profile_id: Uuid,
) -> Result<Vec<String>> {
    let rows = sqlx::query(
        "SELECT domain FROM blocked_domains WHERE profile_id=$1 AND enabled=1 ORDER BY domain",
    )
    .bind(profile_id.to_string())
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|r| r.get::<String, _>("domain")).collect())
}

pub async fn set_blocked_domains(
    pool: &DbPool,
    profile_id: Uuid,
    domains: &[(String, bool)],
) -> Result<()> {
    let now = Utc::now().timestamp();
    sqlx::query("DELETE FROM blocked_domains WHERE profile_id=$1")
        .bind(profile_id.to_string())
        .execute(pool)
        .await?;
    for (domain, enabled) in domains {
        sqlx::query(
            "INSERT INTO blocked_domains (id, profile_id, domain, enabled, created_at)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(profile_id.to_string())
        .bind(domain.as_str())
        .bind(*enabled as i32)
        .bind(now)
        .execute(pool)
        .await?;
    }
    Ok(())
}

pub async fn patch_blocked_domain(pool: &DbPool, id: Uuid, enabled: bool) -> Result<bool> {
    let result = sqlx::query("UPDATE blocked_domains SET enabled=$1 WHERE id=$2")
        .bind(enabled as i32)
        .bind(id.to_string())
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn add_blocked_domain(
    pool: &DbPool,
    profile_id: Uuid,
    domain: &str,
    enabled: bool,
) -> Result<BlockedDomain> {
    let id = Uuid::new_v4();
    let now = Utc::now().timestamp();
    sqlx::query(
        "INSERT INTO blocked_domains (id, profile_id, domain, enabled, created_at)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(id.to_string())
    .bind(profile_id.to_string())
    .bind(domain)
    .bind(enabled as i32)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(BlockedDomain {
        id,
        profile_id,
        domain: domain.to_string(),
        enabled,
        created_at: now,
    })
}

pub async fn delete_blocked_domain(pool: &DbPool, id: Uuid) -> Result<bool> {
    let result = sqlx::query("DELETE FROM blocked_domains WHERE id=$1")
        .bind(id.to_string())
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

// ── config_versions ───────────────────────────────────────────────────────────

pub async fn get_config_version(pool: &DbPool, profile_id: Uuid) -> Result<i64> {
    let v: Option<i64> = sqlx::query_scalar(
        "SELECT version FROM config_versions WHERE profile_id=$1",
    )
    .bind(profile_id.to_string())
    .fetch_optional(pool)
    .await?;
    Ok(v.unwrap_or(1))
}

pub async fn bump_config_version(pool: &DbPool, profile_id: Uuid) -> Result<i64> {
    let now = Utc::now().timestamp();
    sqlx::query(
        "INSERT INTO config_versions (profile_id, version, updated_at) VALUES ($1, 1, $2)
         ON CONFLICT(profile_id) DO UPDATE SET version=config_versions.version+1, updated_at=EXCLUDED.updated_at",
    )
    .bind(profile_id.to_string())
    .bind(now)
    .execute(pool)
    .await?;
    let v: i64 =
        sqlx::query_scalar("SELECT version FROM config_versions WHERE profile_id=$1")
            .bind(profile_id.to_string())
            .fetch_one(pool)
            .await?;
    Ok(v)
}

// ── audit_log ─────────────────────────────────────────────────────────────────

#[allow(dead_code)]
pub async fn audit(
    pool: &DbPool,
    admin_id: Option<Uuid>,
    action: &str,
    target_type: Option<&str>,
    target_id: Option<Uuid>,
    detail: Option<&str>,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO audit_log (id,admin_user_id,action,target_type,target_id,detail,created_at)
         VALUES ($1,$2,$3,$4,$5,$6,$7)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(admin_id.map(|u| u.to_string()))
    .bind(action)
    .bind(target_type)
    .bind(target_id.map(|u| u.to_string()))
    .bind(detail)
    .bind(Utc::now().timestamp())
    .execute(pool)
    .await?;
    Ok(())
}

// ── helpers ───────────────────────────────────────────────────────────────────

pub async fn pending_agent_count(pool: &DbPool) -> Result<i64> {
    Ok(
        sqlx::query_scalar("SELECT COUNT(*) FROM agents WHERE status='pending'")
            .fetch_one(pool)
            .await?,
    )
}

pub fn weekday_for_date(date: &str) -> u8 {
    use chrono::Datelike;
    NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map(|d| d.weekday().num_days_from_monday() as u8)
        .unwrap_or(0)
}

pub fn parse_time(s: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(s, "%H:%M").ok()
}

/// Fresh, fully migrated in-memory SQLite database for tests.
#[cfg(test)]
pub(crate) async fn open_in_memory() -> DbPool {
    sqlx::any::install_default_drivers();
    let pool = sqlx::pool::PoolOptions::<sqlx::Any>::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys=ON").execute(&pool).await.unwrap();
    create_tables(&pool).await.unwrap();
    run_migrations(&pool, true).await.unwrap();
    pool
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use common::models::LocalUser;

    async fn test_pool() -> DbPool {
        open_in_memory().await
    }

    async fn test_pool_before_v5() -> DbPool {
        sqlx::any::install_default_drivers();
        let pool = sqlx::pool::PoolOptions::<sqlx::Any>::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();

        sqlx::query("CREATE TABLE admin_users (id TEXT NOT NULL PRIMARY KEY, username TEXT NOT NULL UNIQUE, password_hash TEXT NOT NULL, timezone TEXT NOT NULL DEFAULT 'UTC', created_at INTEGER NOT NULL)").execute(&pool).await.unwrap();
        sqlx::query("CREATE TABLE agents (id TEXT NOT NULL PRIMARY KEY, machine_id TEXT NOT NULL UNIQUE, display_name TEXT NOT NULL, hostname TEXT NOT NULL, timezone TEXT NOT NULL DEFAULT 'UTC', status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','paired','disabled','pending_delete')), auth_token_hash TEXT, agent_version TEXT, paired_at INTEGER, last_seen_at INTEGER, created_at INTEGER NOT NULL)").execute(&pool).await.unwrap();
        sqlx::query("CREATE TABLE user_profiles (id TEXT NOT NULL PRIMARY KEY, display_name TEXT NOT NULL, language TEXT NOT NULL DEFAULT 'en', created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL)").execute(&pool).await.unwrap();
        sqlx::query("CREATE TABLE agent_users (id TEXT NOT NULL PRIMARY KEY, agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE, profile_id TEXT REFERENCES user_profiles(id) ON DELETE SET NULL, local_uid INTEGER NOT NULL, local_username TEXT NOT NULL, display_name TEXT, status TEXT NOT NULL DEFAULT 'unmanaged' CHECK (status IN ('unmanaged','managed','deleted')), first_seen_at INTEGER NOT NULL, last_reported_at INTEGER NOT NULL, UNIQUE(agent_id, local_uid))").execute(&pool).await.unwrap();
        sqlx::query("CREATE TABLE schedules (id TEXT NOT NULL PRIMARY KEY, profile_id TEXT NOT NULL REFERENCES user_profiles(id) ON DELETE CASCADE, day_of_week INTEGER NOT NULL CHECK (day_of_week BETWEEN 0 AND 6), start_time TEXT NOT NULL, end_time TEXT NOT NULL)").execute(&pool).await.unwrap();
        sqlx::query("CREATE TABLE daily_limits (profile_id TEXT NOT NULL REFERENCES user_profiles(id) ON DELETE CASCADE, day_of_week INTEGER NOT NULL CHECK (day_of_week BETWEEN 0 AND 6), allowed_minutes INTEGER NOT NULL CHECK (allowed_minutes >= 0), PRIMARY KEY (profile_id, day_of_week))").execute(&pool).await.unwrap();
        sqlx::query("CREATE TABLE time_adjustments (id TEXT NOT NULL PRIMARY KEY, profile_id TEXT NOT NULL REFERENCES user_profiles(id) ON DELETE CASCADE, target_date TEXT NOT NULL, adjustment_minutes INTEGER NOT NULL, reason TEXT, created_by TEXT REFERENCES admin_users(id), created_at INTEGER NOT NULL, synced_to_agents INTEGER NOT NULL DEFAULT 0)").execute(&pool).await.unwrap();
        // enforcement_settings WITHOUT preserve_tasks_on_lock (pre-v5 state)
        sqlx::query("CREATE TABLE enforcement_settings (profile_id TEXT NOT NULL PRIMARY KEY REFERENCES user_profiles(id) ON DELETE CASCADE, lockout_grace_minutes INTEGER NOT NULL DEFAULT 5, warning_thresholds TEXT NOT NULL DEFAULT '15,5,1')").execute(&pool).await.unwrap();
        sqlx::query("CREATE TABLE daily_usage (agent_user_id TEXT NOT NULL REFERENCES agent_users(id) ON DELETE CASCADE, date TEXT NOT NULL, used_seconds INTEGER NOT NULL DEFAULT 0, reported_at INTEGER NOT NULL, PRIMARY KEY (agent_user_id, date))").execute(&pool).await.unwrap();
        sqlx::query("CREATE TABLE config_versions (profile_id TEXT NOT NULL PRIMARY KEY REFERENCES user_profiles(id) ON DELETE CASCADE, version INTEGER NOT NULL DEFAULT 1, updated_at INTEGER NOT NULL)").execute(&pool).await.unwrap();
        sqlx::query("CREATE TABLE audit_log (id TEXT NOT NULL PRIMARY KEY, admin_user_id TEXT REFERENCES admin_users(id), action TEXT NOT NULL, target_type TEXT, target_id TEXT, detail TEXT, created_at INTEGER NOT NULL)").execute(&pool).await.unwrap();

        // Schema version at 4 (v5 not applied yet)
        sqlx::query("CREATE TABLE _schema_version (version INTEGER NOT NULL DEFAULT 0)").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO _schema_version (version) VALUES (4)").execute(&pool).await.unwrap();

        pool
    }

    // Regression test for #15: a brand-new homelab install (fresh Docker volume,
    // fresh `install.sh` run) points db_path at a file that doesn't exist yet.
    // `open()` must create it — sqlx's SqliteConnectOptions defaults
    // create_if_missing to false, so without `?mode=rwc` on the URL this used
    // to fail with "(code: 14) unable to open database file" on first boot,
    // while upgrades (where server.db already existed) never showed the bug.
    #[tokio::test]
    async fn open_creates_missing_sqlite_file_on_fresh_install() {
        let dir = std::env::temp_dir().join(format!("screenguard-test-{}", Uuid::new_v4()));
        let db_path = dir.join("server.db").to_string_lossy().to_string();
        assert!(!std::path::Path::new(&db_path).exists());

        let cfg = crate::config::ServerConfig {
            db_path,
            ..Default::default()
        };

        let pool = open(&cfg).await.expect("open() must create a missing sqlite db file");
        // Sanity check the pool is actually usable, not just "connected".
        create_profile(&pool, "Fresh install smoke test").await.unwrap();
        pool.close().await;

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn new_profiles_default_preserve_tasks_to_false() {
        let pool = test_pool().await;
        let profile = create_profile(&pool, "Test profile").await.unwrap();
        assert!(!get_enforcement_settings(&pool, profile.id).await.unwrap().preserve_tasks_on_lock);
    }

    #[tokio::test]
    async fn migration_defaults_existing_profiles_to_false() {
        let pool = test_pool_before_v5().await;
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO user_profiles (id, display_name, created_at, updated_at, language)
             VALUES ($1, 'Existing profile', 1, 1, 'en')",
        )
        .bind(id.to_string())
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO enforcement_settings (profile_id) VALUES ($1)")
            .bind(id.to_string())
            .execute(&pool)
            .await
            .unwrap();
        apply_v5(&pool).await.unwrap();

        assert!(!get_enforcement_settings(&pool, id).await.unwrap().preserve_tasks_on_lock);
    }

    #[tokio::test]
    async fn stores_and_retrieves_preserve_tasks_setting() {
        let pool = test_pool().await;
        let profile = create_profile(&pool, "Test profile").await.unwrap();

        set_preserve_tasks_on_lock(&pool, profile.id, true).await.unwrap();
        assert!(get_enforcement_settings(&pool, profile.id).await.unwrap().preserve_tasks_on_lock);

        set_preserve_tasks_on_lock(&pool, profile.id, false).await.unwrap();
        assert!(!get_enforcement_settings(&pool, profile.id).await.unwrap().preserve_tasks_on_lock);
    }

    #[tokio::test]
    async fn config_propagation_includes_preserve_tasks_setting() {
        let pool = test_pool().await;
        let profile = create_profile(&pool, "Test profile").await.unwrap();
        set_preserve_tasks_on_lock(&pool, profile.id, true).await.unwrap();

        let agent_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO agents
             (id, machine_id, display_name, hostname, timezone, status, agent_version, created_at)
             VALUES ($1, 'machine', 'host', 'host', 'UTC', 'paired', 'test', 1)",
        )
        .bind(agent_id.to_string())
        .execute(&pool)
        .await
        .unwrap();

        upsert_agent_users(
            &pool,
            agent_id,
            &[LocalUser {
                local_uid: 1000,
                username: "test".to_string(),
                display_name: "Test User".to_string(),
            }],
        )
        .await
        .unwrap();
        let agent_user = get_agent_user(&pool, agent_id, 1000).await.unwrap().unwrap();
        update_agent_user(&pool, agent_user.id, Some(profile.id), Some("managed"))
            .await
            .unwrap();

        let config = crate::remaining::build_config_push(&pool, agent_id, 2).await.unwrap();
        assert_eq!(config.users.len(), 1);
        assert!(config.users[0].preserve_tasks_on_lock);
    }

    #[tokio::test]
    async fn new_profile_seeds_default_blocked_domains() {
        let pool = test_pool().await;
        let profile = create_profile(&pool, "Test").await.unwrap();
        let domains = get_blocked_domains(&pool, profile.id).await.unwrap();
        assert_eq!(domains.len(), DEFAULT_BLOCKED_DOMAINS.len());
        assert!(domains.iter().any(|d| d.domain == "youtube.com"));
        assert!(domains.iter().all(|d| !d.enabled));
    }

    #[tokio::test]
    async fn set_and_get_blocked_domains() {
        let pool = test_pool().await;
        let profile = create_profile(&pool, "Test").await.unwrap();
        set_blocked_domains(
            &pool,
            profile.id,
            &[
                ("youtube.com".to_string(), true),
                ("tiktok.com".to_string(), false),
            ],
        )
        .await
        .unwrap();
        let domains = get_blocked_domains(&pool, profile.id).await.unwrap();
        assert_eq!(domains.len(), 2);
        assert!(domains.iter().find(|d| d.domain == "youtube.com").unwrap().enabled);
        assert!(!domains.iter().find(|d| d.domain == "tiktok.com").unwrap().enabled);
    }

    #[tokio::test]
    async fn get_enabled_blocked_domains_returns_only_enabled() {
        let pool = test_pool().await;
        let profile = create_profile(&pool, "Test").await.unwrap();
        set_blocked_domains(
            &pool,
            profile.id,
            &[
                ("youtube.com".to_string(), true),
                ("tiktok.com".to_string(), false),
                ("discord.com".to_string(), true),
            ],
        )
        .await
        .unwrap();
        let enabled = get_enabled_blocked_domains(&pool, profile.id).await.unwrap();
        assert_eq!(enabled.len(), 2);
        assert!(enabled.contains(&"youtube.com".to_string()));
        assert!(enabled.contains(&"discord.com".to_string()));
        assert!(!enabled.contains(&"tiktok.com".to_string()));
    }

    #[tokio::test]
    async fn patch_blocked_domain_toggles_enabled() {
        let pool = test_pool().await;
        let profile = create_profile(&pool, "Test").await.unwrap();
        let domain = add_blocked_domain(&pool, profile.id, "example.com", false).await.unwrap();
        assert!(!domain.enabled);
        patch_blocked_domain(&pool, domain.id, true).await.unwrap();
        let domains = get_blocked_domains(&pool, profile.id).await.unwrap();
        let d = domains.iter().find(|d| d.domain == "example.com").unwrap();
        assert!(d.enabled);
    }

    #[tokio::test]
    async fn delete_blocked_domain_removes_it() {
        let pool = test_pool().await;
        let profile = create_profile(&pool, "Test").await.unwrap();
        let domain = add_blocked_domain(&pool, profile.id, "example.com", true).await.unwrap();
        let before = get_blocked_domains(&pool, profile.id).await.unwrap().len();
        delete_blocked_domain(&pool, domain.id).await.unwrap();
        let after = get_blocked_domains(&pool, profile.id).await.unwrap().len();
        assert_eq!(after, before - 1);
    }

    #[tokio::test]
    async fn config_push_includes_enabled_blocked_domains() {
        let pool = test_pool().await;
        let profile = create_profile(&pool, "Test").await.unwrap();
        set_blocked_domains(
            &pool,
            profile.id,
            &[
                ("youtube.com".to_string(), true),
                ("tiktok.com".to_string(), false),
            ],
        )
        .await
        .unwrap();
        let agent_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO agents
             (id, machine_id, display_name, hostname, timezone, status, agent_version, created_at)
             VALUES ($1, 'machine2', 'host', 'host', 'UTC', 'paired', 'test', 1)",
        )
        .bind(agent_id.to_string())
        .execute(&pool)
        .await
        .unwrap();
        upsert_agent_users(
            &pool,
            agent_id,
            &[LocalUser {
                local_uid: 1000,
                username: "test".to_string(),
                display_name: "Test User".to_string(),
            }],
        )
        .await
        .unwrap();
        let agent_user = get_agent_user(&pool, agent_id, 1000).await.unwrap().unwrap();
        update_agent_user(&pool, agent_user.id, Some(profile.id), Some("managed"))
            .await
            .unwrap();

        let config = crate::remaining::build_config_push(&pool, agent_id, 1).await.unwrap();
        assert_eq!(config.users.len(), 1);
        assert_eq!(config.users[0].blocked_domains, vec!["youtube.com"]);
    }

    async fn test_agent_user(pool: &DbPool) -> Uuid {
        let agent_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO agents
             (id, machine_id, display_name, hostname, timezone, status, agent_version, created_at)
             VALUES ($1, $2, 'host', 'host', 'UTC', 'paired', 'test', 1)",
        )
        .bind(agent_id.to_string())
        .bind(agent_id.to_string())
        .execute(pool)
        .await
        .unwrap();
        upsert_agent_users(
            pool,
            agent_id,
            &[LocalUser {
                local_uid: 1000,
                username: "test".to_string(),
                display_name: "Test User".to_string(),
            }],
        )
        .await
        .unwrap();
        get_agent_user(pool, agent_id, 1000).await.unwrap().unwrap().id
    }

    // Regression test for #13: MSG_USAGE_SYNC carries the agent's whole locally-
    // accumulated total for the day (a snapshot), not a delta, so it must never go
    // through the plain-additive path that live per-heartbeat deltas use — that
    // silently double-counts the entire day's usage on every reconnect-triggered
    // resync. reconcile_usage_seconds must only ever advance the stored total up
    // to the largest total it's been told about, never add on top of it.
    #[tokio::test]
    async fn reconcile_usage_seconds_does_not_double_count_a_resent_total() {
        let pool = test_pool().await;
        let agent_user_id = test_agent_user(&pool).await;

        // Live heartbeats already brought the server to 3000s for today.
        add_usage_seconds(&pool, agent_user_id, "2026-09-14", 3000).await.unwrap();

        // The agent reconnects and resyncs its own cumulative total for today,
        // which is the same 3000s the server already has — must not double it.
        reconcile_usage_seconds(&pool, agent_user_id, "2026-09-14", 3000).await.unwrap();
        assert_eq!(
            get_used_seconds_for_profile_today_by_agent_user(&pool, agent_user_id, "2026-09-14").await,
            3000
        );

        // Genuine new live usage still adds normally (untouched path).
        add_usage_seconds(&pool, agent_user_id, "2026-09-14", 10).await.unwrap();
        assert_eq!(
            get_used_seconds_for_profile_today_by_agent_user(&pool, agent_user_id, "2026-09-14").await,
            3010
        );

        // A sync that reflects genuinely more usage (e.g. accrued while offline)
        // correctly advances the total.
        reconcile_usage_seconds(&pool, agent_user_id, "2026-09-14", 3400).await.unwrap();
        assert_eq!(
            get_used_seconds_for_profile_today_by_agent_user(&pool, agent_user_id, "2026-09-14").await,
            3400
        );

        // A stale/out-of-order resync with a smaller total must not regress it.
        reconcile_usage_seconds(&pool, agent_user_id, "2026-09-14", 3000).await.unwrap();
        assert_eq!(
            get_used_seconds_for_profile_today_by_agent_user(&pool, agent_user_id, "2026-09-14").await,
            3400
        );
    }

    #[tokio::test]
    async fn reconcile_usage_seconds_on_first_sync_of_the_day_inserts_the_row() {
        let pool = test_pool().await;
        let agent_user_id = test_agent_user(&pool).await;

        reconcile_usage_seconds(&pool, agent_user_id, "2026-09-14", 90).await.unwrap();
        assert_eq!(
            get_used_seconds_for_profile_today_by_agent_user(&pool, agent_user_id, "2026-09-14").await,
            90
        );
    }

    async fn get_used_seconds_for_profile_today_by_agent_user(
        pool: &DbPool,
        agent_user_id: Uuid,
        date: &str,
    ) -> i64 {
        sqlx::query_scalar(
            "SELECT used_seconds FROM daily_usage WHERE agent_user_id=$1 AND date=$2",
        )
        .bind(agent_user_id.to_string())
        .bind(date)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    // ── multi-admin ─────────────────────────────────────────────────────────

    async fn test_pool_before_v8() -> DbPool {
        sqlx::any::install_default_drivers();
        let pool = sqlx::pool::PoolOptions::<sqlx::Any>::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        // admin_users WITHOUT must_change_password (pre-v8 shape).
        sqlx::query("CREATE TABLE admin_users (id TEXT NOT NULL PRIMARY KEY, username TEXT NOT NULL UNIQUE, password_hash TEXT NOT NULL, timezone TEXT NOT NULL DEFAULT 'UTC', created_at INTEGER NOT NULL)").execute(&pool).await.unwrap();
        pool
    }

    async fn insert_legacy_admin(pool: &DbPool, username: &str, timezone: &str, created_at: i64) {
        sqlx::query(
            "INSERT INTO admin_users (id, username, password_hash, timezone, created_at) VALUES ($1,$2,$3,$4,$5)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(username)
        .bind("hash")
        .bind(timezone)
        .bind(created_at)
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn create_admin_stores_must_change_password() {
        let pool = test_pool().await;
        create_admin(&pool, "first", "hash", false).await.unwrap();
        let helper_id = create_admin(&pool, "helper", "hash", true).await.unwrap();

        let first = get_admin_by_username(&pool, "first").await.unwrap().unwrap();
        let helper = get_admin_user_by_id(&pool, helper_id).await.unwrap().unwrap();
        assert!(!first.must_change_password);
        assert!(helper.must_change_password);
    }

    #[tokio::test]
    async fn migration_v8_adds_flag_and_syncs_timezone_to_original_admin() {
        let pool = test_pool_before_v8().await;
        insert_legacy_admin(&pool, "original", "Europe/Warsaw", 1).await;
        insert_legacy_admin(&pool, "later", "UTC", 2).await;

        apply_v8(&pool).await.unwrap();

        for name in ["original", "later"] {
            let a = get_admin_by_username(&pool, name).await.unwrap().unwrap();
            assert_eq!(a.timezone, "Europe/Warsaw");
            assert!(!a.must_change_password);
        }
    }

    #[tokio::test]
    async fn timezone_is_installation_wide() {
        let pool = test_pool().await;
        let first = create_admin(&pool, "first", "hash", false).await.unwrap();
        set_admin_timezone(&pool, "Europe/Berlin").await.unwrap();

        // New admins inherit it...
        let second = create_admin(&pool, "second", "hash", true).await.unwrap();
        assert_eq!(get_admin_user_by_id(&pool, second).await.unwrap().unwrap().timezone, "Europe/Berlin");

        // ...and a change by anyone applies to everyone.
        set_admin_timezone(&pool, "America/New_York").await.unwrap();
        assert_eq!(get_admin_timezone(&pool).await.unwrap(), "America/New_York");
        for id in [first, second] {
            assert_eq!(get_admin_user_by_id(&pool, id).await.unwrap().unwrap().timezone, "America/New_York");
        }
    }

    #[tokio::test]
    async fn list_admins_returns_all_admins_ordered_by_creation() {
        let pool = test_pool().await;
        create_admin(&pool, "first", "hash", false).await.unwrap();
        create_admin(&pool, "second", "hash", true).await.unwrap();

        let admins = list_admins(&pool).await.unwrap();
        assert_eq!(admins.len(), 2);
        assert_eq!(admins[0].username, "first");
        assert_eq!(admins[1].username, "second");
    }

    #[tokio::test]
    async fn admin_count_reflects_current_total() {
        let pool = test_pool().await;
        assert_eq!(admin_count(&pool).await.unwrap(), 0);
        create_admin(&pool, "first", "hash", false).await.unwrap();
        assert_eq!(admin_count(&pool).await.unwrap(), 1);
    }

    // ── agent token verification ──────────────────────────────────────────────

    async fn paired_agent(pool: &DbPool, token_hash: &str, verified: bool) -> Uuid {
        let agent = upsert_agent_pending(pool, "m1", "kid-pc", "UTC", "").await.unwrap();
        accept_agent(pool, agent.id, token_hash).await.unwrap();
        if !verified {
            sqlx::query("UPDATE agents SET token_verified=0").execute(pool).await.unwrap();
        }
        agent.id
    }

    #[tokio::test]
    async fn verified_agent_token_must_match() {
        let pool = test_pool().await;
        let id = paired_agent(&pool, "good", true).await;
        assert_eq!(verify_agent_token(&pool, id, Some("good")).await.unwrap(), TokenCheck::Valid);
        assert_eq!(verify_agent_token(&pool, id, Some("bad")).await.unwrap(), TokenCheck::Invalid);
    }

    #[tokio::test]
    async fn legacy_pairing_adopts_its_first_token_then_enforces_it() {
        // A pairing from before verification whose stored hash doesn't match the
        // agent's token (e.g. a double "accept" overwrote it): kept, not locked out.
        let pool = test_pool().await;
        let id = paired_agent(&pool, "overwritten", false).await;
        assert_eq!(verify_agent_token(&pool, id, Some("agents-real")).await.unwrap(), TokenCheck::Adopted);
        assert_eq!(verify_agent_token(&pool, id, Some("agents-real")).await.unwrap(), TokenCheck::Valid);
        assert_eq!(verify_agent_token(&pool, id, Some("overwritten")).await.unwrap(), TokenCheck::Invalid);
    }

    #[tokio::test]
    async fn legacy_pairing_with_matching_token_becomes_verified() {
        let pool = test_pool().await;
        let id = paired_agent(&pool, "good", false).await;
        assert_eq!(verify_agent_token(&pool, id, Some("good")).await.unwrap(), TokenCheck::Valid);
        assert_eq!(verify_agent_token(&pool, id, Some("other")).await.unwrap(), TokenCheck::Invalid);
    }

    #[tokio::test]
    async fn missing_token_is_never_adopted() {
        let pool = test_pool().await;
        let id = paired_agent(&pool, "real", false).await;
        // Unverified pairing without a token: let through, nothing stored.
        assert_eq!(verify_agent_token(&pool, id, None).await.unwrap(), TokenCheck::UnverifiedNoToken);
        assert_eq!(verify_agent_token(&pool, id, None).await.unwrap(), TokenCheck::UnverifiedNoToken);
        // The real token still works afterwards.
        assert_eq!(verify_agent_token(&pool, id, Some("real")).await.unwrap(), TokenCheck::Valid);
        // Once verified, no token means rejected.
        assert_eq!(verify_agent_token(&pool, id, None).await.unwrap(), TokenCheck::Invalid);
    }

    #[tokio::test]
    async fn migration_v9_leaves_existing_pairings_unverified() {
        let pool = test_pool().await;
        let id = paired_agent(&pool, "h", true).await;
        sqlx::query("ALTER TABLE agents DROP COLUMN token_verified").execute(&pool).await.unwrap();
        apply_v9(&pool).await.unwrap();
        let a = get_agent_by_id(&pool, id).await.unwrap().unwrap();
        assert_eq!(a.status, "paired");
        assert_eq!(a.auth_token_hash.as_deref(), Some("h"));
        assert_eq!(verify_agent_token(&pool, id, Some("agents-real")).await.unwrap(), TokenCheck::Adopted);
    }

    #[tokio::test]
    async fn migration_v10_adds_app_usage_and_keeps_pairings() {
        let pool = test_pool().await;
        let id = paired_agent(&pool, "h", true).await;
        // back to the v9 shape
        for stmt in ["DROP TABLE app_usage", "DROP TABLE app_catalog", "ALTER TABLE agents DROP COLUMN app_usage_available"] {
            sqlx::query(stmt).execute(&pool).await.unwrap();
        }
        set_schema_version(&pool, 9).await.unwrap();
        run_migrations(&pool, true).await.unwrap();
        sqlx::query("SELECT categories FROM app_catalog").fetch_all(&pool).await.unwrap();

        let a = get_agent_by_id(&pool, id).await.unwrap().unwrap();
        assert_eq!((a.status.as_str(), a.auth_token_hash.as_deref(), a.app_usage_available), ("paired", Some("h"), None));
        assert_eq!(verify_agent_token(&pool, id, Some("h")).await.unwrap(), TokenCheck::Valid);
        update_agent_app_usage(&pool, id, true).await.unwrap();
        assert_eq!(get_agent_by_id(&pool, id).await.unwrap().unwrap().app_usage_available, Some(true));
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM app_usage").fetch_one(&pool).await.unwrap();
        assert_eq!(n, 0);
        assert_eq!(get_schema_version(&pool, true).await.unwrap(), 11);
    }

    #[tokio::test]
    async fn migration_v11_adds_categories_to_a_v10_catalog() {
        let pool = test_pool().await;
        // a v10 catalog: no categories column
        sqlx::query("DROP TABLE app_catalog").execute(&pool).await.unwrap();
        sqlx::query("CREATE TABLE app_catalog (app_id TEXT NOT NULL PRIMARY KEY, display_name TEXT NOT NULL, icon TEXT, first_seen INTEGER NOT NULL, last_seen INTEGER NOT NULL)").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO app_catalog VALUES ('steam:570','Dota 2',NULL,1,1)").execute(&pool).await.unwrap();
        set_schema_version(&pool, 10).await.unwrap();
        run_migrations(&pool, true).await.unwrap();
        let c: String = sqlx::query_scalar("SELECT categories FROM app_catalog").fetch_one(&pool).await.unwrap();
        assert_eq!(c, "");
        // and running it again (column present) is a no-op
        apply_v11(&pool).await.unwrap();
    }

    #[tokio::test]
    async fn delete_admin_removes_the_row_but_never_the_last() {
        let pool = test_pool().await;
        let first = create_admin(&pool, "first", "hash", false).await.unwrap();
        let id = create_admin(&pool, "helper", "hash", true).await.unwrap();
        assert_eq!(delete_admin(&pool, id).await.unwrap(), DeleteAdmin::Deleted);
        assert!(get_admin_user_by_id(&pool, id).await.unwrap().is_none());
        assert_eq!(delete_admin(&pool, id).await.unwrap(), DeleteAdmin::NotFound);
        assert_eq!(delete_admin(&pool, first).await.unwrap(), DeleteAdmin::LastAdmin);
        assert_eq!(admin_count(&pool).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn concurrent_mutual_deletes_leave_one_admin() {
        // File-backed so the pool really has several connections racing.
        let dir = std::env::temp_dir().join(format!("sg-del-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        sqlx::any::install_default_drivers();
        let url = format!("sqlite:{}?mode=rwc", dir.join("t.db").display());
        let pool = sqlx::pool::PoolOptions::<sqlx::Any>::new()
            .max_connections(4)
            .connect(&url)
            .await
            .unwrap();
        sqlx::query("PRAGMA journal_mode=WAL").execute(&pool).await.unwrap();
        create_tables(&pool).await.unwrap();
        run_migrations(&pool, true).await.unwrap();

        for _ in 0..20 {
            let a = create_admin(&pool, &format!("a-{}", Uuid::new_v4()), "h", false).await.unwrap();
            let b = create_admin(&pool, &format!("b-{}", Uuid::new_v4()), "h", false).await.unwrap();
            let (ra, rb) = tokio::join!(delete_admin(&pool, a), delete_admin(&pool, b));
            let outcomes = [ra.unwrap(), rb.unwrap()];
            assert!(outcomes.contains(&DeleteAdmin::Deleted), "{outcomes:?}");
            assert!(outcomes.contains(&DeleteAdmin::LastAdmin), "{outcomes:?}");
            assert_eq!(admin_count(&pool).await.unwrap(), 1);
            // Reset for the next round.
            sqlx::query("DELETE FROM admin_users").execute(&pool).await.unwrap();
        }
        pool.close().await;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn update_admin_password_changes_hash_and_flag() {
        let pool = test_pool().await;
        let id = create_admin(&pool, "helper", "old-hash", true).await.unwrap();

        let new_hash = crate::api::auth::hash_password("new-password").unwrap();
        update_admin_password(&pool, id, &new_hash, false).await.unwrap();

        let admin = get_admin_user_by_id(&pool, id).await.unwrap().unwrap();
        assert!(crate::api::auth::verify_password("new-password", &admin.password_hash).unwrap());
        assert!(!crate::api::auth::verify_password("old-password", &admin.password_hash).unwrap());
        assert!(!admin.must_change_password);

        update_admin_password(&pool, id, &new_hash, true).await.unwrap();
        assert!(get_admin_user_by_id(&pool, id).await.unwrap().unwrap().must_change_password);
    }
}
