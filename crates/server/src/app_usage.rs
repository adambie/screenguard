//! Per-app usage reports (#27, docs/feature-app-usage-limits.md): ingestion of
//! `app_usage_sync` and the report queries.

use anyhow::Result;
use chrono::{NaiveDate, Utc};
use common::apps::AppId;
use common::messages::AppUsageSync;
use std::collections::HashMap;
use uuid::Uuid;

use crate::db::{self, DbPool};

/// No app can run longer than a day in a day.
const MAX_SECONDS_PER_DAY: u64 = 86_400;
const MAX_NAME_CHARS: usize = 200;
const MAX_ICON_CHARS: usize = 512;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Ingested {
    pub stored: usize,
    /// Invalid app id, a date in the future, or an unknown local user.
    pub rejected: usize,
}

/// Stores an agent's day totals. Totals only grow within a day, so each upsert
/// keeps the larger value: resends after a reconnect never double-count.
/// `today` is the server's date; agents may be up to a day ahead (time zones).
pub async fn ingest(pool: &DbPool, agent_id: Uuid, sync: &AppUsageSync, today: NaiveDate) -> Result<Ingested> {
    let mut out = Ingested::default();
    let mut users: HashMap<u32, Option<Uuid>> = HashMap::new();
    let now = Utc::now().timestamp();
    for e in &sync.entries {
        let Some(app) = AppId::parse(&e.app_id).filter(AppId::is_app) else {
            out.rejected += 1;
            continue;
        };
        if e.date > today.succ_opt().unwrap_or(today) {
            out.rejected += 1;
            continue;
        }
        let agent_user = match users.get(&e.local_uid) {
            Some(u) => *u,
            None => {
                let u = db::get_agent_user(pool, agent_id, e.local_uid).await?.map(|u| u.id);
                users.insert(e.local_uid, u);
                u
            }
        };
        let Some(agent_user) = agent_user else {
            out.rejected += 1;
            continue;
        };
        let name = clean(&e.display_name, MAX_NAME_CHARS).unwrap_or_else(|| app.name.clone());
        let icon = e.icon.as_deref().and_then(|i| clean(i, MAX_ICON_CHARS));
        let seconds = e.seconds_total.min(MAX_SECONDS_PER_DAY) as i64;

        sqlx::query(
            "INSERT INTO app_usage (agent_user_id, date, app_id, seconds) VALUES ($1,$2,$3,$4)
             ON CONFLICT(agent_user_id, date, app_id)
             DO UPDATE SET seconds=CASE
                 WHEN EXCLUDED.seconds > app_usage.seconds THEN EXCLUDED.seconds
                 ELSE app_usage.seconds
             END",
        )
        .bind(agent_user.to_string())
        .bind(e.date.to_string())
        .bind(app.to_string())
        .bind(seconds)
        .execute(pool)
        .await?;
        sqlx::query(
            "INSERT INTO app_catalog (app_id, display_name, icon, first_seen, last_seen) VALUES ($1,$2,$3,$4,$4)
             ON CONFLICT(app_id)
             DO UPDATE SET display_name=EXCLUDED.display_name, icon=EXCLUDED.icon, last_seen=EXCLUDED.last_seen",
        )
        .bind(app.to_string())
        .bind(name)
        .bind(icon)
        .bind(now)
        .execute(pool)
        .await?;
        out.stored += 1;
    }
    Ok(out)
}

/// Trimmed, without control characters, at most `max` chars; `None` if empty.
fn clean(s: &str, max: usize) -> Option<String> {
    let s: String = s.chars().filter(|c| !c.is_control()).take(max).collect();
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::messages::AppUsageEntry;
    use common::models::LocalUser;

    pub(crate) async fn agent_with_user(pool: &DbPool, machine: &str, uid: u32) -> (Uuid, Uuid) {
        let agent = db::upsert_agent_pending(pool, machine, machine, "UTC", "").await.unwrap();
        db::accept_agent(pool, agent.id, "hash").await.unwrap();
        let user = LocalUser { local_uid: uid, username: "kid".into(), display_name: "Kid".into() };
        db::upsert_agent_users(pool, agent.id, &[user]).await.unwrap();
        let au = db::get_agent_user(pool, agent.id, uid).await.unwrap().unwrap();
        (agent.id, au.id)
    }

    pub(crate) fn entry(uid: u32, date: &str, app: &str, name: &str, secs: u64) -> AppUsageEntry {
        AppUsageEntry {
            local_uid: uid,
            date: date.parse().unwrap(),
            app_id: app.into(),
            display_name: name.into(),
            icon: Some("icon".into()),
            seconds_total: secs,
        }
    }

    async fn seconds(pool: &DbPool, au: Uuid, date: &str, app: &str) -> Option<i64> {
        sqlx::query_scalar("SELECT seconds FROM app_usage WHERE agent_user_id=$1 AND date=$2 AND app_id=$3")
            .bind(au.to_string())
            .bind(date)
            .bind(app)
            .fetch_optional(pool)
            .await
            .unwrap()
    }

    fn day(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    #[tokio::test]
    async fn retry_and_growth_keep_the_largest_total() {
        let pool = db::open_in_memory().await;
        let (agent, au) = agent_with_user(&pool, "m1", 1001).await;
        let send = |secs| AppUsageSync { entries: vec![entry(1001, "2026-10-20", "desktop:firefox", "Firefox", secs)] };

        assert_eq!(ingest(&pool, agent, &send(60), day("2026-10-20")).await.unwrap(), Ingested { stored: 1, rejected: 0 });
        ingest(&pool, agent, &send(60), day("2026-10-20")).await.unwrap(); // retry
        assert_eq!(seconds(&pool, au, "2026-10-20", "desktop:firefox").await, Some(60));
        ingest(&pool, agent, &send(120), day("2026-10-20")).await.unwrap(); // growth
        assert_eq!(seconds(&pool, au, "2026-10-20", "desktop:firefox").await, Some(120));
        ingest(&pool, agent, &send(90), day("2026-10-20")).await.unwrap(); // late, older total
        assert_eq!(seconds(&pool, au, "2026-10-20", "desktop:firefox").await, Some(120));
        ingest(&pool, agent, &send(10_000_000), day("2026-10-20")).await.unwrap(); // garbage
        assert_eq!(seconds(&pool, au, "2026-10-20", "desktop:firefox").await, Some(86_400));
    }

    #[tokio::test]
    async fn rejects_bad_ids_future_dates_and_unknown_users() {
        let pool = db::open_in_memory().await;
        let (agent, au) = agent_with_user(&pool, "m1", 1001).await;
        let sync = AppUsageSync {
            entries: vec![
                entry(1001, "2026-10-20", "desktop:../../etc", "x", 5),
                entry(1001, "2026-10-20", "nonsense", "x", 5),
                entry(1001, "2026-10-20", "exe:bash", "bash", 5),
                entry(1001, "2026-10-23", "desktop:foot", "Foot", 5),
                entry(4242, "2026-10-20", "desktop:foot", "Foot", 5),
                entry(1001, "2026-10-21", "desktop:foot", "Foot", 5), // agent a day ahead: fine
            ],
        };
        assert_eq!(ingest(&pool, agent, &sync, day("2026-10-20")).await.unwrap(), Ingested { stored: 1, rejected: 5 });
        assert_eq!(seconds(&pool, au, "2026-10-21", "desktop:foot").await, Some(5));
    }

    /// Same checks on Postgres: `SG_TEST_PG_URL=postgres://… cargo test -p server
    /// app_usage::tests::postgres -- --ignored` (needs an empty database).
    #[tokio::test]
    #[ignore]
    async fn postgres_ingest() {
        let url = std::env::var("SG_TEST_PG_URL").expect("SG_TEST_PG_URL");
        let cfg = crate::config::ServerConfig { database_url: Some(url), ..Default::default() };
        let pool = db::open(&cfg).await.unwrap();
        let (agent, au) = agent_with_user(&pool, "pg-m1", 1001).await;
        let send = |secs| AppUsageSync { entries: vec![entry(1001, "2026-10-20", "desktop:firefox", "Firefox", secs)] };
        ingest(&pool, agent, &send(60), day("2026-10-20")).await.unwrap();
        ingest(&pool, agent, &send(60), day("2026-10-20")).await.unwrap();
        ingest(&pool, agent, &send(30), day("2026-10-20")).await.unwrap();
        assert_eq!(seconds(&pool, au, "2026-10-20", "desktop:firefox").await, Some(60));
        ingest(&pool, agent, &send(90), day("2026-10-20")).await.unwrap();
        assert_eq!(seconds(&pool, au, "2026-10-20", "desktop:firefox").await, Some(90));
    }

    #[tokio::test]
    async fn catalog_keeps_latest_name_and_cleans_input() {
        let pool = db::open_in_memory().await;
        let (agent, _) = agent_with_user(&pool, "m1", 1001).await;
        let mut e = entry(1001, "2026-10-20", "steam:570", "Steam game 570", 5);
        ingest(&pool, agent, &AppUsageSync { entries: vec![e.clone()] }, day("2026-10-20")).await.unwrap();
        e.display_name = "  Dota\u{7} 2  ".into();
        e.icon = Some("".into());
        ingest(&pool, agent, &AppUsageSync { entries: vec![e] }, day("2026-10-20")).await.unwrap();
        let (name, icon): (String, Option<String>) =
            sqlx::query_as("SELECT display_name, icon FROM app_catalog WHERE app_id='steam:570'").fetch_one(&pool).await.unwrap();
        assert_eq!((name.as_str(), icon), ("Dota 2", None));
    }
}
