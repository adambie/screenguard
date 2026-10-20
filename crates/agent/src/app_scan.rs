//! Per-app usage scanner (docs/feature-app-usage-limits.md, Phase 1).
//!
//! Step 1: enumerate the processes of managed users from `/proc`. Identification
//! (which app a process belongs to) builds on the `ProcInfo`s collected here.
//!
//! Privacy: command lines and environment values stay on this machine; only the
//! derived app id is ever reported.


use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

/// Apps are identified by their cgroup v2 unit; without the unified hierarchy the
/// agent doesn't advertise or collect per-app usage.
pub fn supported() -> bool {
    Path::new("/sys/fs/cgroup/cgroup.controllers").exists()
}

/// One process of a managed user, as read once when it was first seen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcInfo {
    pub pid: u32,
    pub ppid: u32,
    /// Real UID (from `status`, not the `/proc/<pid>` owner: non-dumpable
    /// processes such as Electron apps show up as root-owned).
    pub uid: u32,
    /// Start time in clock ticks since boot; with `pid` it identifies the process.
    pub start_ticks: u64,
    pub comm: String,
    /// `/proc/<pid>/exe` target without a trailing ` (deleted)`.
    pub exe: Option<String>,
    pub argv: Vec<String>,
    /// cgroup v2 path (`0::` line).
    pub cgroup: String,
    pub gio_desktop_file: Option<String>,
    pub gio_pid: Option<u32>,
}

/// What `stat` tells us cheaply on every tick.
struct Stat {
    ppid: u32,
    start_ticks: u64,
    comm: String,
}

struct Cached {
    start_ticks: u64,
    comm: String,
    /// `None` for processes of other users: they're remembered only so their
    /// `status` isn't re-read on every tick.
    info: Option<ProcInfo>,
}

/// Enumerates processes, doing the full reads only for processes not seen
/// before (keyed by pid + start time + comm, so PID reuse and `exec` are noticed).
pub struct ProcScanner {
    root: PathBuf,
    cache: HashMap<u32, Cached>,
}

impl ProcScanner {
    pub fn new() -> Self {
        Self::with_root("/proc")
    }

    pub fn with_root(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into(), cache: HashMap::new() }
    }

    /// Returns the current processes of `uids`, keyed by pid.
    pub fn scan(&mut self, uids: &HashSet<u32>) -> HashMap<u32, ProcInfo> {
        let mut seen = HashMap::with_capacity(self.cache.len());
        let Ok(entries) = fs::read_dir(&self.root) else {
            return HashMap::new();
        };
        for entry in entries.flatten() {
            let Some(pid) = entry.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else {
                continue;
            };
            let Some(stat) = self.read_stat(pid) else { continue };
            let cached = match self.cache.remove(&pid) {
                Some(mut c) if c.start_ticks == stat.start_ticks && c.comm == stat.comm => {
                    // reparenting (double fork, launcher exited) changes ppid
                    if let Some(info) = c.info.as_mut() {
                        info.ppid = stat.ppid;
                    }
                    c
                }
                _ => match self.read_full(pid, &stat, uids) {
                    Some(c) => c,
                    None => continue, // exited while reading
                },
            };
            seen.insert(pid, cached);
        }
        self.cache = seen;
        self.cache
            .values()
            .filter_map(|c| c.info.as_ref())
            .filter(|i| uids.contains(&i.uid))
            .map(|i| (i.pid, i.clone()))
            .collect()
    }

    fn path(&self, pid: u32, file: &str) -> PathBuf {
        self.root.join(pid.to_string()).join(file)
    }

    fn read_stat(&self, pid: u32) -> Option<Stat> {
        parse_stat(&fs::read_to_string(self.path(pid, "stat")).ok()?)
    }

    fn read_full(&self, pid: u32, stat: &Stat, uids: &HashSet<u32>) -> Option<Cached> {
        let status = fs::read_to_string(self.path(pid, "status")).ok()?;
        let uid = parse_real_uid(&status)?;
        let mut cached = Cached { start_ticks: stat.start_ticks, comm: stat.comm.clone(), info: None };
        if !uids.contains(&uid) {
            return Some(cached);
        }
        let argv = fs::read(self.path(pid, "cmdline")).map(|b| split_nul(&b)).unwrap_or_default();
        if argv.is_empty() {
            return Some(cached); // kernel thread or zombie
        }
        let exe = fs::read_link(self.path(pid, "exe")).ok().map(|p| {
            let s = p.to_string_lossy().into_owned();
            s.strip_suffix(" (deleted)").map(str::to_string).unwrap_or(s)
        });
        let cgroup = fs::read_to_string(self.path(pid, "cgroup"))
            .ok()
            .and_then(|c| c.lines().find_map(|l| l.strip_prefix("0::").map(str::to_string)))
            .unwrap_or_default();
        let (gio_desktop_file, gio_pid) = fs::read(self.path(pid, "environ"))
            .map(|b| parse_gio_env(&b))
            .unwrap_or((None, None));
        cached.info = Some(ProcInfo {
            pid,
            ppid: stat.ppid,
            uid,
            start_ticks: stat.start_ticks,
            comm: stat.comm.clone(),
            exe,
            argv,
            cgroup,
            gio_desktop_file,
            gio_pid,
        });
        Some(cached)
    }
}

/// `pid (comm) state ppid ... starttime(22nd field) ...` — comm may contain
/// spaces and parentheses, so split at the last `)`.
fn parse_stat(s: &str) -> Option<Stat> {
    let open = s.find('(')?;
    let close = s.rfind(')')?;
    let comm = s.get(open + 1..close)?.to_string();
    let rest: Vec<&str> = s.get(close + 2..)?.split_whitespace().collect();
    Some(Stat { ppid: rest.get(1)?.parse().ok()?, start_ticks: rest.get(19)?.parse().ok()?, comm })
}

fn parse_real_uid(status: &str) -> Option<u32> {
    status.lines().find_map(|l| l.strip_prefix("Uid:"))?.split_whitespace().next()?.parse().ok()
}

fn split_nul(b: &[u8]) -> Vec<String> {
    b.split(|&c| c == 0).filter(|a| !a.is_empty()).map(|a| String::from_utf8_lossy(a).into_owned()).collect()
}

fn parse_gio_env(b: &[u8]) -> (Option<String>, Option<u32>) {
    let (mut file, mut pid) = (None, None);
    for kv in b.split(|&c| c == 0) {
        if let Some(v) = kv.strip_prefix(b"GIO_LAUNCHED_DESKTOP_FILE=") {
            file = Some(String::from_utf8_lossy(v).into_owned());
        } else if let Some(v) = kv.strip_prefix(b"GIO_LAUNCHED_DESKTOP_FILE_PID=") {
            pid = std::str::from_utf8(v).ok().and_then(|s| s.parse().ok());
        }
    }
    (file, pid)
}

#[cfg(test)]
pub(crate) mod fake_proc {
    //! A fake `/proc` tree in a temp dir, shared by the scanner and
    //! identification tests.
    use std::fs;
    use std::os::unix::fs::symlink;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU32, Ordering};

    pub struct FakeProc {
        pub root: PathBuf,
    }

    #[derive(Clone)]
    pub struct P<'a> {
        pub pid: u32,
        pub ppid: u32,
        pub uid: u32,
        pub start: u64,
        pub comm: &'a str,
        pub exe: Option<&'a str>,
        pub argv: &'a [&'a str],
        pub cgroup: &'a str,
        pub env: &'a [&'a str],
    }

    impl Default for P<'_> {
        fn default() -> Self {
            P { pid: 0, ppid: 1, uid: 1000, start: 1000, comm: "x", exe: None, argv: &[], cgroup: "", env: &[] }
        }
    }

    impl FakeProc {
        pub fn new() -> Self {
            static N: AtomicU32 = AtomicU32::new(0);
            let root = std::env::temp_dir()
                .join(format!("sg-fakeproc-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).unwrap();
            FakeProc { root }
        }

        pub fn add(&self, p: P) -> &Self {
            let dir = self.root.join(p.pid.to_string());
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            // fields after comm: state ppid pgrp session tty tpgid flags minflt
            // cminflt majflt cmajflt utime stime cutime cstime priority nice
            // num_threads itrealvalue starttime ...
            fs::write(
                dir.join("stat"),
                format!("{} ({}) S {} 0 0 0 0 0 0 0 0 0 0 0 0 0 20 0 1 0 {} 0 0\n", p.pid, p.comm, p.ppid, p.start),
            )
            .unwrap();
            fs::write(dir.join("status"), format!("Name:\t{}\nUid:\t{}\t{}\t{}\t{}\n", p.comm, p.uid, p.uid, p.uid, p.uid))
                .unwrap();
            fs::write(dir.join("cmdline"), p.argv.iter().map(|a| format!("{a}\0")).collect::<String>()).unwrap();
            fs::write(dir.join("cgroup"), format!("0::{}\n", p.cgroup)).unwrap();
            fs::write(dir.join("environ"), p.env.iter().map(|a| format!("{a}\0")).collect::<String>()).unwrap();
            if let Some(exe) = p.exe {
                symlink(exe, dir.join("exe")).unwrap();
            }
            self
        }

        pub fn remove(&self, pid: u32) {
            fs::remove_dir_all(self.root.join(pid.to_string())).unwrap();
        }

        pub fn path(&self) -> &Path {
            &self.root
        }
    }

    impl Drop for FakeProc {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake_proc::{FakeProc, P};
    use super::*;

    fn uids(u: &[u32]) -> HashSet<u32> {
        u.iter().copied().collect()
    }

    #[test]
    fn reads_processes_of_managed_users_only() {
        let fp = FakeProc::new();
        fp.add(P { pid: 1, ppid: 0, uid: 0, comm: "systemd", argv: &["/sbin/init"], ..Default::default() });
        fp.add(P {
            pid: 500,
            uid: 1001,
            comm: "Web Content",
            exe: Some("/usr/lib/firefox/firefox (deleted)"),
            argv: &["/usr/lib/firefox/firefox", "-contentproc"],
            cgroup: "/user.slice/user-1001.slice/user@1001.service/app.slice/app-firefox@abc.service",
            env: &[
                "HOME=/home/kid",
                "GIO_LAUNCHED_DESKTOP_FILE=/usr/share/applications/firefox.desktop",
                "GIO_LAUNCHED_DESKTOP_FILE_PID=499",
            ],
            ..Default::default()
        });
        fp.add(P { pid: 600, uid: 1000, comm: "bash", argv: &["bash"], ..Default::default() });
        // kernel thread: empty cmdline
        fp.add(P { pid: 2, ppid: 0, uid: 1001, comm: "kthreadd", ..Default::default() });

        let mut sc = ProcScanner::with_root(fp.path());
        let procs = sc.scan(&uids(&[1001]));
        assert_eq!(procs.len(), 1);
        let p = &procs[&500];
        assert_eq!(p.comm, "Web Content");
        assert_eq!(p.ppid, 1);
        assert_eq!(p.exe.as_deref(), Some("/usr/lib/firefox/firefox"));
        assert_eq!(p.argv, vec!["/usr/lib/firefox/firefox", "-contentproc"]);
        assert!(p.cgroup.ends_with("app-firefox@abc.service"));
        assert_eq!(p.gio_desktop_file.as_deref(), Some("/usr/share/applications/firefox.desktop"));
        assert_eq!(p.gio_pid, Some(499));
    }

    #[test]
    fn caches_until_pid_is_reused_or_execs() {
        let fp = FakeProc::new();
        fp.add(P { pid: 10, comm: "bash", argv: &["bash", "/usr/bin/wrapper"], ..Default::default() });
        let mut sc = ProcScanner::with_root(fp.path());
        assert_eq!(sc.scan(&uids(&[1000]))[&10].argv[0], "bash");

        // same pid/start/comm: cached, a changed cmdline is not re-read
        fp.add(P { pid: 10, comm: "bash", argv: &["changed"], ..Default::default() });
        assert_eq!(sc.scan(&uids(&[1000]))[&10].argv[0], "bash");

        // exec changes comm -> re-read
        fp.add(P { pid: 10, comm: "electron", argv: &["electron", "/usr/lib/app/app.asar"], ..Default::default() });
        assert_eq!(sc.scan(&uids(&[1000]))[&10].argv[0], "electron");

        // reparented: ppid is refreshed even though the rest stays cached
        fp.add(P { pid: 10, ppid: 1, comm: "electron", argv: &["ignored"], ..Default::default() });
        fp.add(P { pid: 10, ppid: 900, comm: "electron", argv: &["ignored"], ..Default::default() });
        let p = &sc.scan(&uids(&[1000]))[&10];
        assert_eq!((p.ppid, p.argv[0].as_str()), (900, "electron"));

        // PID reused by a new process -> re-read
        fp.add(P { pid: 10, start: 2000, comm: "electron", argv: &["other"], ..Default::default() });
        assert_eq!(sc.scan(&uids(&[1000]))[&10].argv[0], "other");

        // exited -> gone, and dropped from the cache
        fp.remove(10);
        assert!(sc.scan(&uids(&[1000])).is_empty());
        assert!(sc.cache.is_empty());
    }

    #[test]
    fn other_users_are_cached_without_details() {
        let fp = FakeProc::new();
        fp.add(P { pid: 20, uid: 1000, comm: "bash", argv: &["bash"], ..Default::default() });
        let mut sc = ProcScanner::with_root(fp.path());
        assert!(sc.scan(&uids(&[1001])).is_empty());
        assert!(sc.cache[&20].info.is_none());
    }

    #[test]
    fn finds_itself_in_the_real_proc() {
        let me = std::process::id();
        let uid = parse_real_uid(&fs::read_to_string("/proc/self/status").unwrap()).unwrap();
        let procs = ProcScanner::new().scan(&uids(&[uid]));
        let p = procs.get(&me).expect("own process listed");
        assert!(p.exe.is_some());
        assert!(!p.argv.is_empty());
    }

    #[test]
    fn parses_stat_with_odd_comm() {
        let s = parse_stat("42 (a) b (c)) S 7 0 0 0 0 0 0 0 0 0 0 0 0 0 20 0 1 0 12345 0 0").unwrap();
        assert_eq!((s.comm.as_str(), s.ppid, s.start_ticks), ("a) b (c)", 7, 12345));
    }
}
