//! Forced logout that leaves the display manager's session leader alive.
//!
//! logind's `KillSession(sid, "all", SIGKILL)` also kills the session leader. For a
//! graphical login that leader is the display manager's root-owned helper
//! (`sddm-helper`, `plasmalogin-helper`, `gdm-session-worker`). When it dies of SIGKILL
//! the display manager treats it as a crash instead of a logout: no greeter comes back
//! and the seat is left on a dead VT with a black screen.
//!
//! So the forced logout SIGKILLs only the processes in the session's cgroup that run as
//! the session's user. The leader then sees its child (the desktop session) exit, closes
//! the PAM session normally and the display manager shows the greeter again.
//! SIGKILL is kept on purpose: lock-screen daemons such as kscreenlocker ignore SIGTERM.

use std::path::{Path, PathBuf};

/// Pids a forced logout should SIGKILL, given `(pid, real uid)` of every process in the
/// session's cgroup: the session user's processes, never the leader, never pid 1, and
/// nothing at all for a root session.
pub(crate) fn logout_victims(procs: &[(u32, u32)], session_uid: u32, leader: u32) -> Vec<u32> {
    if session_uid == 0 {
        return Vec::new();
    }
    procs
        .iter()
        .filter(|&&(pid, uid)| uid == session_uid && pid != leader && pid > 1)
        .map(|&(pid, _)| pid)
        .collect()
}

/// Real uid from the text of `/proc/<pid>/status` (first field of the `Uid:` line).
pub(crate) fn parse_real_uid(status: &str) -> Option<u32> {
    status
        .lines()
        .find_map(|l| l.strip_prefix("Uid:"))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

/// cgroup v2 path from the text of `/proc/<pid>/cgroup` (the `0::` line).
pub(crate) fn parse_unified_cgroup(cgroup: &str) -> Option<&str> {
    cgroup.lines().find_map(|l| l.strip_prefix("0::"))
}

/// The session scope's cgroup directory, found through the leader's cgroup. Only
/// accepted when its last component is the scope logind reports for the session, so a
/// leader that moved elsewhere (or a cgroup v1 system) can never widen the kill.
pub(crate) fn scope_dir_from(cgroup_file: &str, scope: &str, root: &Path) -> Option<PathBuf> {
    let rel = parse_unified_cgroup(cgroup_file)?.trim();
    if scope.is_empty() || rel.rsplit('/').next() != Some(scope) {
        return None;
    }
    Some(root.join(rel.trim_start_matches('/')))
}

/// Session scope cgroup directory for a live leader pid.
pub fn session_scope_dir(leader: u32, scope: &str) -> Option<PathBuf> {
    let cgroup = std::fs::read_to_string(format!("/proc/{leader}/cgroup")).ok()?;
    scope_dir_from(&cgroup, scope, Path::new("/sys/fs/cgroup"))
}

/// Every pid listed in `cgroup.procs` of `dir` and its sub-cgroups.
fn cgroup_pids(dir: &Path, out: &mut Vec<u32>) {
    if let Ok(s) = std::fs::read_to_string(dir.join("cgroup.procs")) {
        out.extend(s.lines().filter_map(|l| l.trim().parse::<u32>().ok()));
    }
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                cgroup_pids(&e.path(), out);
            }
        }
    }
}

fn real_uid(pid: u32) -> Option<u32> {
    parse_real_uid(&std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?)
}

/// One pass: SIGKILL the session user's processes in `dir`, sparing the leader.
/// Returns how many were signalled. The uid is read right before each kill, so a pid
/// reused by a process of another user is never hit.
pub fn kill_user_processes(dir: &Path, session_uid: u32, leader: u32) -> usize {
    let mut pids = Vec::new();
    cgroup_pids(dir, &mut pids);
    let procs: Vec<(u32, u32)> =
        pids.into_iter().filter_map(|pid| Some((pid, real_uid(pid)?))).collect();
    let mut n = 0;
    for pid in logout_victims(&procs, session_uid, leader) {
        if real_uid(pid) == Some(session_uid) {
            // SAFETY: plain kill(2) on a pid whose owner we just re-checked.
            if unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) } == 0 {
                n += 1;
            }
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEADER: u32 = 1583;
    const KID: u32 = 1005;

    #[test]
    fn kills_the_users_processes_but_not_the_root_leader() {
        // plasmalogin-helper (root, leader), startplasma-wayland, ksecretd (reparented to 1)
        let procs = [(LEADER, 0), (1600, KID), (1601, KID)];
        assert_eq!(logout_victims(&procs, KID, LEADER), vec![1600, 1601]);
    }

    #[test]
    fn other_users_and_root_processes_in_the_scope_are_spared() {
        let procs = [(LEADER, 0), (1700, 0), (1701, 1000), (1702, KID)];
        assert_eq!(logout_victims(&procs, KID, LEADER), vec![1702]);
    }

    #[test]
    fn a_user_owned_leader_is_spared_too() {
        // e.g. a startx-style session whose leader runs as the user: left to the
        // KillSession fallback rather than killed here.
        let procs = [(1800, KID), (1801, KID)];
        assert_eq!(logout_victims(&procs, KID, 1800), vec![1801]);
    }

    #[test]
    fn never_pid_1_and_never_for_a_root_session() {
        assert!(logout_victims(&[(1, KID)], KID, LEADER).is_empty());
        assert!(logout_victims(&[(1900, 0)], 0, LEADER).is_empty());
    }

    #[test]
    fn parses_real_uid_from_proc_status() {
        let status = "Name:\tstartplasma-way\nPid:\t1600\nUid:\t1005\t1005\t1005\t1005\nGid:\t1005\t1005\t1005\t1005\n";
        assert_eq!(parse_real_uid(status), Some(1005));
        // setuid binary started by the user: real uid is still the user's
        assert_eq!(parse_real_uid("Uid:\t1005\t0\t0\t0\n"), Some(1005));
        assert_eq!(parse_real_uid("Name:\tx\n"), None);
    }

    #[test]
    fn scope_dir_comes_from_the_leaders_unified_cgroup() {
        let root = Path::new("/sys/fs/cgroup");
        let cg = "0::/user.slice/user-1005.slice/session-26.scope\n";
        assert_eq!(
            scope_dir_from(cg, "session-26.scope", root),
            Some(PathBuf::from("/sys/fs/cgroup/user.slice/user-1005.slice/session-26.scope"))
        );
    }

    #[test]
    fn scope_dir_is_refused_when_it_does_not_match_logind() {
        let root = Path::new("/sys/fs/cgroup");
        let cg = "0::/user.slice/user-1005.slice/session-26.scope\n";
        assert_eq!(scope_dir_from(cg, "session-27.scope", root), None);
        assert_eq!(scope_dir_from(cg, "", root), None);
        assert_eq!(scope_dir_from("0::/\n", "session-26.scope", root), None);
        // cgroup v1 only: no unified line
        assert_eq!(scope_dir_from("1:name=systemd:/user.slice\n", "session-26.scope", root), None);
    }

    /// Real processes in a fake cgroup: the "leader" survives, the rest are killed,
    /// a process not listed in the cgroup is untouched.
    #[test]
    fn real_kill_spares_the_leader_and_processes_outside_the_cgroup() {
        use std::process::Command;
        let mut leader = Command::new("sleep").arg("300").spawn().unwrap();
        let mut desktop = Command::new("sleep").arg("300").spawn().unwrap();
        let mut stubborn =
            Command::new("sh").args(["-c", "trap '' TERM; while :; do sleep 1; done"]).spawn().unwrap();
        let mut outside = Command::new("sleep").arg("300").spawn().unwrap();

        let dir = std::env::temp_dir().join(format!("sg-session-kill-{}", std::process::id()));
        let sub = dir.join("nested");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(dir.join("cgroup.procs"), format!("{}\n{}\n", leader.id(), desktop.id())).unwrap();
        std::fs::write(sub.join("cgroup.procs"), format!("{}\n", stubborn.id())).unwrap();

        let me = real_uid(std::process::id()).unwrap();
        assert_eq!(kill_user_processes(&dir, me, leader.id()), 2);

        let reaped = |ch: &mut std::process::Child| {
            (0..50).any(|_| {
                std::thread::sleep(std::time::Duration::from_millis(20));
                ch.try_wait().unwrap().is_some()
            })
        };
        assert!(reaped(&mut desktop));
        assert!(reaped(&mut stubborn));
        assert!(leader.try_wait().unwrap().is_none(), "leader must survive");
        assert!(outside.try_wait().unwrap().is_none(), "process outside the cgroup must survive");

        for ch in [&mut leader, &mut outside] {
            ch.kill().unwrap();
            let _ = ch.wait();
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
