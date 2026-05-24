use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;

const DEFAULT_CONFIG_PATH: &str = "/etc/screenguard/server.toml";
const CONFIG_PATH_ENV: &str = "SCREENGUARD_SERVER_CONFIG";
const DEFAULT_DB_PATH: &str = "/var/lib/screenguard/server.db";
const DB_PATH_ENV: &str = "SCREENGUARD_SERVER_DB";

#[derive(Debug, Deserialize, Clone)]
pub struct ServerConfig {
    #[serde(default = "default_listen_addr")]
    pub listen_addr: String,

    #[serde(default = "default_listen_port")]
    pub listen_port: u16,

    #[serde(default = "default_db_path")]
    pub db_path: String,

    /// JWT signing secret. If absent, one is generated and saved on first run.
    pub jwt_secret: Option<String>,

    #[serde(default = "default_jwt_expiry_hours")]
    pub jwt_expiry_hours: u64,

    #[serde(default = "default_enable_mdns")]
    pub enable_mdns: bool,
}

fn default_listen_addr() -> String { "0.0.0.0".to_string() }
fn default_listen_port() -> u16 { 8080 }
fn default_db_path() -> String { DEFAULT_DB_PATH.to_string() }
fn default_jwt_expiry_hours() -> u64 { 24 }
fn default_enable_mdns() -> bool { true }

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            listen_addr: default_listen_addr(),
            listen_port: default_listen_port(),
            db_path: default_db_path(),
            jwt_secret: None,
            jwt_expiry_hours: default_jwt_expiry_hours(),
            enable_mdns: default_enable_mdns(),
        }
    }
}

pub fn load(path: Option<&str>) -> Result<ServerConfig> {
    let env_path = std::env::var(CONFIG_PATH_ENV).ok();
    let path = path.or(env_path.as_deref()).unwrap_or(DEFAULT_CONFIG_PATH);

    let mut config = if Path::new(path).exists() {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read config: {path}"))?;
        toml::from_str::<ServerConfig>(&raw)
            .with_context(|| format!("Failed to parse config: {path}"))?
    } else {
        tracing::warn!("Config file not found at {path}, using defaults");
        ServerConfig::default()
    };

    // PARENTAL_SERVER_DB overrides db_path from config.
    if let Ok(db) = std::env::var(DB_PATH_ENV) {
        config.db_path = db;
    }

    Ok(config)
}
