//! Aegis core: config, metrics, blocklist trie, cache, DNS proxy, IPC types.

pub mod cache;
pub mod config;
pub mod dns;
pub mod ipc;
pub mod metrics;
pub mod normalize;
pub mod paths;
pub mod trie;

pub use cache::DnsCache;
pub use config::Config;
pub use metrics::Metrics;
pub use paths::AegisPaths;
pub use trie::{Allowlist, Blocklist};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
