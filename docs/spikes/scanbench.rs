//! Build: rustc -O docs/spikes/scanbench.rs -o /tmp/scanbench && /tmp/scanbench [UID]
//! Benchmark for docs/feature-app-usage-limits.md step 0.6 — read-only.
//! Full scan: for every process of UID read stat, cmdline, cgroup, exe, environ.
//! Cached tick: list /proc, check owner + start time; full reads only for new PIDs.
use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::time::{Duration, Instant};

fn cpu() -> Duration {
    let mut ru: libc_rusage = unsafe { std::mem::zeroed() };
    unsafe { getrusage(0, &mut ru) };
    let t = |s: i64, u: i64| Duration::from_secs(s as u64) + Duration::from_micros(u as u64);
    t(ru.utime_s, ru.utime_us) + t(ru.stime_s, ru.stime_us)
}
#[repr(C)]
struct libc_rusage { utime_s: i64, utime_us: i64, stime_s: i64, stime_us: i64, rest: [i64; 14] }
extern "C" { fn getrusage(who: i32, r: *mut libc_rusage) -> i32; }

fn start_time(pid: &str) -> Option<u64> {
    let s = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    s[s.rfind(')')? + 2..].split(' ').nth(19)?.parse().ok()
}

fn full_read(pid: &str) -> usize {
    let mut n = 0;
    for f in ["stat", "cmdline", "cgroup", "environ"] {
        n += fs::read(format!("/proc/{pid}/{f}")).map(|v| v.len()).unwrap_or(0);
    }
    n + fs::read_link(format!("/proc/{pid}/exe")).map(|p| p.as_os_str().len()).unwrap_or(0)
}

fn pids_of(uid: u32) -> Vec<String> {
    fs::read_dir("/proc").unwrap().flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            if !name.bytes().all(|b| b.is_ascii_digit()) { return None; }
            (e.metadata().ok()?.uid() == uid).then_some(name)
        }).collect()
}

fn main() {
    let uid: u32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1000);
    let total = fs::read_dir("/proc").unwrap().flatten()
        .filter(|e| e.file_name().to_string_lossy().bytes().all(|b| b.is_ascii_digit())).count();
    let n = 200;

    let (c0, w0) = (cpu(), Instant::now());
    let mut procs = 0;
    for _ in 0..n {
        let pids = pids_of(uid);
        procs = pids.len();
        for p in &pids { std::hint::black_box(full_read(p)); }
    }
    let (fc, fw) = ((cpu() - c0) / n, w0.elapsed() / n);

    let mut cache: HashMap<String, u64> = HashMap::new();
    let (c0, w0) = (cpu(), Instant::now());
    for _ in 0..n {
        let pids = pids_of(uid);
        let mut seen = HashMap::with_capacity(pids.len());
        for p in pids {
            let st = start_time(&p);
            if let Some(st) = st {
                if cache.get(&p) != Some(&st) { std::hint::black_box(full_read(&p)); }
                seen.insert(p, st);
            }
        }
        cache = seen;
    }
    let (cc, cw) = ((cpu() - c0) / n, w0.elapsed() / n);

    println!("{total} processes on the system, {procs} of uid {uid}");
    println!("full scan:   cpu {:>7.2} ms  wall {:>7.2} ms", fc.as_secs_f64() * 1e3, fw.as_secs_f64() * 1e3);
    println!("cached tick: cpu {:>7.2} ms  wall {:>7.2} ms", cc.as_secs_f64() * 1e3, cw.as_secs_f64() * 1e3);
    for iv in [2.0, 5.0, 10.0] {
        println!("  at {iv:>4}s interval: full {:.3} % CPU, cached {:.3} % CPU",
                 fc.as_secs_f64() / iv * 100.0, cc.as_secs_f64() / iv * 100.0);
    }
}
