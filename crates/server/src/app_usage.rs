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
const MAX_CATEGORIES: usize = 20;

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
        // freedesktop category names are plain words (`Game`, `X-Foo-Bar`)
        let categories = e
            .categories
            .iter()
            .filter(|c| !c.is_empty() && c.len() <= 64 && c.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_'))
            .take(MAX_CATEGORIES)
            .cloned()
            .collect::<Vec<_>>()
            .join(";");
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
            "INSERT INTO app_catalog (app_id, display_name, icon, categories, first_seen, last_seen) VALUES ($1,$2,$3,$4,$5,$5)
             ON CONFLICT(app_id)
             DO UPDATE SET display_name=EXCLUDED.display_name, icon=EXCLUDED.icon,
                           categories=EXCLUDED.categories, last_seen=EXCLUDED.last_seen",
        )
        .bind(app.to_string())
        .bind(name)
        .bind(icon)
        .bind(categories)
        .bind(now)
        .execute(pool)
        .await?;
        out.stored += 1;
    }
    Ok(out)
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct AppTotal {
    pub app_id: String,
    pub name: String,
    pub icon: Option<String>,
    pub categories: Vec<String>,
    pub seconds: i64,
}

/// Per-app totals of a profile between two dates (inclusive), summed over all
/// its agents; longest first. `None` dates mean unbounded.
pub async fn totals(pool: &DbPool, profile_id: Uuid, from: Option<NaiveDate>, to: Option<NaiveDate>) -> Result<Vec<AppTotal>> {
    let rows: Vec<(String, Option<String>, Option<String>, Option<String>, i64)> = sqlx::query_as(
        "SELECT u.app_id, c.display_name, c.icon, c.categories, SUM(u.seconds) AS total
         FROM app_usage u
         JOIN agent_users au ON au.id = u.agent_user_id
         LEFT JOIN app_catalog c ON c.app_id = u.app_id
         WHERE au.profile_id = $1 AND u.date >= $2 AND u.date <= $3
         GROUP BY u.app_id, c.display_name, c.icon, c.categories",
    )
    .bind(profile_id.to_string())
    .bind(from.map_or("0000-00-00".to_string(), |d| d.to_string()))
    .bind(to.map_or("9999-99-99".to_string(), |d| d.to_string()))
    .fetch_all(pool)
    .await?;
    let mut out: Vec<AppTotal> = rows
        .into_iter()
        .map(|(app_id, name, icon, categories, seconds)| AppTotal {
            name: name.unwrap_or_else(|| app_id.split_once(':').map_or(app_id.clone(), |(_, n)| n.to_string())),
            app_id,
            icon,
            categories: split_categories(categories.as_deref().unwrap_or("")),
            seconds,
        })
        .collect();
    out.sort_by(|a, b| b.seconds.cmp(&a.seconds).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    Ok(out)
}

/// (date, app_id, seconds) per day between two dates, for the week view.
pub async fn daily(pool: &DbPool, profile_id: Uuid, from: NaiveDate, to: NaiveDate) -> Result<Vec<(String, String, i64)>> {
    Ok(sqlx::query_as(
        "SELECT u.date, u.app_id, SUM(u.seconds) AS total
         FROM app_usage u
         JOIN agent_users au ON au.id = u.agent_user_id
         WHERE au.profile_id = $1 AND u.date >= $2 AND u.date <= $3
         GROUP BY u.date, u.app_id
         ORDER BY u.date, u.app_id",
    )
    .bind(profile_id.to_string())
    .bind(from.to_string())
    .bind(to.to_string())
    .fetch_all(pool)
    .await?)
}

/// Does any agent of the profile report per-app usage?
pub async fn profile_has_capability(pool: &DbPool, profile_id: Uuid) -> Result<bool> {
    let n: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM agents a JOIN agent_users au ON au.agent_id = a.id
         WHERE au.profile_id = $1 AND a.app_usage_available = 1",
    )
    .bind(profile_id.to_string())
    .fetch_one(pool)
    .await?;
    Ok(n > 0)
}

pub fn split_categories(s: &str) -> Vec<String> {
    s.split(';').filter(|c| !c.is_empty()).map(str::to_string).collect()
}

/// Trimmed, without control characters, at most `max` chars; `None` if empty.
fn clean(s: &str, max: usize) -> Option<String> {
    let s: String = s.chars().filter(|c| !c.is_control()).take(max).collect();
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

#[cfg(test)]
pub(crate) mod tests {
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
            categories: if app.starts_with("steam:") { vec!["Game".into()] } else { vec![] },
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

        let (a1, _, _, profile) = seeded(&pool).await;
        let t = totals(&pool, profile, Some(day("2026-10-19")), Some(day("2026-10-25"))).await.unwrap();
        assert_eq!(t.iter().map(|a| (a.app_id.as_str(), a.seconds)).collect::<Vec<_>>(), vec![("steam:570", 4200), ("desktop:firefox", 3000)]);
        assert_eq!(t[0].name, "Dota 2");
        assert_eq!(daily(&pool, profile, day("2026-10-19"), day("2026-10-25")).await.unwrap().len(), 3);
        db::update_agent_app_usage(&pool, a1, true).await.unwrap();
        assert!(profile_has_capability(&pool, profile).await.unwrap());
    }

    /// Two agents in one profile, one in another; returns (agent1, agent2, other, profile).
    pub(crate) async fn seeded(pool: &DbPool) -> (Uuid, Uuid, Uuid, Uuid) {
        let profile = db::create_profile(pool, "Kid").await.unwrap().id;
        let other_profile = db::create_profile(pool, "Other").await.unwrap().id;
        let (a1, au1) = agent_with_user(pool, "laptop", 1001).await;
        let (a2, au2) = agent_with_user(pool, "desktop", 1000).await;
        let (a3, au3) = agent_with_user(pool, "other", 1000).await;
        for (au, p) in [(au1, profile), (au2, profile), (au3, other_profile)] {
            db::update_agent_user(pool, au, Some(p), Some("managed")).await.unwrap();
        }
        let d = day("2026-10-20");
        let put = |agent, entries: Vec<AppUsageEntry>| async move {
            ingest(pool, agent, &AppUsageSync { entries }, d).await.unwrap();
        };
        put(a1, vec![
            entry(1001, "2026-10-20", "desktop:firefox", "Firefox", 1800),
            entry(1001, "2026-10-20", "steam:570", "Dota 2", 3600),
            entry(1001, "2026-10-19", "steam:570", "Dota 2", 600), // Monday of that week
            entry(1001, "2026-10-12", "desktop:foot", "Foot", 60),  // previous week
        ]).await;
        put(a2, vec![entry(1000, "2026-10-20", "desktop:firefox", "Firefox", 1200)]).await;
        put(a3, vec![entry(1000, "2026-10-20", "desktop:firefox", "Firefox", 9999)]).await;
        (a1, a2, a3, profile)
    }

    #[tokio::test]
    async fn totals_sum_agents_of_the_profile_only() {
        let pool = db::open_in_memory().await;
        let (_, _, _, profile) = seeded(&pool).await;
        let t = |from, to| {
            let pool = pool.clone();
            async move {
                totals(&pool, profile, from, to).await.unwrap().into_iter().map(|a| (a.app_id, a.seconds)).collect::<Vec<_>>()
            }
        };
        let d = |s| Some(day(s));
        assert_eq!(t(d("2026-10-20"), d("2026-10-20")).await, vec![("steam:570".into(), 3600), ("desktop:firefox".into(), 3000)]);
        assert_eq!(t(d("2026-10-19"), d("2026-10-25")).await, vec![("steam:570".into(), 4200), ("desktop:firefox".into(), 3000)]);
        assert_eq!(t(None, None).await, vec![("steam:570".into(), 4200), ("desktop:firefox".into(), 3000), ("desktop:foot".into(), 60)]);
        let first = &totals(&pool, profile, None, None).await.unwrap()[0];
        assert_eq!((first.name.as_str(), first.icon.as_deref()), ("Dota 2", Some("icon")));
        assert_eq!(first.categories, vec!["Game"]);

        let days = daily(&pool, profile, day("2026-10-19"), day("2026-10-25")).await.unwrap();
        assert_eq!(days, vec![
            ("2026-10-19".into(), "steam:570".into(), 600),
            ("2026-10-20".into(), "desktop:firefox".into(), 3000),
            ("2026-10-20".into(), "steam:570".into(), 3600),
        ]);
    }

    #[tokio::test]
    async fn capability_follows_the_profiles_agents() {
        let pool = db::open_in_memory().await;
        let (a1, _, a3, profile) = seeded(&pool).await;
        assert!(!profile_has_capability(&pool, profile).await.unwrap());
        db::update_agent_app_usage(&pool, a3, true).await.unwrap(); // other profile's agent
        assert!(!profile_has_capability(&pool, profile).await.unwrap());
        db::update_agent_app_usage(&pool, a1, true).await.unwrap();
        assert!(profile_has_capability(&pool, profile).await.unwrap());
    }

    #[tokio::test]
    async fn catalog_keeps_latest_name_and_cleans_input() {
        let pool = db::open_in_memory().await;
        let (agent, _) = agent_with_user(&pool, "m1", 1001).await;
        let mut e = entry(1001, "2026-10-20", "steam:570", "Steam game 570", 5);
        ingest(&pool, agent, &AppUsageSync { entries: vec![e.clone()] }, day("2026-10-20")).await.unwrap();
        e.display_name = "  Dota\u{7} 2  ".into();
        e.icon = Some("".into());
        e.categories = vec!["Game".into(), "Bad;Inject".into(), "".into(), "X-Valve".into()];
        ingest(&pool, agent, &AppUsageSync { entries: vec![e] }, day("2026-10-20")).await.unwrap();
        let (name, icon, cats): (String, Option<String>, String) =
            sqlx::query_as("SELECT display_name, icon, categories FROM app_catalog WHERE app_id='steam:570'").fetch_one(&pool).await.unwrap();
        assert_eq!((name.as_str(), icon, cats.as_str()), ("Dota 2", None, "Game;X-Valve"));
    }
}
