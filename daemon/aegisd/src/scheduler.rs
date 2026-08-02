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
            tokio::time::sleep(sleep_for).await;
            let urls = config.load().lists.urls.clone();
            if !config.load().lists.auto_update {
                continue;
            }
            let outcome = updater.update(&urls).await;
            info!(?outcome, "auto-update finished");
        }
    });
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
