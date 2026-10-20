//! Closing apps whose group is out of time or blocked (#27 Phase 2, steps
//! 2.10–2.12).
//!
//! Safety rules (docs/feature-app-usage-limits.md, "Enforcement"): only
//! processes of a managed child's UID, only apps that matched a limited group
//! (never the "other" bucket: session, services, desktop components), never the
//! agent or tray. Signals go to the exact processes identified as the app — not
//! to a whole cgroup, which may hold unrelated processes (e.g. a terminal's scope)
//! — and each process's start time is re-checked first so a reused PID is never hit.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use common::apps::AppId;
use common::models::{AppApprovalConfig, AppGroupConfig};

use crate::app_ident::Shown;
use crate::app_limits::GroupStatus;

/// Between "will close in N seconds" and SIGTERM, and again before SIGKILL.
pub const GRACE: Duration = Duration::from_secs(10);
/// A relaunch within this time is closed at once, with at most one notification.
pub const RELAUNCH_WINDOW: Duration = Duration::from_secs(60);

/// A running process that belongs to an app (counted or autostarted).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningProc {
    pub pid: u32,
    pub ppid: u32,
    /// Started with the session (XDG autostart): not subject to new-app approval.
    pub autostart: bool,
    pub start_ticks: u64,
    pub uid: u32,
    pub app_id: AppId,
    pub comm: String,
    pub exe: Option<String>,
}

/// Why an app gets closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// Its group used up today's time: warn, then close.
    Limit,
    /// Its group is blocked today (0 minutes): close at once.
    Blocked,
    /// New-app approval: not approved yet — close at once and ask the parents.
    NeedsApproval,
    /// New-app approval: a parent said no.
    Denied,
}

impl Reason {
    fn immediate(self) -> bool {
        self != Reason::Limit
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub pid: u32,
    pub start_ticks: u64,
    /// The app's top process (its parent isn't the same app): the only one that
    /// gets SIGTERM, so the app can close its own helpers cleanly.
    pub top: bool,
    pub uid: u32,
    pub app_id: AppId,
    /// Group (limits) or the app id (approval) — notices are rate-limited per key.
    pub key: String,
    pub group_name: String,
    pub app_name: String,
    pub reason: Reason,
}

/// Processes to close: apps not approved (when new-app approval is on), and apps
/// in a group that is exhausted or blocked today.
pub fn targets(
    procs: &[RunningProc],
    status: &HashMap<u32, Vec<GroupStatus>>,
    groups: &HashMap<u32, Vec<AppGroupConfig>>,
    approval: &HashMap<u32, AppApprovalConfig>,
    names: &HashMap<(u32, AppId), Shown>,
    managed: &HashSet<u32>,
) -> Vec<Target> {
    let mut out = Vec::new();
    for p in procs {
        // (a) a managed child's process, never root or a system account
        if p.uid < 1000 || !managed.contains(&p.uid) {
            continue;
        }
        // never ScreenGuard itself
        let own = |s: &str| s.contains("screenguard");
        if own(&p.comm) || p.exe.as_deref().is_some_and(own) {
            continue;
        }
        let shown = names.get(&(p.uid, p.app_id.clone()));
        let app_name = shown.map_or(p.app_id.name.clone(), |s| s.name.clone());
        let id = p.app_id.to_string();
        // autostarted tray apps were there before the mode was on; their usage
        // is never counted, so they would never be "already used" — leave them
        // to group limits only
        if let Some(a) = approval.get(&p.uid).filter(|a| !p.autostart && !a.approved.contains(&id)) {
            let reason = if a.denied.contains(&id) { Reason::Denied } else { Reason::NeedsApproval };
            out.push(Target { pid: p.pid, start_ticks: p.start_ticks, top: true, uid: p.uid, app_id: p.app_id.clone(), key: id, group_name: String::new(), app_name, reason });
            continue;
        }
        let (Some(st), Some(gs)) = (status.get(&p.uid), groups.get(&p.uid)) else { continue };
        let cats = shown.map(|s| s.categories.as_slice()).unwrap_or(&[]);
        // (b) in a limited group that has no time left
        let hit = st.iter().filter(|s| s.exhausted()).find(|s| {
            gs.iter().any(|g| g.id == s.group_id && common::apps::group_matches(&g.rules, &p.app_id.to_string(), cats))
        });
        if let Some(s) = hit {
            out.push(Target {
                pid: p.pid,
                start_ticks: p.start_ticks,
                top: true,
                uid: p.uid,
                app_id: p.app_id.clone(),
                key: s.group_id.clone(),
                group_name: s.name.clone(),
                app_name,
                reason: if s.blocked() { Reason::Blocked } else { Reason::Limit },
            });
        }
    }
    // a target whose parent is a target of the same app is a helper, not the top
    let of_app: HashMap<u32, AppId> = procs.iter().map(|p| (p.pid, p.app_id.clone())).collect();
    let targeted: HashSet<u32> = out.iter().map(|t| t.pid).collect();
    let parents: HashMap<u32, u32> = procs.iter().map(|p| (p.pid, p.ppid)).collect();
    for t in &mut out {
        if let Some(pp) = parents.get(&t.pid) {
            t.top = !(targeted.contains(pp) && of_app.get(pp) == Some(&t.app_id));
        }
    }
    out
}

/// Sending signals, so the closing logic can be tested without killing anything.
pub trait Signals {
    /// The process still exists and is the same one (same start time).
    fn alive(&self, pid: u32, start_ticks: u64) -> bool;
    fn term(&self, pid: u32, start_ticks: u64);
    fn kill(&self, pid: u32, start_ticks: u64);
}

/// A notification to show the child.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub uid: u32,
    pub app_id: AppId,
    pub group_name: String,
    pub app_name: String,
    pub reason: Reason,
    /// Seconds until the app closes (0 = closing now).
    pub closes_in: u64,
}

#[derive(Debug, Clone, Copy)]
enum Stage {
    Warned(Instant),
    Termed(Instant),
}

#[derive(Default)]
pub struct Closer {
    stage: HashMap<(u32, u64), Stage>,
    /// (uid, group) → when the child was last told about it.
    told: HashMap<(u32, String), Instant>,
}

impl Closer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Advances every target one step: warn → (grace) → SIGTERM → (grace) → SIGKILL.
    /// Blocked groups and relaunches skip the warning delay. Targets that are gone
    /// (exited, or the group got more time) are forgotten.
    pub fn enforce(&mut self, now: Instant, targets: &[Target], sig: &dyn Signals) -> Vec<Notice> {
        let wanted: HashSet<(u32, u64)> = targets.iter().map(|t| (t.pid, t.start_ticks)).collect();
        self.stage.retain(|k, _| wanted.contains(k));
        self.told.retain(|_, at| now.duration_since(*at) < RELAUNCH_WINDOW * 10);

        let mut notices = Vec::new();
        for t in targets {
            let key = (t.pid, t.start_ticks);
            if !sig.alive(t.pid, t.start_ticks) {
                self.stage.remove(&key);
                continue;
            }
            let told_key = (t.uid, t.key.clone());
            let recently_told = self.told.get(&told_key).is_some_and(|at| now.duration_since(*at) < RELAUNCH_WINDOW);
            match self.stage.get(&key).copied() {
                None => {
                    let immediate = t.reason.immediate() || recently_told;
                    if !recently_told {
                        self.told.insert(told_key, now);
                        notices.push(Notice {
                            uid: t.uid,
                            app_id: t.app_id.clone(),
                            group_name: t.group_name.clone(),
                            app_name: t.app_name.clone(),
                            reason: t.reason,
                            closes_in: if immediate { 0 } else { GRACE.as_secs() },
                        });
                    }
                    if immediate {
                        if t.top {
                            tracing::info!("uid={}: closing '{}' (pid {}, {:?} {})", t.uid, t.app_name, t.pid, t.reason, t.group_name);
                            sig.term(t.pid, t.start_ticks);
                        }
                        self.stage.insert(key, Stage::Termed(now));
                    } else {
                        self.stage.insert(key, Stage::Warned(now));
                    }
                }
                Some(Stage::Warned(at)) if now.duration_since(at) >= GRACE => {
                    if t.top {
                        tracing::info!("uid={}: closing '{}' (pid {}, {:?} {})", t.uid, t.app_name, t.pid, t.reason, t.group_name);
                        sig.term(t.pid, t.start_ticks);
                    }
                    self.stage.insert(key, Stage::Termed(now));
                }
                Some(Stage::Termed(at)) if now.duration_since(at) >= GRACE => {
                    tracing::info!("uid={}: '{}' still running after the grace period, killing pid {}", t.uid, t.app_name, t.pid);
                    sig.kill(t.pid, t.start_ticks);
                }
                _ => {}
            }
        }
        notices
    }
}

/// Real signals, re-checking `/proc/<pid>/stat` start time right before each one.
pub struct ProcSignals;

impl ProcSignals {
    fn start_ticks(pid: u32) -> Option<u64> {
        let s = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        s.get(s.rfind(')')? + 2..)?.split_whitespace().nth(19)?.parse().ok()
    }

    fn send(pid: u32, start_ticks: u64, signal: libc::c_int) {
        if pid > 1 && Self::start_ticks(pid) == Some(start_ticks) {
            // SAFETY: plain kill(2) on a pid we just verified; no memory involved.
            unsafe {
                libc::kill(pid as libc::pid_t, signal);
            }
        }
    }
}

impl Signals for ProcSignals {
    fn alive(&self, pid: u32, start_ticks: u64) -> bool {
        Self::start_ticks(pid) == Some(start_ticks)
    }
    fn term(&self, pid: u32, start_ticks: u64) {
        Self::send(pid, start_ticks, libc::SIGTERM);
    }
    fn kill(&self, pid: u32, start_ticks: u64) {
        Self::send(pid, start_ticks, libc::SIGKILL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::apps::{GroupRule, RuleKind};
    use common::models::DailyLimit;
    use std::cell::RefCell;

    fn proc_(pid: u32, uid: u32, app: &str, comm: &str) -> RunningProc {
        RunningProc { pid, ppid: 1, autostart: false, start_ticks: 100 + pid as u64, uid, app_id: AppId::parse(app).unwrap(), comm: comm.into(), exe: Some(format!("/usr/bin/{comm}")) }
    }

    type Setup = (HashMap<u32, Vec<GroupStatus>>, HashMap<u32, Vec<AppGroupConfig>>, HashMap<(u32, AppId), Shown>);

    fn setup(remaining: i64, limit: u32) -> Setup {
        let g = AppGroupConfig {
            id: "games".into(),
            name: "Games".into(),
            builtin: Some("games".into()),
            rules: vec![GroupRule { kind: RuleKind::Category, value: "Game".into(), exclude: false }, GroupRule { kind: RuleKind::Prefix, value: "steam:".into(), exclude: false }],
            daily_limits: vec![DailyLimit { day_of_week: 0, allowed_minutes: limit }],
            adjustment_today: 0,
        };
        let st = GroupStatus { group_id: "games".into(), name: "Games".into(), limit_minutes: limit, used_seconds: 0, remaining_seconds: remaining };
        let names: HashMap<(u32, AppId), Shown> = [
            ((1001, AppId::parse("desktop:supertux").unwrap()), Shown { name: "SuperTux".into(), icon: None, categories: vec!["Game".into()] }),
            ((1001, AppId::parse("desktop:firefox").unwrap()), Shown { name: "Firefox".into(), icon: None, categories: vec!["WebBrowser".into()] }),
        ]
        .into();
        ([(1001, vec![st])].into(), [(1001, vec![g])].into(), names)
    }

    #[test]
    fn only_exhausted_group_apps_of_managed_children_are_targets() {
        let managed: HashSet<u32> = [1001].into();
        let procs = [
            proc_(10, 1001, "desktop:supertux", "supertux2"),
            proc_(11, 1001, "steam:570", "dota2"),
            proc_(12, 1001, "desktop:firefox", "firefox"),           // not in the group
            proc_(13, 1000, "desktop:supertux", "supertux2"),        // the parent's process
            proc_(14, 0, "desktop:supertux", "supertux2"),           // root
            proc_(15, 1001, "desktop:supertux", "screenguard-tray"), // never ourselves
        ];
        let none = HashMap::new();
        let (st, gs, names) = setup(0, 60);
        let t = targets(&procs, &st, &gs, &none, &names, &managed);
        assert_eq!(t.iter().map(|t| t.pid).collect::<Vec<_>>(), vec![10, 11]);
        assert_eq!((t[0].app_name.as_str(), t[0].group_name.as_str(), t[0].reason), ("SuperTux", "Games", Reason::Limit));
        // time left: nothing to close
        let (st, gs, names) = setup(300, 60);
        assert!(targets(&procs, &st, &gs, &none, &names, &managed).is_empty());
        // blocked day
        let (st, gs, names) = setup(0, 0);
        assert_eq!(targets(&procs, &st, &gs, &none, &names, &managed)[0].reason, Reason::Blocked);

        // new-app approval: everything not approved, even with time left; denied is distinct
        let (st, gs, names) = setup(300, 60);
        let approval: HashMap<u32, AppApprovalConfig> =
            [(1001, AppApprovalConfig { approved: vec!["desktop:firefox".into()], denied: vec!["steam:570".into()] })].into();
        let t = targets(&procs, &st, &gs, &approval, &names, &managed);
        let r: Vec<(u32, Reason)> = t.iter().map(|t| (t.pid, t.reason)).collect();
        assert_eq!(r, vec![(10, Reason::NeedsApproval), (11, Reason::Denied)]); // firefox approved; others untouched

        // autostarted apps are not asked about (only group limits apply to them)
        let mut auto = proc_(16, 1001, "desktop:supertux", "supertux2");
        auto.autostart = true;
        assert!(targets(&[auto.clone()], &st, &gs, &approval, &names, &managed).is_empty());
        let (st0, gs0, names0) = setup(0, 60);
        assert_eq!(targets(&[auto], &st0, &gs0, &none, &names0, &managed)[0].reason, Reason::Limit);
    }

    #[test]
    fn only_the_top_process_of_an_app_gets_sigterm() {
        let managed: HashSet<u32> = [1001].into();
        let (st, gs, names) = setup(0, 0); // blocked: close at once
        let mut main = proc_(10, 1001, "desktop:supertux", "supertux2");
        main.ppid = 900;
        let mut helper = proc_(11, 1001, "desktop:supertux", "supertux2");
        helper.ppid = 10;
        let mut helper2 = proc_(12, 1001, "desktop:supertux", "supertux2");
        helper2.ppid = 11;
        let t = targets(&[main, helper, helper2], &st, &gs, &HashMap::new(), &names, &managed);
        assert_eq!(t.iter().map(|t| (t.pid, t.top)).collect::<Vec<_>>(), vec![(10, true), (11, false), (12, false)]);

        // the helpers ignore nothing here, but only pid 10 is signalled; after the
        // grace period anything left is killed
        let sig = FakeSignals { ignores_term: [10, 11, 12].into(), ..Default::default() };
        let mut c = Closer::new();
        let t0 = Instant::now();
        c.enforce(t0, &t, &sig);
        assert_eq!(*sig.log.borrow(), vec![("term", 10)]);
        c.enforce(t0 + GRACE, &t, &sig);
        assert_eq!(*sig.log.borrow(), vec![("term", 10), ("kill", 10), ("kill", 11), ("kill", 12)]);
    }

    #[derive(Default)]
    struct FakeSignals {
        dead: RefCell<HashSet<u32>>,
        ignores_term: HashSet<u32>,
        log: RefCell<Vec<(&'static str, u32)>>,
    }

    impl Signals for FakeSignals {
        fn alive(&self, pid: u32, _: u64) -> bool {
            !self.dead.borrow().contains(&pid)
        }
        fn term(&self, pid: u32, _: u64) {
            self.log.borrow_mut().push(("term", pid));
            if !self.ignores_term.contains(&pid) {
                self.dead.borrow_mut().insert(pid);
            }
        }
        fn kill(&self, pid: u32, _: u64) {
            self.log.borrow_mut().push(("kill", pid));
            self.dead.borrow_mut().insert(pid);
        }
    }

    fn target(pid: u32, blocked: bool) -> Target {
        Target {
            pid,
            start_ticks: 1,
            top: true,
            uid: 1001,
            app_id: AppId::parse("desktop:supertux").unwrap(),
            key: "games".into(),
            group_name: "Games".into(),
            app_name: "SuperTux".into(),
            reason: if blocked { Reason::Blocked } else { Reason::Limit },
        }
    }

    #[test]
    fn warn_then_term_then_kill() {
        let sig = FakeSignals { ignores_term: [10].into(), ..Default::default() };
        let mut c = Closer::new();
        let t0 = Instant::now();
        let n = c.enforce(t0, &[target(10, false)], &sig);
        assert_eq!(n.len(), 1);
        assert_eq!(n[0].closes_in, 10);
        assert!(sig.log.borrow().is_empty()); // warned only
        c.enforce(t0 + Duration::from_secs(5), &[target(10, false)], &sig);
        assert!(sig.log.borrow().is_empty()); // still in grace
        c.enforce(t0 + Duration::from_secs(10), &[target(10, false)], &sig);
        assert_eq!(*sig.log.borrow(), vec![("term", 10)]);
        c.enforce(t0 + Duration::from_secs(20), &[target(10, false)], &sig); // ignored TERM
        assert_eq!(*sig.log.borrow(), vec![("term", 10), ("kill", 10)]);
        assert!(c.enforce(t0 + Duration::from_secs(25), &[target(10, false)], &sig).is_empty()); // gone
    }

    #[test]
    fn blocked_and_relaunched_apps_close_at_once_without_spam() {
        let sig = FakeSignals::default();
        let mut c = Closer::new();
        let t0 = Instant::now();
        let n = c.enforce(t0, &[target(20, true)], &sig);
        assert_eq!((n.len(), n[0].reason, n[0].closes_in), (1, Reason::Blocked, 0));
        assert_eq!(*sig.log.borrow(), vec![("term", 20)]);
        // relaunched 5 s later as a new process: closed at once, no second notice
        let n = c.enforce(t0 + Duration::from_secs(5), &[target(21, false)], &sig);
        assert!(n.is_empty());
        assert_eq!(*sig.log.borrow(), vec![("term", 20), ("term", 21)]);
        // after the relaunch window the child is told again
        let n = c.enforce(t0 + Duration::from_secs(70), &[target(22, true)], &sig);
        assert_eq!(n.len(), 1);
    }

    #[test]
    fn more_time_cancels_a_pending_close() {
        let sig = FakeSignals::default();
        let mut c = Closer::new();
        let t0 = Instant::now();
        c.enforce(t0, &[target(30, false)], &sig);
        c.enforce(t0 + Duration::from_secs(5), &[], &sig); // the parent added time
        c.enforce(t0 + Duration::from_secs(15), &[], &sig);
        assert!(sig.log.borrow().is_empty());
    }

    /// Real processes: only the target dies, one that ignores SIGTERM is killed.
    #[test]
    fn real_signals_only_hit_the_targets() {
        use std::process::Command;
        let mut victim = Command::new("sleep").arg("300").spawn().unwrap();
        let mut stubborn = Command::new("sh").args(["-c", "trap '' TERM; while :; do sleep 1; done"]).spawn().unwrap();
        let mut bystander = Command::new("sleep").arg("300").spawn().unwrap();
        std::thread::sleep(Duration::from_millis(200)); // let sh install its trap
        let start = |pid: u32| ProcSignals::start_ticks(pid).unwrap();
        let t = |child: &std::process::Child| {
            let pid = child.id();
            Target { pid, start_ticks: start(pid), top: true, uid: 1001, app_id: AppId::parse("desktop:x").unwrap(), key: "g".into(), group_name: "G".into(), app_name: "x".into(), reason: Reason::Blocked }
        };
        let targets = [t(&victim), t(&stubborn)];
        // a wrong start time (reused PID) is never signalled
        let wrong = Target { start_ticks: targets[0].start_ticks + 1, ..targets[0].clone() };
        ProcSignals.term(wrong.pid, wrong.start_ticks);
        std::thread::sleep(Duration::from_millis(100));
        assert!(victim.try_wait().unwrap().is_none());

        let mut c = Closer::new();
        let t0 = Instant::now();
        c.enforce(t0, &targets, &ProcSignals); // blocked → SIGTERM now
        let reaped = |ch: &mut std::process::Child| (0..50).any(|_| {
            std::thread::sleep(Duration::from_millis(20));
            ch.try_wait().unwrap().is_some()
        });
        assert!(reaped(&mut victim));
        assert!(!reaped(&mut stubborn));
        c.enforce(t0 + GRACE, &targets, &ProcSignals); // still there → SIGKILL
        assert!(reaped(&mut stubborn));
        assert!(bystander.try_wait().unwrap().is_none(), "untargeted process must survive");
        bystander.kill().unwrap();
        let _ = bystander.wait();
    }
}
