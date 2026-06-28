//! Application configuration loaded from TOML with sensible defaults.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub paths: PathsConfig,
    pub auth: AuthConfig,
    #[serde(default)]
    pub monitor: MonitorConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// HTTP listen address (e.g. "127.0.0.1:8080").
    #[serde(default = "default_listen")]
    pub listen: String,
    /// Filesystem path to the static web assets.
    #[serde(default = "default_web_root")]
    pub web_root: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathsConfig {
    #[serde(default = "default_db")]
    pub db: PathBuf,
    #[serde(default = "default_audit")]
    pub audit: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    /// Session lifetime in seconds.
    #[serde(default = "default_session_ttl")]
    pub session_ttl: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorConfig {
    /// Enable the background metric collector.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Sampling interval in seconds.
    #[serde(default = "default_interval")]
    pub interval_sec: u64,
    /// How long to keep samples (days).
    #[serde(default = "default_retention")]
    pub retention_days: u64,
}

impl Default for MonitorConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            interval_sec: default_interval(),
            retention_days: default_retention(),
        }
    }
}

fn default_true() -> bool { true }
fn default_interval() -> u64 { 30 }
fn default_retention() -> u64 { 30 }

impl Default for Config {
    fn default() -> Self {
        Self {
            server: ServerConfig {
                listen: default_listen(),
                web_root: default_web_root(),
            },
            paths: PathsConfig {
                db: default_db(),
                audit: default_audit(),
            },
            auth: AuthConfig {
                session_ttl: default_session_ttl(),
            },
            monitor: MonitorConfig::default(),
        }
    }
}

fn default_listen() -> String {
    "127.0.0.1:8080".into()
}
fn default_web_root() -> PathBuf {
    PathBuf::from("/usr/local/share/fwp/web")
}
fn default_db() -> PathBuf {
    PathBuf::from("/var/db/fwp/fwp.db")
}
fn default_audit() -> PathBuf {
    PathBuf::from("/var/db/fwp/audit.log")
}
fn default_session_ttl() -> u64 {
    8 * 3600
}

impl Config {
    /// Load config from the given path; if it does not exist, write the
    /// default config there and return it.
    pub fn load_or_create(path: &std::path::Path) -> anyhow::Result<Self> {
        if !path.exists() {
            let cfg = Config::default();
            let toml = toml::to_string_pretty(&cfg)
                .map_err(|e| anyhow::anyhow!("serialize config: {e}"))?;
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).ok();
            }
            std::fs::write(path, toml).ok();
            return Ok(cfg);
        }
        let raw = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("read config {}: {e}", path.display()))?;
        let cfg: Config = toml::from_str(&raw)
            .map_err(|e| anyhow::anyhow!("parse config {}: {e}", path.display()))?;
        Ok(cfg)
    }
}
