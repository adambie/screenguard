//! Per-app usage scanner (docs/feature-app-usage-limits.md, Phase 1).

use std::path::Path;

/// Apps are identified by their cgroup v2 unit; without the unified hierarchy the
/// agent doesn't advertise or collect per-app usage.
pub fn supported() -> bool {
    Path::new("/sys/fs/cgroup/cgroup.controllers").exists()
}
