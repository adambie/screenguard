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

    /// Seconds to add for this tick. The first tick only starts the clock; a gap
    /// longer than 2× the interval (suspend, clock jump, stalled loop) is capped.
    pub fn tick(
        &mut self,
        now: Instant,
        counting: &HashSet<u32>,
        running: &HashMap<u32, BTreeSet<AppId>>,
    ) -> Vec<(u32, AppId, u64)> {
        let Some(last) = self.last_tick.replace(now) else {
            return Vec::new();
        };
        let elapsed_ms = now.saturating_duration_since(last).min(self.interval * 2).as_millis() as u64;
        self.carry_ms.retain(|uid, _| counting.contains(uid));
        let mut out = Vec::new();
        for (&uid, apps) in running {
            if !counting.contains(&uid) || apps.is_empty() {
                continue;
            }
            let carry = self.carry_ms.entry(uid).or_insert(0);
            let total = *carry + elapsed_ms;
            let secs = total / 1000;
            *carry = total % 1000;
            if secs > 0 {
                out.extend(apps.iter().map(|a| (uid, a.clone(), secs)));
            }
        }
        out
    }
}

/// What one tick produced: seconds to add, plus name/icon for each app seen.
#[derive(Debug, Default)]
pub struct TickResult {
    pub add: Vec<(u32, AppId, u64)>,
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
            for id in app_ident::identify_all(&mine, catalog).into_values() {
                if id.launch != Launch::App {
                    continue; // autostart and "other" are not counted (decision log)
                }
                result
                    .names
                    .entry((user.uid, id.app_id.clone()))
                    .or_insert_with(|| app_ident::display(&id, catalog, &user.language, &user.home));
                running.entry(user.uid).or_default().insert(id.app_id);
            }
        }
        result.add = self.accounting.tick(now, &counting, &running);
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
            .add(P { pid: 12, ppid: 900, start: 200, comm: "nextcloud", exe: Some("/usr/bin/nextcloud"), argv: &["nextcloud"], cgroup: &nc, ..Default::default() });
        let home = fp.path().join("home");
        fixture::standard(&home.join(".local/share/applications"), &home.join(".config/autostart"));

        let mut tracker = AppTracker::new();
        tracker.scanner = ProcScanner::with_root(fp.path());
        let users = [TrackedUser { uid: 1000, home, language: "en".into(), counting: true }];
        let t0 = Instant::now();
        assert!(tracker.tick(t0, &users).add.is_empty());
        let r = tracker.tick(t0 + SCAN_INTERVAL, &users);
        assert_eq!(r.add, vec![(1000, AppId::parse("desktop:firefox").unwrap(), 5)]);
        let shown = &r.names[&(1000, AppId::parse("desktop:firefox").unwrap())];
        assert_eq!((shown.name.as_str(), shown.icon.as_deref()), ("Firefox", Some("firefox")));
        assert_eq!(shown.categories, vec!["Network", "WebBrowser"]);
    }
}
