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

/// Warning thresholds to fire now, as (group id, minutes left). Only for groups
/// in use right now (`in_use`), once per threshold per day (`notified` keeps
/// what fired); a threshold re-arms when the remaining time goes back above it
/// (extra time). Exhausted groups get no warning — they get closed instead.
pub fn due_warnings(
    status: &[GroupStatus],
    in_use: &std::collections::HashSet<String>,
    thresholds: &[u32],
    notified: &mut HashMap<String, std::collections::HashSet<u32>>,
) -> Vec<(String, i32)> {
    let mut out = Vec::new();
    for g in status {
        let left = ((g.remaining_seconds.max(0) + 59) / 60) as i32;
        let fired = notified.entry(g.group_id.clone()).or_default();
        fired.retain(|&t| left <= t as i32);
        if g.exhausted() || g.blocked() || !in_use.contains(&g.group_id) {
            continue;
        }
        // only the tightest threshold crossed, so a late start doesn't fire 15, 5 and 1 at once
        let crossed: Vec<u32> = thresholds.iter().copied().filter(|&t| left <= t as i32 && !fired.contains(&t)).collect();
        if !crossed.is_empty() {
            fired.extend(crossed);
            out.push((g.group_id.clone(), left));
        }
    }
    out
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

    #[test]
    fn warnings_fire_once_per_threshold_while_in_use() {
        let st = |secs: i64| vec![GroupStatus { group_id: "games".into(), name: "Games".into(), limit_minutes: 60, used_seconds: 0, remaining_seconds: secs }];
        let in_use: std::collections::HashSet<String> = ["games".to_string()].into();
        let idle = std::collections::HashSet::new();
        let th = [15, 5, 1];
        let mut n = HashMap::new();
        assert!(due_warnings(&st(20 * 60), &in_use, &th, &mut n).is_empty());
        assert_eq!(due_warnings(&st(14 * 60 + 1), &in_use, &th, &mut n), vec![("games".to_string(), 15)]);
        assert!(due_warnings(&st(14 * 60), &in_use, &th, &mut n).is_empty()); // once
        assert!(due_warnings(&st(4 * 60), &idle, &th, &mut n).is_empty()); // not in use: no warning
        assert_eq!(due_warnings(&st(4 * 60), &in_use, &th, &mut n), vec![("games".to_string(), 4)]);
        // a late start just below 1 min fires a single warning, not three
        let mut fresh = HashMap::new();
        assert_eq!(due_warnings(&st(50), &in_use, &th, &mut fresh), vec![("games".to_string(), 1)]);
        assert!(due_warnings(&st(30), &in_use, &th, &mut fresh).is_empty());
        // extra time re-arms the 5 min threshold
        assert!(due_warnings(&st(20 * 60), &in_use, &th, &mut n).is_empty());
        assert_eq!(due_warnings(&st(5 * 60), &in_use, &th, &mut n), vec![("games".to_string(), 5)]);
        // exhausted: no warning
        assert!(due_warnings(&st(0), &in_use, &th, &mut HashMap::new()).is_empty());
    }
}
