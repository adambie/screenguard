//! App groups (#27 Phase 2): global groups of apps defined by rules, with
//! per-profile daily limits. Predefined: "Games" and "Web browsers".

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

/// Creates the predefined groups that don't exist yet. Called once: by
/// migration v12, or on a fresh install — a group the parent deleted stays gone.
pub async fn seed_builtin(pool: &DbPool) -> Result<()> {
    for (key, name, rules) in BUILTIN {
        let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM app_groups WHERE builtin = $1")
            .bind(*key)
            .fetch_one(pool)
            .await?;
        if exists > 0 {
            continue;
        }
        let id = create_group_with(pool, name, Some(key)).await?;
        for (kind, value) in *rules {
            add_rule(pool, &id, &GroupRule { kind: *kind, value: value.to_string(), exclude: false }).await?;
        }
    }
    Ok(())
}

async fn create_group_with(pool: &DbPool, name: &str, builtin: Option<&str>) -> Result<String> {
    let id = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO app_groups (id, name, builtin, created_at) VALUES ($1,$2,$3,$4)")
        .bind(&id)
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

pub async fn create_group(pool: &DbPool, name: &str) -> Result<String> {
    create_group_with(pool, &clean_name(name)?, None).await
}

/// `false` if there is no such group.
pub async fn rename_group(pool: &DbPool, id: &str, name: &str) -> Result<bool> {
    let r = sqlx::query("UPDATE app_groups SET name = $1 WHERE id = $2").bind(clean_name(name)?).bind(id).execute(pool).await?;
    Ok(r.rows_affected() > 0)
}

pub async fn delete_group(pool: &DbPool, id: &str) -> Result<bool> {
    // explicit, so it doesn't depend on foreign keys being enabled
    sqlx::query("DELETE FROM app_group_rules WHERE group_id = $1").bind(id).execute(pool).await?;
    sqlx::query("DELETE FROM app_group_limits WHERE group_id = $1").bind(id).execute(pool).await?;
    let r = sqlx::query("DELETE FROM app_groups WHERE id = $1").bind(id).execute(pool).await?;
    Ok(r.rows_affected() > 0)
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

/// All groups with their rules: predefined first, then by name.
pub async fn list_groups(pool: &DbPool) -> Result<Vec<Group>> {
    let groups: Vec<(String, String, Option<String>)> =
        sqlx::query_as("SELECT id, name, builtin FROM app_groups").fetch_all(pool).await?;
    let rules: Vec<(String, String, String, String, i32)> =
        sqlx::query_as("SELECT id, group_id, kind, value, exclude FROM app_group_rules ORDER BY kind, value")
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
pub async fn set_membership(pool: &DbPool, group_id: &str, app_id: &str, categories: &[String], member: bool) -> Result<Option<()>> {
    if common::apps::AppId::parse(app_id).is_none() {
        bail!("invalid app id {app_id:?}");
    }
    let Some(group) = get_group(pool, group_id).await? else {
        return Ok(None);
    };
    // drop the explicit rule that says the opposite of what's wanted
    if let Some(r) = group.rules.iter().find(|r| r.rule.kind == RuleKind::AppId && r.rule.value == app_id && r.rule.exclude == member) {
        delete_rule(pool, group_id, &r.id).await?;
    }
    let rules = get_group(pool, group_id).await?.map(|g| g.rules()).unwrap_or_default();
    if common::apps::group_matches(&rules, app_id, categories) != member {
        add_rule(pool, group_id, &GroupRule { kind: RuleKind::AppId, value: app_id.into(), exclude: !member }).await?;
    }
    Ok(Some(()))
}

pub async fn get_group(pool: &DbPool, id: &str) -> Result<Option<Group>> {
    Ok(list_groups(pool).await?.into_iter().find(|g| g.id == id))
}

pub async fn group_exists(pool: &DbPool, id: &str) -> Result<bool> {
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM app_groups WHERE id = $1").bind(id).fetch_one(pool).await?;
    Ok(n > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    #[tokio::test]
    async fn fresh_install_has_the_predefined_groups_once() {
        let pool = db::open_in_memory().await;
        let groups = list_groups(&pool).await.unwrap();
        assert_eq!(groups.iter().map(|g| (g.name.as_str(), g.builtin.as_deref())).collect::<Vec<_>>(),
            vec![("Games", Some("games")), ("Web browsers", Some("browsers"))]);
        let games = &groups[0];
        assert!(common::apps::group_matches(&games.rules(), "steam:570", &[]));
        assert!(common::apps::group_matches(&games.rules(), "desktop:supertux", &["Game".into()]));
        assert!(common::apps::group_matches(&groups[1].rules(), "desktop:firefox", &["WebBrowser".into()]));

        // seeding again adds nothing; a deleted predefined group stays deleted
        seed_builtin(&pool).await.unwrap();
        assert_eq!(list_groups(&pool).await.unwrap().len(), 2);
        assert!(delete_group(&pool, &games.id).await.unwrap());
        // (migrations don't re-run seeding; seed_builtin is only called by v12 / fresh install)
        assert_eq!(list_groups(&pool).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn custom_groups_and_rules() {
        let pool = db::open_in_memory().await;
        let id = create_group(&pool, "  School  ").await.unwrap();
        assert!(create_group(&pool, "   ").await.is_err());
        assert!(create_group(&pool, &"x".repeat(61)).await.is_err());
        let r1 = add_rule(&pool, &id, &GroupRule { kind: RuleKind::AppId, value: "desktop:libreoffice-writer".into(), exclude: false }).await.unwrap();
        // duplicate: same id back
        assert_eq!(add_rule(&pool, &id, &GroupRule { kind: RuleKind::AppId, value: "desktop:libreoffice-writer".into(), exclude: false }).await.unwrap(), r1);
        // invalid rules
        for (kind, value) in [(RuleKind::AppId, "nonsense"), (RuleKind::Prefix, "steam"), (RuleKind::Prefix, "Steam:"), (RuleKind::Category, "Bad;Cat")] {
            assert!(add_rule(&pool, &id, &GroupRule { kind, value: value.into(), exclude: false }).await.is_err(), "{value}");
        }
        let g = list_groups(&pool).await.unwrap().into_iter().find(|g| g.id == id).unwrap();
        assert_eq!((g.name.as_str(), g.rules.len(), g.builtin), ("School", 1, None));

        // excluding the same app replaces the include rule
        add_rule(&pool, &id, &GroupRule { kind: RuleKind::AppId, value: "desktop:libreoffice-writer".into(), exclude: true }).await.unwrap();
        let g = list_groups(&pool).await.unwrap().into_iter().find(|g| g.id == id).unwrap();
        assert_eq!(g.rules.len(), 1);
        assert!(g.rules[0].rule.exclude);

        assert!(rename_group(&pool, &id, "Homework").await.unwrap());
        assert!(!rename_group(&pool, "nope", "x").await.unwrap());
        assert!(delete_rule(&pool, &id, &g.rules[0].id).await.unwrap());
        assert!(delete_group(&pool, &id).await.unwrap());
        assert!(!group_exists(&pool, &id).await.unwrap());
    }

    #[tokio::test]
    async fn membership_toggles_through_entries_and_exclusions() {
        let pool = db::open_in_memory().await;
        let browsers = list_groups(&pool).await.unwrap()[1].clone();
        let web = vec!["WebBrowser".to_string()];
        let rules = |pool: DbPool, id: String| async move {
            list_groups(&pool).await.unwrap().into_iter().find(|g| g.id == id).unwrap().rules()
        };
        let is_in = |rules: &[GroupRule], app: &str, cats: &[String]| common::apps::group_matches(rules, app, cats);

        // Firefox is in via its category: taking it out adds an exclusion
        set_membership(&pool, &browsers.id, "desktop:firefox", &web, false).await.unwrap().unwrap();
        let r = rules(pool.clone(), browsers.id.clone()).await;
        assert!(!is_in(&r, "desktop:firefox", &web));
        assert_eq!(r.len(), 2);
        // putting it back removes the exclusion (no extra entry needed)
        set_membership(&pool, &browsers.id, "desktop:firefox", &web, true).await.unwrap().unwrap();
        let r = rules(pool.clone(), browsers.id.clone()).await;
        assert!(is_in(&r, "desktop:firefox", &web));
        assert_eq!(r.len(), 1);

        // an app without the category gets an explicit entry, removing deletes it
        set_membership(&pool, &browsers.id, "flatpak:io.github.tor", &[], true).await.unwrap().unwrap();
        assert!(is_in(&rules(pool.clone(), browsers.id.clone()).await, "flatpak:io.github.tor", &[]));
        set_membership(&pool, &browsers.id, "flatpak:io.github.tor", &[], false).await.unwrap().unwrap();
        let r = rules(pool.clone(), browsers.id.clone()).await;
        assert!(!is_in(&r, "flatpak:io.github.tor", &[]));
        assert_eq!(r.len(), 1);

        assert!(set_membership(&pool, "nope", "desktop:firefox", &web, true).await.unwrap().is_none());
        assert!(set_membership(&pool, &browsers.id, "bad id", &web, true).await.is_err());
    }
}
