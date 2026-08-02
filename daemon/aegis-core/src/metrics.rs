use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

#[derive(Debug, Default)]
pub struct Metrics {
    pub queries: AtomicU64,
    pub blocked: AtomicU64,
    pub cache_hit: AtomicU64,
    pub cache_miss: AtomicU64,
    pub upstream_ok: AtomicU64,
    pub upstream_errors: AtomicU64,
    pub started_at_unix: AtomicU64,
    pub filtering: AtomicBool,
}

impl Metrics {
    pub fn new(filtering: bool) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Self {
            started_at_unix: AtomicU64::new(now),
            filtering: AtomicBool::new(filtering),
            ..Self::default()
        }
    }

    pub fn snapshot(&self) -> MetricsSnapshot {
        let started = self.started_at_unix.load(Ordering::Relaxed);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(started);
        MetricsSnapshot {
            queries: self.queries.load(Ordering::Relaxed),
            blocked: self.blocked.load(Ordering::Relaxed),
            cache_hit: self.cache_hit.load(Ordering::Relaxed),
            cache_miss: self.cache_miss.load(Ordering::Relaxed),
            upstream_ok: self.upstream_ok.load(Ordering::Relaxed),
            upstream_errors: self.upstream_errors.load(Ordering::Relaxed),
            uptime_secs: now.saturating_sub(started),
            filtering: self.filtering.load(Ordering::Relaxed),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MetricsSnapshot {
    pub queries: u64,
    pub blocked: u64,
    pub cache_hit: u64,
    pub cache_miss: u64,
    pub upstream_ok: u64,
    pub upstream_errors: u64,
    pub uptime_secs: u64,
    pub filtering: bool,
}
