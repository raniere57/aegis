use std::sync::Arc;
use std::time::Duration;

use aegis_core::config::Config;
use aegis_lists::Updater;
use arc_swap::ArcSwap;
use tracing::info;

/// Spawn background auto-update loop with interval + jitter (0–30 min).
pub fn spawn_auto_update(config: Arc<ArcSwap<Config>>, updater: Arc<Updater>) {
    tokio::spawn(async move {
        // Stagger first run a bit after boot (initial update already kicked once).
        loop {
            let cfg = config.load();
            if !cfg.lists.auto_update {
                tokio::time::sleep(Duration::from_secs(60)).await;
                continue;
            }
            let hours = cfg.lists.interval_hours.clamp(1, 168);
            let base = Duration::from_secs(hours * 3600);
            let jitter_secs = fastrand_u64(0, 30 * 60);
            let sleep_for = base + Duration::from_secs(jitter_secs);
            info!(
                hours,
                jitter_secs,
                "next auto-update scheduled"
            );
            sleep_until_wall_clock(sleep_for).await;
            let urls = config.load().lists.urls.clone();
            if !config.load().lists.auto_update {
                continue;
            }
            let outcome = updater.update(&urls).await;
            info!(?outcome, "auto-update finished");
        }
    });
}

/// tokio's timer runs on a monotonic clock that macOS freezes while the machine sleeps, so a
/// plain `sleep(24h)` on a laptop that is closed every night can take days of real time to fire.
/// Wake often and compare against the wall clock instead.
async fn sleep_until_wall_clock(total: Duration) {
    const TICK: Duration = Duration::from_secs(60);
    let deadline = std::time::SystemTime::now() + total;
    loop {
        let remaining = match deadline.duration_since(std::time::SystemTime::now()) {
            Ok(d) => d,
            Err(_) => return, // deadline already passed (or the clock jumped forward)
        };
        if remaining.is_zero() {
            return;
        }
        tokio::time::sleep(remaining.min(TICK)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn wall_clock_sleep_returns_after_the_requested_span() {
        let started = std::time::Instant::now();
        sleep_until_wall_clock(Duration::from_millis(150)).await;
        assert!(started.elapsed() >= Duration::from_millis(150));
        // Must not round up to the 60s tick.
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn zero_and_past_deadlines_return_immediately() {
        let started = std::time::Instant::now();
        sleep_until_wall_clock(Duration::ZERO).await;
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}

fn fastrand_u64(min: u64, max_inclusive: u64) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    std::time::SystemTime::now().hash(&mut h);
    std::process::id().hash(&mut h);
    let r = h.finish();
    if max_inclusive <= min {
        return min;
    }
    min + (r % (max_inclusive - min + 1))
}
