//! New-app approval (#27 Phase 2E): with the mode on, an app a child hasn't
//! been allowed yet is closed on start and a request goes to the parents.

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

/// Records a request; an app that already has a decision (or a pending request)
/// is left alone. Returns true if a new request was created.
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
    Ok(true)
}

/// Approve or deny (also changes an earlier decision, or decides in advance).
pub async fn decide(pool: &DbPool, profile_id: Uuid, app_id: &str, approve: bool) -> Result<()> {
    if common::apps::AppId::parse(app_id).is_none() {
        bail!("invalid app id {app_id:?}");
    }
    let status = if approve { "approved" } else { "denied" };
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

/// Pending requests of every profile: (profile id, profile name, approval).
pub async fn pending_all(pool: &DbPool) -> Result<Vec<(Uuid, String, Approval)>> {
    let mut out = Vec::new();
    for p in crate::db::list_profiles(pool).await? {
        for a in list(pool, p.id).await?.into_iter().filter(|a| a.status == "pending") {
            out.push((p.id, p.display_name.clone(), a));
        }
    }
    Ok(out)
}

/// What the agent needs: `None` when the mode is off.
pub async fn config(pool: &DbPool, profile_id: Uuid) -> Result<Option<common::models::AppApprovalConfig>> {
    if !mode(pool, profile_id).await? {
        return Ok(None);
    }
    let all = list(pool, profile_id).await?;
    let pick = |s: &str| all.iter().filter(|a| a.status == s).map(|a| a.app_id.clone()).collect::<Vec<_>>();
    Ok(Some(common::models::AppApprovalConfig { approved: pick("approved"), denied: pick("denied") }))
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

        // a new app: one pending request, repeated requests change nothing
        assert!(request(&pool, profile, "steam:1091500").await.unwrap());
        assert!(!request(&pool, profile, "steam:1091500").await.unwrap());
        assert!(!request(&pool, profile, "desktop:firefox").await.unwrap()); // already approved
        let l = list(&pool, profile).await.unwrap();
        assert_eq!((l[0].app_id.as_str(), l[0].status.as_str()), ("steam:1091500", "pending"));
        assert_eq!(pending_all(&pool).await.unwrap().len(), 1);

        // deny, then change mind
        decide(&pool, profile, "steam:1091500", false).await.unwrap();
        assert_eq!(config(&pool, profile).await.unwrap().unwrap().denied, vec!["steam:1091500"]);
        decide(&pool, profile, "steam:1091500", true).await.unwrap();
        let c = config(&pool, profile).await.unwrap().unwrap();
        assert!(c.approved.contains(&"steam:1091500".to_string()) && c.denied.is_empty());
        assert!(pending_all(&pool).await.unwrap().is_empty());

        // allow in advance; turning the mode off keeps the decisions for next time
        decide(&pool, profile, "flatpak:org.example.App", true).await.unwrap();
        assert!(decide(&pool, profile, "bad id", true).await.is_err());
        set_mode(&pool, profile, false).await.unwrap();
        assert!(config(&pool, profile).await.unwrap().is_none());
        set_mode(&pool, profile, true).await.unwrap();
        assert!(config(&pool, profile).await.unwrap().unwrap().approved.contains(&"flatpak:org.example.App".to_string()));
    }
}
