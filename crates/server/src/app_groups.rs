//! App groups (#27 Phase 2): each profile (child) has its own groups of apps,
//! defined by rules, with daily limits. Predefined: "Games" and "Web browsers"
//! (created with every profile).

use anyhow::{Result, bail};
use chrono::Utc;
use common::apps::{GroupRule, RuleKind};
use serde::Serialize;
use uuid::Uuid;

use crate::db::DbPool;

/// Predefined groups: (builtin key, name, rules). Most parents don't know which
/// games or browsers exist, so these are built from freedesktop categories and
/// pick up new apps by themselves.
type Builtin = (&'static str, &'static str, &'static [(RuleKind, &'static str)]);
const BUILTIN: &[Builtin] = &[
    ("games", "Games", &[(RuleKind::Category, "Game"), (RuleKind::Prefix, "steam:"), (RuleKind::Prefix, "minecraft:")]),
    ("browsers", "Web browsers", &[(RuleKind::Category, "WebBrowser")]),
];

pub const MAX_GROUP_NAME: usize = 60;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuleRow {
    pub id: String,
    #[serde(flatten)]
    pub rule: GroupRule,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Group {
    pub id: String,
    pub name: String,
    /// `games` / `browsers` for the predefined groups.
    pub builtin: Option<String>,
    pub rules: Vec<RuleRow>,
}

impl Group {
    pub fn rules(&self) -> Vec<GroupRule> {
        self.rules.iter().map(|r| r.rule.clone()).collect()
    }
}

/// The one group of an app among `groups` (in display order), see `common::apps::assign_group`.
pub fn group_of<'a>(groups: &'a [Group], app_id: &str, categories: &[String]) -> Option<&'a str> {
    let rules: Vec<(String, Vec<GroupRule>)> = groups.iter().map(|g| (g.id.clone(), g.rules())).collect();
    let refs: Vec<(&str, &[GroupRule])> = rules.iter().map(|(id, r)| (id.as_str(), r.as_slice())).collect();
    let id = common::apps::assign_group(&refs, app_id, categories)?;
    groups.iter().find(|g| g.id == id).map(|g| g.id.as_str())
}

fn kind_str(k: RuleKind) -> &'static str {
    match k {
        RuleKind::AppId => "app_id",
        RuleKind::Prefix => "prefix",
        RuleKind::Category => "category",
    }
}

fn kind_from(s: &str) -> Option<RuleKind> {
    Some(match s {
        "app_id" => RuleKind::AppId,
        "prefix" => RuleKind::Prefix,
        "category" => RuleKind::Category,
        _ => return None,
    })
}

/// Creates a profile's predefined groups that don't exist yet. Called when a
/// profile is created (and by migration v16) — a group the parent deleted later
/// stays gone.
pub async fn seed_builtin(pool: &DbPool, profile_id: Uuid) -> Result<()> {
    for (key, name, rules) in BUILTIN {
        let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM app_groups WHERE builtin = $1 AND profile_id = $2")
            .bind(*key)
            .bind(profile_id.to_string())
            .fetch_one(pool)
            .await?;
        if exists > 0 {
            continue;
        }
        let id = create_group_with(pool, profile_id, name, Some(key)).await?;
        for (kind, value) in *rules {
            add_rule(pool, &id, &GroupRule { kind: *kind, value: value.to_string(), exclude: false }).await?;
        }
    }
    Ok(())
}

async fn create_group_with(pool: &DbPool, profile_id: Uuid, name: &str, builtin: Option<&str>) -> Result<String> {
    let id = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO app_groups (id, profile_id, name, builtin, created_at) VALUES ($1,$2,$3,$4,$5)")
        .bind(&id)
        .bind(profile_id.to_string())
        .bind(name)
        .bind(builtin)
        .bind(Utc::now().timestamp())
        .execute(pool)
        .await?;
    Ok(id)
}

fn clean_name(name: &str) -> Result<String> {
    let name: String = name.chars().filter(|c| !c.is_control()).collect();
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_GROUP_NAME {
        bail!("group name must be 1-{MAX_GROUP_NAME} characters");
    }
    Ok(name.to_string())
}

pub async fn create_group(pool: &DbPool, profile_id: Uuid, name: &str) -> Result<String> {
    create_group_with(pool, profile_id, &clean_name(name)?, None).await
}

/// `false` if the profile has no such group.
pub async fn rename_group(pool: &DbPool, profile_id: Uuid, id: &str, name: &str) -> Result<bool> {
    let r = sqlx::query("UPDATE app_groups SET name = $1 WHERE id = $2 AND profile_id = $3")
        .bind(clean_name(name)?)
        .bind(id)
        .bind(profile_id.to_string())
        .execute(pool)
        .await?;
    Ok(r.rows_affected() > 0)
}

pub async fn delete_group(pool: &DbPool, profile_id: Uuid, id: &str) -> Result<bool> {
    if !group_exists(pool, profile_id, id).await? {
        return Ok(false);
    }
    delete_group_rows(pool, id).await?;
    Ok(true)
}

async fn delete_group_rows(pool: &DbPool, id: &str) -> Result<()> {
    // explicit, so it doesn't depend on foreign keys being enabled
    sqlx::query("DELETE FROM app_group_rules WHERE group_id = $1").bind(id).execute(pool).await?;
    sqlx::query("DELETE FROM app_group_limits WHERE group_id = $1").bind(id).execute(pool).await?;
    sqlx::query("DELETE FROM app_group_adjustments WHERE group_id = $1").bind(id).execute(pool).await?;
    sqlx::query("DELETE FROM app_group_usage WHERE group_id = $1").bind(id).execute(pool).await?;
    sqlx::query("DELETE FROM app_groups WHERE id = $1").bind(id).execute(pool).await?;
    Ok(())
}

/// Deletes every group of a profile (the profile is being deleted).
pub async fn delete_profile_groups(pool: &DbPool, profile_id: Uuid) -> Result<()> {
    let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM app_groups WHERE profile_id = $1")
        .bind(profile_id.to_string())
        .fetch_all(pool)
        .await?;
    for id in ids {
        delete_group_rows(pool, &id).await?;
    }
    Ok(())
}

/// Validates and adds a rule; adding an existing rule is a no-op. Returns the rule id.
pub async fn add_rule(pool: &DbPool, group_id: &str, rule: &GroupRule) -> Result<String> {
    let value = rule.value.trim();
    let valid = match rule.kind {
        RuleKind::AppId => common::apps::AppId::parse(value).is_some(),
        RuleKind::Prefix => value.ends_with(':') && value.len() <= 32 && value[..value.len() - 1].chars().all(|c| c.is_ascii_lowercase()),
        RuleKind::Category => !value.is_empty() && value.len() <= 64 && value.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
    };
    if !valid {
        bail!("invalid {} rule: {value:?}", kind_str(rule.kind));
    }
    let existing: Option<String> = sqlx::query_scalar(
        "SELECT id FROM app_group_rules WHERE group_id=$1 AND kind=$2 AND value=$3 AND exclude=$4",
    )
    .bind(group_id)
    .bind(kind_str(rule.kind))
    .bind(value)
    .bind(rule.exclude as i32)
    .fetch_optional(pool)
    .await?;
    if let Some(id) = existing {
        return Ok(id);
    }
    // an app can't be both explicitly in and explicitly out: the new rule replaces the opposite
    sqlx::query("DELETE FROM app_group_rules WHERE group_id=$1 AND kind=$2 AND value=$3 AND exclude=$4")
        .bind(group_id)
        .bind(kind_str(rule.kind))
        .bind(value)
        .bind((!rule.exclude) as i32)
        .execute(pool)
        .await?;
    let id = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO app_group_rules (id, group_id, kind, value, exclude) VALUES ($1,$2,$3,$4,$5)")
        .bind(&id)
        .bind(group_id)
        .bind(kind_str(rule.kind))
        .bind(value)
        .bind(rule.exclude as i32)
        .execute(pool)
        .await?;
    Ok(id)
}

pub async fn delete_rule(pool: &DbPool, group_id: &str, rule_id: &str) -> Result<bool> {
    let r = sqlx::query("DELETE FROM app_group_rules WHERE id = $1 AND group_id = $2").bind(rule_id).bind(group_id).execute(pool).await?;
    Ok(r.rows_affected() > 0)
}

/// A profile's groups with their rules: predefined first, then by name.
pub async fn list_groups(pool: &DbPool, profile_id: Uuid) -> Result<Vec<Group>> {
    let groups: Vec<(String, String, Option<String>)> = sqlx::query_as("SELECT id, name, builtin FROM app_groups WHERE profile_id = $1")
        .bind(profile_id.to_string())
        .fetch_all(pool)
        .await?;
    let rules: Vec<(String, String, String, String, i32)> = sqlx::query_as(
        "SELECT r.id, r.group_id, r.kind, r.value, r.exclude FROM app_group_rules r
         JOIN app_groups g ON g.id = r.group_id WHERE g.profile_id = $1 ORDER BY r.kind, r.value",
    )
    .bind(profile_id.to_string())
    .fetch_all(pool)
    .await?;
    let mut out: Vec<Group> = groups
        .into_iter()
        .map(|(id, name, builtin)| {
            let rules = rules
                .iter()
                .filter(|r| r.1 == id)
                .filter_map(|(rid, _, kind, value, exclude)| {
                    Some(RuleRow { id: rid.clone(), rule: GroupRule { kind: kind_from(kind)?, value: value.clone(), exclude: *exclude != 0 } })
                })
                .collect();
            Group { id, name, builtin, rules }
        })
        .collect();
    out.sort_by(|a, b| {
        let order = |g: &Group| BUILTIN.iter().position(|(k, _, _)| g.builtin.as_deref() == Some(k)).unwrap_or(BUILTIN.len());
        order(a).cmp(&order(b)).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(out)
}

/// Puts an app into a group (`member`) or takes it out, whatever the rules that
/// currently decide it: removes/adds an explicit entry, or an exclusion when the
/// app is in the group through a category or prefix. `None` if no such group.
pub async fn set_membership(
    pool: &DbPool,
    profile_id: Uuid,
    group_id: &str,
    app_id: &str,
    categories: &[String],
    member: bool,
) -> Result<Option<()>> {
    if common::apps::AppId::parse(app_id).is_none() {
        bail!("invalid app id {app_id:?}");
    }
    let groups = list_groups(pool, profile_id).await?;
    let Some(group) = groups.iter().find(|g| g.id == group_id) else {
        return Ok(None);
    };
    // drop the explicit rule that says the opposite of what's wanted
    if let Some(r) = group.rules.iter().find(|r| r.rule.kind == RuleKind::AppId && r.rule.value == app_id && r.rule.exclude == member) {
        delete_rule(pool, group_id, &r.id).await?;
    }
    if member {
        // an app is in one group: moving it here takes it out of any group that names it
        for other in groups.iter().filter(|g| g.id != group_id) {
            for r in other.rules.iter().filter(|r| r.rule.kind == RuleKind::AppId && !r.rule.exclude && r.rule.value == app_id) {
                delete_rule(pool, &other.id, &r.id).await?;
            }
        }
    }
    let groups = list_groups(pool, profile_id).await?;
    let in_this = group_of(&groups, app_id, categories) == Some(group_id);
    if member && !in_this {
        // an explicit entry wins over category matches in other groups
        add_rule(pool, group_id, &GroupRule { kind: RuleKind::AppId, value: app_id.into(), exclude: false }).await?;
        if group_of(&list_groups(pool, profile_id).await?, app_id, categories) != Some(group_id) {
            // still taken by an exclusion here — remove it
            if let Some(g) = list_groups(pool, profile_id).await?.into_iter().find(|g| g.id == group_id) {
                for r in g.rules.iter().filter(|r| r.rule.exclude && r.rule.kind == RuleKind::AppId && r.rule.value == app_id) {
                    delete_rule(pool, group_id, &r.id).await?;
                }
            }
        }
    } else if !member && in_this {
        add_rule(pool, group_id, &GroupRule { kind: RuleKind::AppId, value: app_id.into(), exclude: true }).await?;
    }
    Ok(Some(()))
}

pub async fn get_group(pool: &DbPool, profile_id: Uuid, id: &str) -> Result<Option<Group>> {
    Ok(list_groups(pool, profile_id).await?.into_iter().find(|g| g.id == id))
}

/// A profile's limits per group: group id → (weekday 0=Mon, minutes), 0 = blocked.
pub async fn limits(pool: &DbPool, profile_id: &str) -> Result<std::collections::HashMap<String, Vec<(u8, i32)>>> {
    let rows: Vec<(String, i32, i32)> = sqlx::query_as(
        "SELECT group_id, day_of_week, allowed_minutes FROM app_group_limits WHERE profile_id = $1 ORDER BY day_of_week",
    )
    .bind(profile_id)
    .fetch_all(pool)
    .await?;
    let mut out: std::collections::HashMap<String, Vec<(u8, i32)>> = std::collections::HashMap::new();
    for (g, dow, mins) in rows {
        out.entry(g).or_default().push((dow as u8, mins));
    }
    Ok(out)
}

/// Replaces a profile's limits for one group; an empty list removes the limit.
pub async fn replace_limits(pool: &DbPool, profile_id: &str, group_id: &str, entries: &[(u8, i32)]) -> Result<()> {
    let mut seen = [false; 7];
    for &(dow, mins) in entries {
        if dow > 6 || !(0..=1440).contains(&mins) || std::mem::replace(&mut seen[dow as usize], true) {
            bail!("invalid limit: day {dow}, {mins} minutes");
        }
    }
    sqlx::query("DELETE FROM app_group_limits WHERE profile_id = $1 AND group_id = $2")
        .bind(profile_id)
        .bind(group_id)
        .execute(pool)
        .await?;
    for &(dow, mins) in entries {
        sqlx::query("INSERT INTO app_group_limits (profile_id, group_id, day_of_week, allowed_minutes) VALUES ($1,$2,$3,$4)")
            .bind(profile_id)
            .bind(group_id)
            .bind(dow as i32)
            .bind(mins)
            .execute(pool)
            .await?;
    }
    Ok(())
}

/// Adds extra (or, if negative, fewer) minutes for a group on a date.
pub async fn add_adjustment(pool: &DbPool, profile_id: &str, group_id: &str, date: &str, minutes: i32) -> Result<()> {
    if minutes == 0 || !(-1440..=1440).contains(&minutes) {
        bail!("adjustment must be between -1440 and 1440 minutes, not 0");
    }
    sqlx::query(
        "INSERT INTO app_group_adjustments (id, profile_id, group_id, target_date, minutes, created_at) VALUES ($1,$2,$3,$4,$5,$6)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(profile_id)
    .bind(group_id)
    .bind(date)
    .bind(minutes)
    .bind(Utc::now().timestamp())
    .execute(pool)
    .await?;
    Ok(())
}

/// Sum of a profile's adjustments per group on a date.
pub async fn adjustments(pool: &DbPool, profile_id: &str, date: &str) -> Result<std::collections::HashMap<String, i32>> {
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT group_id, SUM(minutes) FROM app_group_adjustments WHERE profile_id = $1 AND target_date = $2 GROUP BY group_id",
    )
    .bind(profile_id)
    .bind(date)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|(g, m)| (g, m as i32)).collect())
}

/// Does the profile have this group?
pub async fn group_exists(pool: &DbPool, profile_id: Uuid, id: &str) -> Result<bool> {
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM app_groups WHERE id = $1 AND profile_id = $2")
        .bind(id)
        .bind(profile_id.to_string())
        .fetch_one(pool)
        .await?;
    Ok(n > 0)
}

/// Migration v16: groups used to be shared by all profiles. Give every profile
/// its own copy (rules included), move its limits, extra time and recorded
/// group time to the copies, then drop the shared ones. Profiles without any
/// shared group (database older than groups) get the predefined ones.
pub async fn split_shared_groups(pool: &DbPool) -> Result<()> {
    let shared: Vec<(String, String, Option<String>)> =
        sqlx::query_as("SELECT id, name, builtin FROM app_groups WHERE profile_id IS NULL").fetch_all(pool).await?;
    for profile in crate::db::list_profiles(pool).await? {
        let p = profile.id.to_string();
        if shared.is_empty() {
            seed_builtin(pool, profile.id).await?;
            continue;
        }
        for (old, name, builtin) in &shared {
            let new = create_group_with(pool, profile.id, name, builtin.as_deref()).await?;
            let rules: Vec<(String, String, i32)> =
                sqlx::query_as("SELECT kind, value, exclude FROM app_group_rules WHERE group_id = $1").bind(old).fetch_all(pool).await?;
            for (kind, value, exclude) in rules {
                sqlx::query("INSERT INTO app_group_rules (id, group_id, kind, value, exclude) VALUES ($1,$2,$3,$4,$5)")
                    .bind(Uuid::new_v4().to_string())
                    .bind(&new)
                    .bind(kind)
                    .bind(value)
                    .bind(exclude)
                    .execute(pool)
                    .await?;
            }
            for table in ["app_group_limits", "app_group_adjustments"] {
                sqlx::query(&format!("UPDATE {table} SET group_id = $1 WHERE group_id = $2 AND profile_id = $3"))
                    .bind(&new)
                    .bind(old)
                    .bind(&p)
                    .execute(pool)
                    .await?;
            }
            sqlx::query(
                "UPDATE app_group_usage SET group_id = $1 WHERE group_id = $2
                 AND agent_user_id IN (SELECT id FROM agent_users WHERE profile_id = $3)",
            )
            .bind(&new)
            .bind(old)
            .bind(&p)
            .execute(pool)
            .await?;
        }
    }
    for (old, _, _) in &shared {
        delete_group_rows(pool, old).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    async fn kid(pool: &DbPool, name: &str) -> Uuid {
        db::create_profile(pool, name).await.unwrap().id
    }

    #[tokio::test]
    async fn every_profile_gets_its_own_predefined_groups() {
        let pool = db::open_in_memory().await;
        assert!(list_groups(&pool, Uuid::new_v4()).await.unwrap().is_empty());
        let (a, b) = (kid(&pool, "A").await, kid(&pool, "B").await);
        let ga = list_groups(&pool, a).await.unwrap();
        assert_eq!(ga.iter().map(|g| (g.name.as_str(), g.builtin.as_deref())).collect::<Vec<_>>(),
            vec![("Games", Some("games")), ("Web browsers", Some("browsers"))]);
        assert!(common::apps::group_matches(&ga[0].rules(), "steam:570", &[]));
        assert!(common::apps::group_matches(&ga[0].rules(), "desktop:supertux", &["Game".into()]));
        assert!(common::apps::group_matches(&ga[1].rules(), "desktop:firefox", &["WebBrowser".into()]));
        let gb = list_groups(&pool, b).await.unwrap();
        assert_ne!(ga[0].id, gb[0].id);

        // seeding again adds nothing; a deleted predefined group stays deleted
        seed_builtin(&pool, a).await.unwrap();
        assert_eq!(list_groups(&pool, a).await.unwrap().len(), 2);
        assert!(delete_group(&pool, a, &ga[0].id).await.unwrap());
        assert_eq!(list_groups(&pool, a).await.unwrap().len(), 1);
        assert_eq!(list_groups(&pool, b).await.unwrap().len(), 2); // B untouched

        // another profile's group can't be touched
        assert!(!delete_group(&pool, a, &gb[0].id).await.unwrap());
        assert!(!rename_group(&pool, a, &gb[0].id, "x").await.unwrap());
        assert!(set_membership(&pool, a, &gb[0].id, "desktop:firefox", &[], true).await.unwrap().is_none());

        // deleting a profile deletes its groups
        db::delete_profile(&pool, b).await.unwrap();
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM app_groups WHERE profile_id = $1").bind(b.to_string()).fetch_one(&pool).await.unwrap();
        assert_eq!(n, 0);
    }

    #[tokio::test]
    async fn custom_groups_and_rules() {
        let pool = db::open_in_memory().await;
        let p = kid(&pool, "Kid").await;
        let id = create_group(&pool, p, "  School  ").await.unwrap();
        assert!(create_group(&pool, p, "   ").await.is_err());
        assert!(create_group(&pool, p, &"x".repeat(61)).await.is_err());
        let r1 = add_rule(&pool, &id, &GroupRule { kind: RuleKind::AppId, value: "desktop:libreoffice-writer".into(), exclude: false }).await.unwrap();
        assert_eq!(add_rule(&pool, &id, &GroupRule { kind: RuleKind::AppId, value: "desktop:libreoffice-writer".into(), exclude: false }).await.unwrap(), r1);
        for (kind, value) in [(RuleKind::AppId, "nonsense"), (RuleKind::Prefix, "steam"), (RuleKind::Prefix, "Steam:"), (RuleKind::Category, "Bad;Cat")] {
            assert!(add_rule(&pool, &id, &GroupRule { kind, value: value.into(), exclude: false }).await.is_err(), "{value}");
        }
        let g = get_group(&pool, p, &id).await.unwrap().unwrap();
        assert_eq!((g.name.as_str(), g.rules.len(), g.builtin), ("School", 1, None));
        // excluding the same app replaces the include rule
        add_rule(&pool, &id, &GroupRule { kind: RuleKind::AppId, value: "desktop:libreoffice-writer".into(), exclude: true }).await.unwrap();
        let g = get_group(&pool, p, &id).await.unwrap().unwrap();
        assert!(g.rules.len() == 1 && g.rules[0].rule.exclude);
        assert!(rename_group(&pool, p, &id, "Homework").await.unwrap());
        assert!(delete_rule(&pool, &id, &g.rules[0].id).await.unwrap());
        assert!(delete_group(&pool, p, &id).await.unwrap());
        assert!(!group_exists(&pool, p, &id).await.unwrap());
    }

    #[tokio::test]
    async fn per_profile_limits() {
        let pool = db::open_in_memory().await;
        let p = kid(&pool, "Kid").await;
        let ps = p.to_string();
        let games = list_groups(&pool, p).await.unwrap()[0].id.clone();
        replace_limits(&pool, &ps, &games, &[(0, 60), (5, 120), (6, 0)]).await.unwrap();
        assert_eq!(limits(&pool, &ps).await.unwrap()[&games], vec![(0, 60), (5, 120), (6, 0)]);
        replace_limits(&pool, &ps, &games, &[(0, 30)]).await.unwrap();
        assert_eq!(limits(&pool, &ps).await.unwrap()[&games], vec![(0, 30)]);
        for bad in [vec![(7, 10)], vec![(0, 1441)], vec![(0, -1)], vec![(1, 10), (1, 20)]] {
            assert!(replace_limits(&pool, &ps, &games, &bad).await.is_err(), "{bad:?}");
        }
        replace_limits(&pool, &ps, &games, &[]).await.unwrap();
        assert!(limits(&pool, &ps).await.unwrap().is_empty());
        replace_limits(&pool, &ps, &games, &[(2, 45)]).await.unwrap();
        delete_group(&pool, p, &games).await.unwrap();
        assert!(limits(&pool, &ps).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn adjustments_sum_per_day_and_reach_the_config() {
        let pool = db::open_in_memory().await;
        let (agent, au) = crate::app_usage::tests::agent_with_user(&pool, "m1", 1001).await;
        let p = kid(&pool, "Kid").await;
        db::update_agent_user(&pool, au, Some(p), Some("managed")).await.unwrap();
        let games = list_groups(&pool, p).await.unwrap()[0].id.clone();
        let today = chrono::Local::now().date_naive().to_string();
        add_adjustment(&pool, &p.to_string(), &games, &today, 15).await.unwrap();
        add_adjustment(&pool, &p.to_string(), &games, &today, 30).await.unwrap();
        add_adjustment(&pool, &p.to_string(), &games, "2020-01-01", 60).await.unwrap();
        assert!(add_adjustment(&pool, &p.to_string(), &games, &today, 0).await.is_err());
        assert_eq!(adjustments(&pool, &p.to_string(), &today).await.unwrap()[&games], 45);
        let push = crate::remaining::build_config_push(&pool, agent, 1).await.unwrap();
        let g = push.users[0].app_groups.iter().find(|g| g.id == games).unwrap();
        assert_eq!(g.adjustment_today, 45);
    }

    #[tokio::test]
    async fn config_push_carries_the_childs_groups_and_limits() {
        let pool = db::open_in_memory().await;
        let (agent, au) = crate::app_usage::tests::agent_with_user(&pool, "m1", 1001).await;
        let p = kid(&pool, "Kid").await;
        let other = kid(&pool, "Other").await;
        db::update_agent_user(&pool, au, Some(p), Some("managed")).await.unwrap();
        let games = list_groups(&pool, p).await.unwrap()[0].id.clone();
        replace_limits(&pool, &p.to_string(), &games, &[(5, 120)]).await.unwrap();
        let push = crate::remaining::build_config_push(&pool, agent, 7).await.unwrap();
        let groups = &push.users[0].app_groups;
        assert_eq!(groups.len(), 2);
        let g = groups.iter().find(|g| g.id == games).unwrap();
        assert_eq!((g.builtin.as_deref(), g.daily_limits[0].allowed_minutes), (Some("games"), 120));
        let other_ids: Vec<String> = list_groups(&pool, other).await.unwrap().into_iter().map(|g| g.id).collect();
        assert!(groups.iter().all(|g| !other_ids.contains(&g.id))); // only this child's groups
    }

    #[tokio::test]
    async fn membership_moves_between_the_childs_groups() {
        let pool = db::open_in_memory().await;
        let p = kid(&pool, "Kid").await;
        let groups = list_groups(&pool, p).await.unwrap();
        let browsers = groups[1].id.clone();
        let web = vec!["WebBrowser".to_string()];
        // in via its category; taking it out adds an exclusion, putting it back removes it
        set_membership(&pool, p, &browsers, "desktop:firefox", &web, false).await.unwrap().unwrap();
        assert_eq!(group_of(&list_groups(&pool, p).await.unwrap(), "desktop:firefox", &web), None);
        set_membership(&pool, p, &browsers, "desktop:firefox", &web, true).await.unwrap().unwrap();
        let g = list_groups(&pool, p).await.unwrap();
        assert_eq!(group_of(&g, "desktop:firefox", &web), Some(browsers.as_str()));
        assert_eq!(g[1].rules.len(), 1);
        // one group per app: into School, then back
        let school = create_group(&pool, p, "School").await.unwrap();
        set_membership(&pool, p, &school, "desktop:firefox", &web, true).await.unwrap().unwrap();
        assert_eq!(group_of(&list_groups(&pool, p).await.unwrap(), "desktop:firefox", &web), Some(school.as_str()));
        set_membership(&pool, p, &browsers, "desktop:firefox", &web, true).await.unwrap().unwrap();
        let g = list_groups(&pool, p).await.unwrap();
        assert_eq!(group_of(&g, "desktop:firefox", &web), Some(browsers.as_str()));
        assert!(g.iter().find(|x| x.id == school).unwrap().rules.is_empty());
        assert!(set_membership(&pool, p, &browsers, "bad id", &web, true).await.is_err());
    }

    #[tokio::test]
    async fn v16_gives_every_profile_a_copy_of_the_shared_groups() {
        let pool = db::open_in_memory().await;
        let (a, b) = (kid(&pool, "A").await, kid(&pool, "B").await);
        // back to v15: shared groups only, with a limit for A and recorded time
        delete_profile_groups(&pool, a).await.unwrap();
        delete_profile_groups(&pool, b).await.unwrap();
        let shared = Uuid::new_v4().to_string();
        sqlx::query("INSERT INTO app_groups (id, profile_id, name, builtin, created_at) VALUES ($1, NULL, 'Games', 'games', 1)").bind(&shared).execute(&pool).await.unwrap();
        add_rule(&pool, &shared, &GroupRule { kind: RuleKind::Category, value: "Game".into(), exclude: false }).await.unwrap();
        add_rule(&pool, &shared, &GroupRule { kind: RuleKind::AppId, value: "desktop:firefox".into(), exclude: true }).await.unwrap();
        replace_limits(&pool, &a.to_string(), &shared, &[(0, 60)]).await.unwrap();

        split_shared_groups(&pool).await.unwrap();

        let ga = list_groups(&pool, a).await.unwrap();
        let gb = list_groups(&pool, b).await.unwrap();
        assert_eq!((ga.len(), gb.len()), (1, 1));
        assert_ne!(ga[0].id, gb[0].id);
        assert_eq!(ga[0].rules.len(), 2); // rules copied, exclusion included
        assert_eq!(limits(&pool, &a.to_string()).await.unwrap()[&ga[0].id], vec![(0, 60)]);
        assert!(limits(&pool, &b.to_string()).await.unwrap().is_empty());
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM app_groups WHERE profile_id IS NULL").fetch_one(&pool).await.unwrap();
        assert_eq!(n, 0);
    }
}
