use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;

const DEFAULT_CONFIG_PATH: &str = "/etc/screenguard/agent.toml";
const CONFIG_PATH_ENV: &str = "SCREENGUARD_AGENT_CONFIG";

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct AgentConfig {
    /// Direct server WSS URL; if set, mDNS discovery is skipped.
    pub server_url: Option<String>,

    #[serde(default = "default_heartbeat_interval")]
    pub heartbeat_interval: u64,

    #[serde(default = "default_user_scan_interval")]
    pub user_scan_interval: u64,

    #[serde(default = "default_cache_ttl_hours")]
    pub cache_ttl_hours: u64,

    #[serde(default = "default_min_uid")]
    pub min_uid: u32,
}

fn default_heartbeat_interval() -> u64 { 10 }
fn default_user_scan_interval() -> u64 { 300 }
fn default_cache_ttl_hours() -> u64 { 48 }
fn default_min_uid() -> u32 { 1000 }

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            server_url: None,
            heartbeat_interval: default_heartbeat_interval(),
            user_scan_interval: default_user_scan_interval(),
            cache_ttl_hours: default_cache_ttl_hours(),
            min_uid: default_min_uid(),
        }
    }
}

pub fn load(path: Option<&str>) -> Result<AgentConfig> {
    let env_path = std::env::var(CONFIG_PATH_ENV).ok();
    let path = path.or(env_path.as_deref()).unwrap_or(DEFAULT_CONFIG_PATH);
    if !Path::new(path).exists() {
        tracing::warn!("Config file not found at {path}, using defaults");
        return Ok(AgentConfig::default());
    }
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read config file: {path}"))?;
    let config: AgentConfig = toml::from_str(&raw)
        .with_context(|| format!("Failed to parse config file: {path}"))?;
    Ok(config)
}
