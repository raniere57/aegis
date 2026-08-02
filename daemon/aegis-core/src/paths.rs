use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Runtime filesystem layout. Dev mode uses `~/.aegis/`; privileged uses `/Library/...`.
#[derive(Debug, Clone)]
pub struct AegisPaths {
    pub root: PathBuf,
    pub config: PathBuf,
    pub blocklist: PathBuf,
    pub meta_db: PathBuf,
    pub dns_backup: PathBuf,
    pub socket: PathBuf,
}

impl AegisPaths {
    pub fn dev() -> Self {
        let root = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".aegis");
        Self::from_root(root, true)
    }

    pub fn privileged() -> Self {
        Self::from_root(PathBuf::from("/Library/Application Support/Aegis"), false)
    }

    fn from_root(root: PathBuf, dev: bool) -> Self {
        let socket = if dev {
            root.join("aegis.sock")
        } else {
            PathBuf::from("/var/run/aegis.sock")
        };
        Self {
            config: root.join("config.toml"),
            blocklist: root.join("blocklist.bin"),
            meta_db: root.join("meta.sqlite"),
            dns_backup: root.join("dns-backup.json"),
            socket,
            root,
        }
    }

    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.root)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsBackup {
    pub saved_at: String,
    pub services: Vec<DnsServiceBackup>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsServiceBackup {
    pub name: String,
    pub servers: Vec<String>,
}
