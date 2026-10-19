use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;

const DEFAULT_CONFIG_PATH: &str = "/etc/screenguard/server.toml";
const CONFIG_PATH_ENV: &str = "SCREENGUARD_SERVER_CONFIG";
const DEFAULT_DB_PATH: &str = "/var/lib/screenguard/server.db";
const DB_PATH_ENV: &str = "SCREENGUARD_SERVER_DB";
const DATABASE_URL_ENV: &str = "DATABASE_URL";
const JWT_SECRET_ENV: &str = "SCREENGUARD_SERVER_JWT_SECRET";
const GITHUB_REPO_ENV: &str = "SCREENGUARD_GITHUB_REPO";
const PORT_ENV: &str = "SCREENGUARD_SERVER_PORT";

#[derive(Debug, Deserialize, Clone)]
pub struct ServerConfig {
    #[serde(default = "default_listen_addr")]
    pub listen_addr: String,

    #[serde(default = "default_listen_port")]
    pub listen_port: u16,

    #[serde(default = "default_db_path")]
    pub db_path: String,

    /// PostgreSQL connection URL. When set, takes precedence over db_path.
    /// Homelab ignores this; cloud server sets DATABASE_URL instead of db_path.
    pub database_url: Option<String>,

    /// GitHub repo used for agent release checks (owner/repo).
    #[serde(default = "default_github_repo")]
    pub github_repo: String,

    /// JWT signing secret. If absent, one is generated and saved on first run.
    pub jwt_secret: Option<String>,

    #[serde(default = "default_jwt_expiry_hours")]
    pub jwt_expiry_hours: u64,

    #[serde(default = "default_enable_mdns")]
    pub enable_mdns: bool,
}

fn default_github_repo() -> String { "adambie/screenguard".to_string() }
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
            database_url: None,
            github_repo: default_github_repo(),
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

    apply_env_overrides(&mut config, |key| std::env::var(key).ok())?;
    Ok(config)
}

/// Environment variables take precedence over the config file (handy in Docker,
/// where the image ships no server.toml).
fn apply_env_overrides(config: &mut ServerConfig, env: impl Fn(&str) -> Option<String>) -> Result<()> {
    if let Some(db) = env(DB_PATH_ENV) {
        config.db_path = db;
    }
    if let Some(url) = env(DATABASE_URL_ENV) {
        config.database_url = Some(url);
    }
    if let Some(secret) = env(JWT_SECRET_ENV) {
        config.jwt_secret = Some(secret);
    }
    if let Some(repo) = env(GITHUB_REPO_ENV) {
        config.github_repo = repo;
    }
    if let Some(port) = env(PORT_ENV) {
        config.listen_port = port
            .trim()
            .parse()
            .with_context(|| format!("Invalid {PORT_ENV}: {port:?}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn with_env(vars: &[(&str, &str)]) -> Result<ServerConfig> {
        let vars: HashMap<String, String> =
            vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let mut config = ServerConfig::default();
        apply_env_overrides(&mut config, |k| vars.get(k).cloned())?;
        Ok(config)
    }

    #[test]
    fn port_defaults_to_8080() {
        assert_eq!(with_env(&[]).unwrap().listen_port, 8080);
    }

    #[test]
    fn port_env_overrides_default() {
        assert_eq!(with_env(&[(PORT_ENV, "8089")]).unwrap().listen_port, 8089);
    }

    #[test]
    fn invalid_port_env_is_an_error() {
        assert!(with_env(&[(PORT_ENV, "eighty")]).is_err());
        assert!(with_env(&[(PORT_ENV, "70000")]).is_err());
    }
}
