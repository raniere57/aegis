use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("toml: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("toml serialize: {0}")]
    TomlSer(#[from] toml::ser::Error),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub daemon: DaemonConfig,
    #[serde(default)]
    pub upstream: UpstreamConfig,
    #[serde(default)]
    pub cache: CacheConfig,
    #[serde(default)]
    pub lists: ListsConfig,
    #[serde(default)]
    pub allowlist: AllowlistConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    #[serde(default = "default_listen_dev")]
    pub listen: Vec<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpstreamConfig {
    #[serde(default = "default_upstreams")]
    pub servers: Vec<String>,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
    #[serde(default = "default_inflight")]
    pub max_inflight: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheConfig {
    #[serde(default = "default_cache_size")]
    pub size: usize,
    #[serde(default = "default_nx_ttl")]
    pub nxdomain_ttl_secs: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListsConfig {
    #[serde(default = "default_true")]
    pub auto_update: bool,
    #[serde(default = "default_interval")]
    pub interval_hours: u64,
    #[serde(default = "default_list_urls")]
    pub urls: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AllowlistConfig {
    #[serde(default)]
    pub domains: Vec<String>,
}

fn default_true() -> bool {
    true
}
fn default_listen_dev() -> Vec<String> {
    // Avoid 5353 — reserved by mDNS/Bonjour on macOS.
    vec!["127.0.0.1:53553".into()]
}
fn default_upstreams() -> Vec<String> {
    // Keep at least one IPv6 literal: on an IPv6-only / NAT64 network there is no IPv4 route,
    // so an all-IPv4 list makes every cache miss SERVFAIL with no diagnosis.
    vec![
        "1.1.1.1:53".into(),
        "1.0.0.1:53".into(),
        "[2606:4700:4700::1111]:53".into(),
    ]
}
fn default_timeout() -> u64 {
    1500
}
fn default_inflight() -> usize {
    256
}
fn default_cache_size() -> usize {
    8192
}
fn default_nx_ttl() -> u32 {
    45
}
fn default_interval() -> u64 {
    24
}
fn default_list_urls() -> Vec<String> {
    // HaGeZi Multi Normal — plain domain list, actively maintained, good default balance.
    // Serve from raw.githubusercontent: jsdelivr @latest 403s and the old `hosts/` dir is gone.
    vec![
        "https://raw.githubusercontent.com/hagezi/dns-blocklists/main/wildcard/multi-onlydomains.txt"
            .into(),
    ]
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            listen: default_listen_dev(),
            enabled: true,
        }
    }
}

impl Default for UpstreamConfig {
    fn default() -> Self {
        Self {
            servers: default_upstreams(),
            timeout_ms: default_timeout(),
            max_inflight: default_inflight(),
        }
    }
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            size: default_cache_size(),
            nxdomain_ttl_secs: default_nx_ttl(),
        }
    }
}

impl Default for ListsConfig {
    fn default() -> Self {
        Self {
            auto_update: true,
            interval_hours: default_interval(),
            urls: default_list_urls(),
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        if !path.exists() {
            let cfg = Self::default();
            cfg.save(path)?;
            return Ok(cfg);
        }
        let text = fs::read_to_string(path)?;
        Ok(toml::from_str(&text)?)
    }

    /// Write atomically. `fs::write` truncates in place, so a crash or a SIGKILL — which the
    /// installer delivers via `launchctl kickstart -k` — during a save leaves a half-written
    /// or empty config, and the daemon then refuses to start while DNS still points at us.
    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        use std::io::Write;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(self)?;
        let tmp = path.with_extension("toml.tmp");
        {
            let mut f = fs::File::create(&tmp)?;
            f.write_all(text.as_bytes())?;
            f.sync_all()?;
        }
        fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn with_privileged_listen(mut self) -> Self {
        self.daemon.listen = vec!["127.0.0.1:53".into(), "[::1]:53".into()];
        self
    }
}
