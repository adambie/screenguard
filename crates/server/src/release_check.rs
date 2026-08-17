use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{interval, Duration};

pub type LatestRelease = Arc<RwLock<Option<String>>>;

pub fn parse_semver(v: &str) -> Option<(u32, u32, u32)> {
    let mut p = v.split('.');
    Some((p.next()?.parse().ok()?, p.next()?.parse().ok()?, p.next()?.parse().ok()?))
}

pub fn is_older(agent: &str, latest: &str) -> bool {
    match (parse_semver(agent), parse_semver(latest)) {
        (Some(a), Some(l)) => a < l,
        _ => agent != latest,
    }
}

/// Fetch the highest `v<major>.<minor>.<patch>` release from GitHub, ignoring
/// mobile/other tags so that `mobile0.0.x` releases don't appear as "latest".
async fn fetch_latest(repo: &str) -> Option<String> {
    let url = format!("https://api.github.com/repos/{repo}/releases?per_page=50");
    let client = reqwest::Client::builder()
        .user_agent("screenguard-server/release-check")
        .timeout(Duration::from_secs(10))
        .build()
        .ok()?;
    let releases: Vec<serde_json::Value> = client
        .get(&url)
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;
    releases
        .iter()
        .filter_map(|r| r["tag_name"].as_str())
        .filter(|t| t.starts_with('v'))
        .filter_map(|t| {
            let v = t.trim_start_matches('v');
            let parsed = parse_semver(v)?;
            Some((parsed, v.to_string()))
        })
        .max_by_key(|(v, _)| *v)
        .map(|(_, v)| v)
}

/// Spawn a background task that refreshes the cached latest release every hour.
/// The first fetch happens immediately.
pub fn spawn_updater(repo: &'static str, cache: LatestRelease) {
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(3600));
        loop {
            ticker.tick().await;
            match fetch_latest(repo).await {
                Some(v) => {
                    tracing::info!("Latest agent release on GitHub: v{v}");
                    *cache.write().await = Some(v);
                }
                None => tracing::warn!("Could not fetch latest release from GitHub"),
            }
        }
    });
}
