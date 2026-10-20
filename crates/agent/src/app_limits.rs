//! Remaining time per app group (#27 Phase 2, step 2.8). Computed on the agent
//! from the config's limits, this machine's live group time and the group time
//! the server last reported for the child's other computers — so the same code
//! works online and offline.

use std::collections::HashMap;

use common::models::AppGroupConfig;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupStatus {
    pub group_id: String,
    /// Display name in the child's language.
    pub name: String,
    pub limit_minutes: u32,
    pub used_seconds: u64,
    /// May go negative when the limit is exceeded.
    pub remaining_seconds: i64,
}

impl GroupStatus {
    pub fn blocked(&self) -> bool {
        self.limit_minutes == 0
    }

    pub fn exhausted(&self) -> bool {
        self.remaining_seconds <= 0
    }
}

/// Status of every group that has a limit on `weekday` (0 = Monday).
/// `adjustments` are extra minutes per group for today (may be negative).
pub fn group_status(
    groups: &[AppGroupConfig],
    weekday: u8,
    local_used: &HashMap<String, u64>,
    elsewhere: &HashMap<String, u64>,
    adjustments: &HashMap<String, i32>,
    lang: &str,
) -> Vec<GroupStatus> {
    groups
        .iter()
        .filter_map(|g| {
            let limit = g.daily_limits.iter().find(|l| l.day_of_week == weekday)?.allowed_minutes;
            let used = local_used.get(&g.id).copied().unwrap_or(0) + elsewhere.get(&g.id).copied().unwrap_or(0);
            // a blocked day stays blocked: extra time is for groups with time
            let allowed = if limit == 0 { 0 } else { (limit as i64 + adjustments.get(&g.id).copied().unwrap_or(0) as i64).max(0) };
            Some(GroupStatus {
                group_id: g.id.clone(),
                name: crate::i18n::group_name(g.builtin.as_deref(), &g.name, lang),
                limit_minutes: limit,
                used_seconds: used,
                remaining_seconds: allowed * 60 - used as i64,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::apps::{GroupRule, RuleKind};
    use common::models::DailyLimit;

    fn group(id: &str, builtin: Option<&str>, limits: &[(u8, u32)]) -> AppGroupConfig {
        AppGroupConfig {
            id: id.into(),
            name: if builtin == Some("games") { "Games".into() } else { id.into() },
            builtin: builtin.map(str::to_string),
            rules: vec![GroupRule { kind: RuleKind::Category, value: "Game".into(), exclude: false }],
            daily_limits: limits.iter().map(|&(day_of_week, allowed_minutes)| DailyLimit { day_of_week, allowed_minutes }).collect(),
        }
    }

    fn map(v: &[(&str, u64)]) -> HashMap<String, u64> {
        v.iter().map(|(k, s)| (k.to_string(), *s)).collect()
    }

    #[test]
    fn remaining_combines_local_and_elsewhere() {
        let groups = [group("games", Some("games"), &[(0, 60), (6, 0)]), group("school", None, &[]), group("web", None, &[(1, 30)])];
        // Monday: games limited to 60 min, school no limit, web not limited on Mondays
        let s = group_status(&groups, 0, &map(&[("games", 1200)]), &map(&[("games", 600)]), &HashMap::new(), "pl");
        assert_eq!(s.len(), 1);
        assert_eq!((s[0].name.as_str(), s[0].used_seconds, s[0].remaining_seconds), ("Gry", 1800, 1800));
        assert!(!s[0].exhausted());

        // over the limit → negative; offline (no elsewhere) still counts local time
        let s = group_status(&groups, 0, &map(&[("games", 4000)]), &HashMap::new(), &HashMap::new(), "en");
        assert_eq!((s[0].name.as_str(), s[0].remaining_seconds), ("Games", -400));
        assert!(s[0].exhausted());

        // Sunday is blocked, even with extra time
        let adj: HashMap<String, i32> = [("games".to_string(), 30)].into();
        let s = group_status(&groups, 6, &HashMap::new(), &HashMap::new(), &adj, "en");
        assert!(s[0].blocked() && s[0].exhausted());

        // extra time on a normal day; negative adjustments can't go below 0
        let s = group_status(&groups, 0, &map(&[("games", 3600)]), &HashMap::new(), &adj, "en");
        assert_eq!(s[0].remaining_seconds, 1800);
        let less: HashMap<String, i32> = [("games".to_string(), -120)].into();
        let s = group_status(&groups, 0, &HashMap::new(), &HashMap::new(), &less, "en");
        assert_eq!(s[0].remaining_seconds, 0);
    }
}
