//! Blocking new apps (#27 Phase 2E): with the mode on, an app a child hasn't
//! used before is closed on start and put into the child's Blocked group; the
//! parents see a notice and allow it by taking it out of that group.
//!
//! Statuses: `approved` = known (its group decides), `pending` = blocked as new
//! and the parents haven't looked yet (a dashboard notice). `denied` is from
//! before the Blocked group (v17 moves those apps into it).

use anyhow::{Result, bail};
use chrono::Utc;
use serde::Serialize;
use uuid::Uuid;

use crate::db::DbPool;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Approval {
    pub app_id: String,
    pub name: String,
    pub icon: Option<String>,
    /// `pending`, `approved` or `denied`.
    pub status: String,
    pub requested_at: Option<i64>,
}

pub async fn mode(pool: &DbPool, profile_id: Uuid) -> Result<bool> {
    let m: Option<i32> = sqlx::query_scalar("SELECT approval_mode FROM profile_app_settings WHERE profile_id = $1")
        .bind(profile_id.to_string())
        .fetch_optional(pool)
        .await?;
    Ok(m.unwrap_or(0) != 0)
}

/// Turns the mode on or off. Turning it on approves every app the child has
/// already used, so nothing that works today stops working.
pub async fn set_mode(pool: &DbPool, profile_id: Uuid, on: bool) -> Result<()> {
    let p = profile_id.to_string();
    sqlx::query("DELETE FROM profile_app_settings WHERE profile_id = $1").bind(&p).execute(pool).await?;
    sqlx::query("INSERT INTO profile_app_settings (profile_id, approval_mode) VALUES ($1, $2)")
        .bind(&p)
        .bind(on as i32)
        .execute(pool)
        .await?;
    if on {
        let used: Vec<String> = sqlx::query_scalar(
            "SELECT DISTINCT u.app_id FROM app_usage u JOIN agent_users au ON au.id = u.agent_user_id
             WHERE au.profile_id = $1",
        )
        .bind(&p)
        .fetch_all(pool)
        .await?;
        let now = Utc::now().timestamp();
        for app in used {
            let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM app_approvals WHERE profile_id = $1 AND app_id = $2")
                .bind(&p)
                .bind(&app)
                .fetch_one(pool)
                .await?;
            if exists == 0 {
                sqlx::query("INSERT INTO app_approvals (profile_id, app_id, status, decided_at) VALUES ($1, $2, 'approved', $3)")
                    .bind(&p)
                    .bind(&app)
                    .bind(now)
                    .execute(pool)
                    .await?;
            }
        }
    }
    Ok(())
}

/// A new app was started: record it (a notice for the parents) and put it into
/// the Blocked group. An app that is already known is left alone. Returns true
/// if it was new.
pub async fn request(pool: &DbPool, profile_id: Uuid, app_id: &str) -> Result<bool> {
    let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM app_approvals WHERE profile_id = $1 AND app_id = $2")
        .bind(profile_id.to_string())
        .bind(app_id)
        .fetch_one(pool)
        .await?;
    if exists > 0 {
        return Ok(false);
    }
    sqlx::query("INSERT INTO app_approvals (profile_id, app_id, status, requested_at) VALUES ($1, $2, 'pending', $3)")
        .bind(profile_id.to_string())
        .bind(app_id)
        .bind(Utc::now().timestamp())
        .execute(pool)
        .await?;
    set_blocked(pool, profile_id, app_id, true).await?;
    Ok(true)
}

/// Puts an app into the profile's Blocked group or takes it out.
async fn set_blocked(pool: &DbPool, profile_id: Uuid, app_id: &str, blocked: bool) -> Result<()> {
    if let Some(gid) = crate::app_groups::blocked_group(pool, profile_id).await? {
        let categories = crate::app_usage::app_categories(pool, app_id).await?;
        crate::app_groups::set_membership(pool, profile_id, &gid, app_id, &categories, blocked).await?;
    }
    Ok(())
}

/// The parents decided about a new app: allow it (out of the Blocked group) or
/// keep it blocked (in the group). Either way the notice is gone.
pub async fn decide(pool: &DbPool, profile_id: Uuid, app_id: &str, approve: bool) -> Result<()> {
    if common::apps::AppId::parse(app_id).is_none() {
        bail!("invalid app id {app_id:?}");
    }
    set_status(pool, profile_id, app_id, "approved").await?;
    set_blocked(pool, profile_id, app_id, !approve).await
}

/// The parents moved the app themselves (on the board): the notice is gone.
pub async fn seen(pool: &DbPool, profile_id: Uuid, app_id: &str) -> Result<()> {
    sqlx::query("UPDATE app_approvals SET status = 'approved', decided_at = $1 WHERE profile_id = $2 AND app_id = $3 AND status <> 'approved'")
        .bind(Utc::now().timestamp())
        .bind(profile_id.to_string())
        .bind(app_id)
        .execute(pool)
        .await?;
    Ok(())
}

async fn set_status(pool: &DbPool, profile_id: Uuid, app_id: &str, status: &str) -> Result<()> {
    let now = Utc::now().timestamp();
    let r = sqlx::query("UPDATE app_approvals SET status = $1, decided_at = $2 WHERE profile_id = $3 AND app_id = $4")
        .bind(status)
        .bind(now)
        .bind(profile_id.to_string())
        .bind(app_id)
        .execute(pool)
        .await?;
    if r.rows_affected() == 0 {
        sqlx::query("INSERT INTO app_approvals (profile_id, app_id, status, decided_at) VALUES ($1, $2, $3, $4)")
            .bind(profile_id.to_string())
            .bind(app_id)
            .bind(status)
            .bind(now)
            .execute(pool)
            .await?;
    }
    Ok(())
}

/// All decisions and requests of a profile: pending first, then by name.
pub async fn list(pool: &DbPool, profile_id: Uuid) -> Result<Vec<Approval>> {
    // app_id, name, icon, status, requested_at
    type Row = (String, Option<String>, Option<String>, String, Option<i64>);
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT a.app_id, c.display_name, c.icon, a.status, a.requested_at FROM app_approvals a
         LEFT JOIN app_catalog c ON c.app_id = a.app_id WHERE a.profile_id = $1",
    )
    .bind(profile_id.to_string())
    .fetch_all(pool)
    .await?;
    let mut out: Vec<Approval> = rows
        .into_iter()
        .map(|(app_id, name, icon, status, requested_at)| Approval {
            name: name.unwrap_or_else(|| app_id.split_once(':').map_or(app_id.clone(), |(_, n)| n.to_string())),
            app_id,
            icon,
            status,
            requested_at,
        })
        .collect();
    out.sort_by(|a, b| {
        (a.status != "pending").cmp(&(b.status != "pending")).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(out)
}

/// New apps blocked that the parents haven't looked at, of every profile:
/// (profile id, profile name, approval). Shown even with the mode off: they
/// stay blocked.
pub async fn pending_all(pool: &DbPool) -> Result<Vec<(Uuid, String, Approval)>> {
    let mut out = Vec::new();
    for p in crate::db::list_profiles(pool).await? {
        for a in list(pool, p.id).await?.into_iter().filter(|a| a.status == "pending") {
            out.push((p.id, p.display_name.clone(), a));
        }
    }
    Ok(out)
}

/// What the agent needs: `None` when the mode is off. Every known app counts as
/// approved — a new one is kept closed by the Blocked group, not by this list.
pub async fn config(pool: &DbPool, profile_id: Uuid) -> Result<Option<common::models::AppApprovalConfig>> {
    if !mode(pool, profile_id).await? {
        return Ok(None);
    }
    let all = list(pool, profile_id).await?;
    let pick = |f: fn(&str) -> bool| all.iter().filter(|a| f(&a.status)).map(|a| a.app_id.clone()).collect::<Vec<_>>();
    Ok(Some(common::models::AppApprovalConfig { approved: pick(|s| s != "denied"), denied: pick(|s| s == "denied") }))
}

/// Migration v17: every profile gets a Blocked group; apps denied before go
/// into it (and count as known), apps waiting for a decision too (the notice stays).
pub async fn move_into_blocked(pool: &DbPool) -> Result<()> {
    for p in crate::db::list_profiles(pool).await? {
        crate::app_groups::seed_blocked(pool, p.id).await?;
        for a in list(pool, p.id).await? {
            match a.status.as_str() {
                "denied" => {
                    set_status(pool, p.id, &a.app_id, "approved").await?;
                    set_blocked(pool, p.id, &a.app_id, true).await?;
                }
                "pending" => set_blocked(pool, p.id, &a.app_id, true).await?,
                _ => {}
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    #[tokio::test]
    async fn mode_requests_and_decisions() {
        let pool = db::open_in_memory().await;
        let (_, _, _, profile) = crate::app_usage::tests::seeded(&pool).await; // firefox, steam:570, foot used
        assert!(!mode(&pool, profile).await.unwrap());
        assert!(config(&pool, profile).await.unwrap().is_none());

        // turning it on approves what the child already used
        set_mode(&pool, profile, true).await.unwrap();
        let mut approved = config(&pool, profile).await.unwrap().unwrap().approved;
        approved.sort();
        assert_eq!(approved, vec!["desktop:firefox", "desktop:foot", "steam:570"]);

        // a new app: into Blocked with a notice; repeated requests change nothing
        let blocked = crate::app_groups::blocked_group(&pool, profile).await.unwrap().unwrap();
        let group = |app: &'static str| {
            let pool = pool.clone();
            async move { crate::app_groups::group_of(&crate::app_groups::list_groups(&pool, profile).await.unwrap(), app, &[]).map(str::to_string) }
        };
        assert!(request(&pool, profile, "steam:1091500").await.unwrap());
        assert!(!request(&pool, profile, "steam:1091500").await.unwrap());
        assert!(!request(&pool, profile, "desktop:firefox").await.unwrap()); // already known
        let l = list(&pool, profile).await.unwrap();
        assert_eq!((l[0].app_id.as_str(), l[0].status.as_str()), ("steam:1091500", "pending"));
        assert_eq!(group("steam:1091500").await, Some(blocked.clone())); // not Games (steam:)
        // known to the agent: the Blocked group keeps it closed, not the approval list
        assert!(config(&pool, profile).await.unwrap().unwrap().approved.contains(&"steam:1091500".to_string()));
        assert_eq!(pending_all(&pool).await.unwrap().len(), 1);
        set_mode(&pool, profile, false).await.unwrap(); // still blocked with the mode off: still shown
        assert_eq!(pending_all(&pool).await.unwrap().len(), 1);
        set_mode(&pool, profile, true).await.unwrap();

        // keep it blocked: the notice goes, the app stays in Blocked
        decide(&pool, profile, "steam:1091500", false).await.unwrap();
        assert!(pending_all(&pool).await.unwrap().is_empty());
        assert_eq!(group("steam:1091500").await, Some(blocked.clone()));
        // allow it after all: back to where its rules put it (Games)
        decide(&pool, profile, "steam:1091500", true).await.unwrap();
        let games = crate::app_groups::list_groups(&pool, profile).await.unwrap()[0].id.clone();
        assert_eq!(group("steam:1091500").await, Some(games));
        assert!(config(&pool, profile).await.unwrap().unwrap().denied.is_empty());

        // allow in advance; turning the mode off keeps the decisions for next time
        decide(&pool, profile, "flatpak:org.example.App", true).await.unwrap();
        assert!(decide(&pool, profile, "bad id", true).await.is_err());
        set_mode(&pool, profile, false).await.unwrap();
        assert!(config(&pool, profile).await.unwrap().is_none());
        set_mode(&pool, profile, true).await.unwrap();
        assert!(config(&pool, profile).await.unwrap().unwrap().approved.contains(&"flatpak:org.example.App".to_string()));
    }

    #[tokio::test]
    async fn v17_moves_denied_and_waiting_apps_into_blocked() {
        let pool = db::open_in_memory().await;
        let (_, _, _, profile) = crate::app_usage::tests::seeded(&pool).await;
        // as before v17: no Blocked group, a denied and a waiting app
        let blocked = crate::app_groups::blocked_group(&pool, profile).await.unwrap().unwrap();
        crate::app_groups::delete_group(&pool, profile, &blocked).await.unwrap();
        let p = profile.to_string();
        for (app, status) in [("desktop:denied", "denied"), ("desktop:waiting", "pending"), ("desktop:firefox", "approved")] {
            sqlx::query("INSERT INTO app_approvals (profile_id, app_id, status) VALUES ($1, $2, $3)")
                .bind(&p).bind(app).bind(status).execute(&pool).await.unwrap();
        }
        move_into_blocked(&pool).await.unwrap();
        let groups = crate::app_groups::list_groups(&pool, profile).await.unwrap();
        let blocked = crate::app_groups::blocked_group(&pool, profile).await.unwrap().unwrap();
        let of = |a: &str| crate::app_groups::group_of(&groups, a, &[]).map(str::to_string);
        assert_eq!((of("desktop:denied"), of("desktop:waiting")), (Some(blocked.clone()), Some(blocked)));
        assert_eq!(of("desktop:firefox"), None);
        let status: Vec<(String, String)> = list(&pool, profile).await.unwrap().into_iter().map(|a| (a.app_id, a.status)).collect();
        assert!(status.contains(&("desktop:denied".into(), "approved".into())));
        assert!(status.contains(&("desktop:waiting".into(), "pending".into()))); // the notice stays
        move_into_blocked(&pool).await.unwrap(); // twice is harmless
        assert_eq!(crate::app_groups::list_groups(&pool, profile).await.unwrap().len(), groups.len());
    }
}
