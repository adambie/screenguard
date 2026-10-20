//! Per-app usage accounting (docs/feature-app-usage-limits.md, step 1.8).
//!
//! Every scan tick, each app with at least one running process gets the tick's
//! seconds once — but only while the user's session counts as usage (active,
//! unlocked, not idle: the same rule as screen time).


use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use common::apps::AppId;

use crate::app_catalog::CatalogCache;
use crate::app_ident::{self, Launch};
use crate::app_scan::ProcScanner;

/// Scan interval chosen in Phase 0 (step 0.6).
pub const SCAN_INTERVAL: Duration = Duration::from_secs(5);

/// Turns wall-clock ticks into whole seconds per (uid, app).
pub struct AppAccounting {
    interval: Duration,
    last_tick: Option<Instant>,
    /// Sub-second remainder per uid, so 5.3 s ticks don't lose 0.3 s each time.
    carry_ms: HashMap<u32, u64>,
}

impl AppAccounting {
    pub fn new(interval: Duration) -> Self {
        Self { interval, last_tick: None, carry_ms: HashMap::new() }
    }

    /// Whole seconds per user for this tick, for users that count and have
    /// something running (`active`). The first tick only starts the clock; a gap
    /// longer than 2× the interval (suspend, clock jump, stalled loop) is capped.
    pub fn seconds(&mut self, now: Instant, counting: &HashSet<u32>, active: &HashSet<u32>) -> HashMap<u32, u64> {
        let Some(last) = self.last_tick.replace(now) else {
            return HashMap::new();
        };
        let elapsed_ms = now.saturating_duration_since(last).min(self.interval * 2).as_millis() as u64;
        self.carry_ms.retain(|uid, _| counting.contains(uid));
        let mut out = HashMap::new();
        for &uid in active {
            if !counting.contains(&uid) {
                continue;
            }
            let carry = self.carry_ms.entry(uid).or_insert(0);
            let total = *carry + elapsed_ms;
            *carry = total % 1000;
            if total >= 1000 {
                out.insert(uid, total / 1000);
            }
        }
        out
    }

    /// Seconds to add per (uid, app) for this tick: each running app gets the
    /// user's seconds once.
    #[cfg(test)]
    pub fn tick(
        &mut self,
        now: Instant,
        counting: &HashSet<u32>,
        running: &HashMap<u32, BTreeSet<AppId>>,
    ) -> Vec<(u32, AppId, u64)> {
        let active: HashSet<u32> = running.iter().filter(|(_, a)| !a.is_empty()).map(|(u, _)| *u).collect();
        let secs = self.seconds(now, counting, &active);
        running
            .iter()
            .filter_map(|(uid, apps)| secs.get(uid).map(|s| (uid, apps, *s)))
            .flat_map(|(uid, apps, s)| apps.iter().map(move |a| (*uid, a.clone(), s)))
            .collect()
    }
}

/// What one tick produced: seconds to add, plus name/icon for each app seen.
#[derive(Debug, Default)]
pub struct TickResult {
    pub add: Vec<(u32, AppId, u64)>,
    /// (uid, group id, seconds): a group gets the tick once if any member runs,
    /// so two browsers open at once don't count double.
    pub add_groups: Vec<(u32, String, u64)>,
    /// Counted apps running right now, per uid.
    pub running: HashMap<u32, BTreeSet<AppId>>,
    /// Every process of an app (counted or autostarted) — what app limits can close.
    pub procs: Vec<crate::app_enforce::RunningProc>,
    /// (uid, app) → name, icon and categories, in the user's profile language.
    pub names: HashMap<(u32, AppId), app_ident::Shown>,
}

/// Scanner + catalogue + identification + accounting.
pub struct AppTracker {
    scanner: ProcScanner,
    catalogs: CatalogCache,
    accounting: AppAccounting,
}

/// A managed user as the tracker needs it.
pub struct TrackedUser {
    pub uid: u32,
    pub home: PathBuf,
    pub language: String,
    /// Session counts as usage right now.
    pub counting: bool,
    /// App groups from the profile's config.
    pub groups: Vec<common::models::AppGroupConfig>,
}

impl AppTracker {
    pub fn new() -> Self {
        Self { scanner: ProcScanner::new(), catalogs: CatalogCache::new(), accounting: AppAccounting::new(SCAN_INTERVAL) }
    }

    pub fn tick(&mut self, now: Instant, users: &[TrackedUser]) -> TickResult {
        let uids: HashSet<u32> = users.iter().map(|u| u.uid).collect();
        let counting: HashSet<u32> = users.iter().filter(|u| u.counting).map(|u| u.uid).collect();
        let procs = self.scanner.scan(&uids);

        let mut running: HashMap<u32, BTreeSet<AppId>> = HashMap::new();
        let mut result = TickResult::default();
        for user in users {
            let mine: HashMap<u32, _> = procs.iter().filter(|(_, p)| p.uid == user.uid).map(|(k, v)| (*k, v.clone())).collect();
            if mine.is_empty() {
                continue;
            }
            let catalog = self.catalogs.get(&user.home);
            for (pid, id) in app_ident::identify_all(&mine, catalog) {
                if id.launch == Launch::Other {
                    continue;
                }
                result
                    .names
                    .entry((user.uid, id.app_id.clone()))
                    .or_insert_with(|| app_ident::display(&id, catalog, &user.language, &user.home));
                let p = &mine[&pid];
                result.procs.push(crate::app_enforce::RunningProc {
                    pid,
                    start_ticks: p.start_ticks,
                    uid: p.uid,
                    app_id: id.app_id.clone(),
                    comm: p.comm.clone(),
                    exe: p.exe.clone(),
                });
                if id.launch == Launch::App {
                    // autostarted apps are not counted (decision log)
                    running.entry(user.uid).or_default().insert(id.app_id);
                }
            }
        }
        let active: HashSet<u32> = running.keys().copied().collect();
        let secs = self.accounting.seconds(now, &counting, &active);
        for user in users {
            let (Some(&s), Some(apps)) = (secs.get(&user.uid), running.get(&user.uid)) else { continue };
            result.add.extend(apps.iter().map(|a| (user.uid, a.clone(), s)));
            for g in &user.groups {
                let member_running = apps.iter().any(|a| {
                    let cats = result.names.get(&(user.uid, a.clone())).map(|n| n.categories.as_slice()).unwrap_or(&[]);
                    common::apps::group_matches(&g.rules, &a.to_string(), cats)
                });
                if member_running {
                    result.add_groups.push((user.uid, g.id.clone(), s));
                }
            }
        }
        result.running = running;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apps(ids: &[&str]) -> BTreeSet<AppId> {
        ids.iter().map(|s| AppId::parse(s).unwrap()).collect()
    }

    /// (ms since t0, counting uids, running apps)
    type Tick<'a> = (u64, &'a [u32], &'a HashMap<u32, BTreeSet<AppId>>);

    fn totals(acc: &mut AppAccounting, t0: Instant, ticks: &[Tick]) -> HashMap<(u32, String), u64> {
        let mut out = HashMap::new();
        for (ms, counting, running) in ticks {
            let counting: HashSet<u32> = counting.iter().copied().collect();
            for (uid, app, secs) in acc.tick(t0 + Duration::from_millis(*ms), &counting, running) {
                *out.entry((uid, app.to_string())).or_insert(0) += secs;
            }
        }
        out
    }

    #[test]
    fn exact_totals_with_fake_clock() {
        let t0 = Instant::now();
        let both = HashMap::from([(1000, apps(&["desktop:firefox", "steam:570"])), (1001, apps(&["desktop:foot"]))]);
        let only_ff = HashMap::from([(1000, apps(&["desktop:firefox"]))]);
        let mut acc = AppAccounting::new(Duration::from_secs(5));
        let t = totals(&mut acc, t0, &[
            (0, &[1000, 1001], &both),       // starts the clock
            (5_000, &[1000, 1001], &both),   // +5 each
            (10_300, &[1000, 1001], &both),  // +5, carry 300 ms
            (15_600, &[1000], &both),        // 1001 not counting (locked/idle): +5 (carry 600)
            (20_900, &[1000], &only_ff),     // steam closed: firefox +5 (carry 900 → 5.3 s = +5, carry 200)
        ]);
        assert_eq!(t[&(1000, "desktop:firefox".into())], 20);
        assert_eq!(t[&(1000, "steam:570".into())], 15);
        assert_eq!(t[&(1001, "desktop:foot".into())], 10);
    }

    #[test]
    fn carry_adds_up_over_many_ticks() {
        let t0 = Instant::now();
        let running = HashMap::from([(1000, apps(&["desktop:foot"]))]);
        let mut acc = AppAccounting::new(Duration::from_secs(5));
        let ticks: Vec<Tick> =
            (0..=100).map(|i| (i * 5_010, &[1000u32][..], &running)).collect();
        // 100 ticks of 5.01 s = 501 s
        assert_eq!(totals(&mut acc, t0, &ticks)[&(1000, "desktop:foot".into())], 501);
    }

    #[test]
    fn gaps_are_capped_at_two_intervals() {
        let t0 = Instant::now();
        let running = HashMap::from([(1000, apps(&["desktop:foot"]))]);
        let mut acc = AppAccounting::new(Duration::from_secs(5));
        let t = totals(&mut acc, t0, &[(0, &[1000], &running), (3_600_000, &[1000], &running), (3_605_000, &[1000], &running)]);
        assert_eq!(t[&(1000, "desktop:foot".into())], 15); // 10 (capped) + 5
    }

    #[test]
    fn no_time_when_not_counting_or_nothing_runs() {
        let t0 = Instant::now();
        let running = HashMap::from([(1000, apps(&["desktop:foot"]))]);
        let mut acc = AppAccounting::new(Duration::from_secs(5));
        let t = totals(&mut acc, t0, &[(0, &[], &running), (5_000, &[], &running), (10_000, &[1000], &HashMap::new())]);
        assert!(t.is_empty());
    }

    #[test]
    fn tracker_counts_apps_once_and_skips_autostart() {
        use crate::app_catalog::fixture;
        use crate::app_scan::fake_proc::{FakeProc, P};
        let fp = FakeProc::new();
        let u = "/user.slice/user-1000.slice/user@1000.service/app.slice";
        let (ff, nc) = (format!("{u}/app-gnome-firefox-1.scope"), format!("{u}/app-nextcloud@autostart.service"));
        fp.add(P { pid: 900, ppid: 1, start: 100, comm: "systemd", argv: &["systemd", "--user"], ..Default::default() })
            .add(P { pid: 10, ppid: 900, start: 9_000, comm: "firefox", exe: Some("/usr/lib/firefox/firefox"), argv: &["firefox"], cgroup: &ff, ..Default::default() })
            .add(P { pid: 11, ppid: 10, start: 9_001, comm: "Web Content", exe: Some("/usr/lib/firefox/firefox"), argv: &["firefox", "-contentproc"], cgroup: &ff, ..Default::default() })
            .add(P { pid: 13, ppid: 900, start: 9_000, comm: "chromium", exe: Some("/usr/bin/chromium"), argv: &["chromium"], cgroup: &format!("{u}/app-gnome-chromium-2.scope"), ..Default::default() })
            .add(P { pid: 12, ppid: 900, start: 200, comm: "nextcloud", exe: Some("/usr/bin/nextcloud"), argv: &["nextcloud"], cgroup: &nc, ..Default::default() });
        let home = fp.path().join("home");
        fixture::standard(&home.join(".local/share/applications"), &home.join(".config/autostart"));

        let mut tracker = AppTracker::new();
        tracker.scanner = ProcScanner::with_root(fp.path());
        use common::apps::{GroupRule, RuleKind};
        let group = |id: &str, kind, value: &str| common::models::AppGroupConfig {
            id: id.into(),
            name: id.into(),
            builtin: None,
            rules: vec![GroupRule { kind, value: value.into(), exclude: false }],
            daily_limits: vec![],
            adjustment_today: 0,
        };
        let groups = vec![
            group("browsers", RuleKind::Category, "WebBrowser"),
            group("games", RuleKind::Category, "Game"),
            group("all-desktop", RuleKind::Prefix, "desktop:"),
        ];
        let users = [TrackedUser { uid: 1000, home, language: "en".into(), counting: true, groups }];
        let t0 = Instant::now();
        assert!(tracker.tick(t0, &users).add.is_empty());
        let r = tracker.tick(t0 + SCAN_INTERVAL, &users);
        let mut add = r.add.clone();
        add.sort();
        assert_eq!(add, vec![(1000, AppId::parse("desktop:chromium").unwrap(), 5), (1000, AppId::parse("desktop:firefox").unwrap(), 5)]);
        // a group counts the tick once, however many of its apps run
        let mut g = r.add_groups.clone();
        g.sort();
        assert_eq!(g, vec![(1000, "all-desktop".to_string(), 5), (1000, "browsers".to_string(), 5)]);
        assert!(r.running[&1000].contains(&AppId::parse("desktop:firefox").unwrap()));
        let shown = &r.names[&(1000, AppId::parse("desktop:firefox").unwrap())];
        assert_eq!((shown.name.as_str(), shown.icon.as_deref()), ("Firefox", Some("firefox")));
        assert_eq!(shown.categories, vec!["Network", "WebBrowser"]);
    }
}
