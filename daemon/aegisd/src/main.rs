mod ctl;
mod scheduler;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use aegis_core::config::Config;
use aegis_core::dns::{DnsProxy, FilterState};
use aegis_core::metrics::Metrics;
use aegis_core::paths::AegisPaths;
use aegis_core::trie::{Allowlist, Blocklist};
use aegis_lists::Updater;
use anyhow::{Context, Result};
use arc_swap::ArcSwap;
use clap::Parser;
use tracing_subscriber::EnvFilter;

use crate::ctl::serve_control;
use crate::scheduler::spawn_auto_update;

#[derive(Parser, Debug)]
#[command(name = "aegisd", version, about = "Aegis DNS filter daemon")]
struct Args {
    /// Use ~/.aegis and port 5353 (no root).
    #[arg(long)]
    dev: bool,

    /// Override config path.
    #[arg(long)]
    config: Option<PathBuf>,

    /// Override listen addresses (comma-separated).
    #[arg(long)]
    listen: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();
    let paths = if args.dev {
        AegisPaths::dev()
    } else {
        AegisPaths::privileged()
    };
    paths.ensure_dirs().context("create data dirs")?;

    let config_path = args.config.unwrap_or_else(|| paths.config.clone());
    let mut config = Config::load(&config_path).context("load config")?;
    if !args.dev && args.listen.is_none() {
        // Prefer privileged ports when not in --dev, unless config already customized.
        if config.daemon.listen.len() == 1
            && (config.daemon.listen[0].ends_with(":5353")
                || config.daemon.listen[0].ends_with(":53553"))
        {
            config = config.with_privileged_listen();
        }
    }
    if let Some(listen) = args.listen {
        config.daemon.listen = listen
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
    }
    config.save(&config_path)?;

    let filtering = config.daemon.enabled;
    let metrics = Arc::new(Metrics::new(filtering));
    let mut allowlist = Allowlist::from_domains(&config.allowlist.domains);
    // Inject list hostnames into allowlist so updates never self-block
    for url in &config.lists.urls {
        if let Some(host) = host_from_url(url) {
            allowlist.insert(&host);
        }
    }

    let filter = Arc::new(FilterState::new(Blocklist::new(), allowlist));
    let config = Arc::new(ArcSwap::from_pointee(config));

    let updater = Arc::new(
        Updater::new(
            paths.meta_db.clone(),
            paths.blocklist.clone(),
            Arc::clone(&filter),
        )
        .context("create updater")?,
    );
    if let Err(e) = updater.load_existing() {
        tracing::warn!(error = %e, "could not load existing blocklist (fail-open)");
    }

    let proxy = Arc::new(DnsProxy::new(
        Arc::clone(&config),
        Arc::clone(&metrics),
        Arc::clone(&filter),
    ));

    let listen_addrs: Vec<SocketAddr> = config
        .load()
        .daemon
        .listen
        .iter()
        .filter_map(|s| s.parse().ok())
        .collect();
    if listen_addrs.is_empty() {
        anyhow::bail!("no valid listen addresses");
    }

    // Control socket
    let ctl_paths = paths.clone();
    let ctl_config = Arc::clone(&config);
    let ctl_metrics = Arc::clone(&metrics);
    let ctl_filter = Arc::clone(&filter);
    let ctl_updater = Arc::clone(&updater);
    let ctl_proxy_cache = Arc::clone(&proxy.cache);
    tokio::spawn(async move {
        if let Err(e) = serve_control(
            ctl_paths,
            ctl_config,
            ctl_metrics,
            ctl_filter,
            ctl_updater,
            ctl_proxy_cache,
        )
        .await
        {
            tracing::error!(error = %e, "control server exited");
        }
    });

    // Auto-update scheduler
    spawn_auto_update(Arc::clone(&config), Arc::clone(&updater));

    // Initial update only if no blocklist yet (avoid huge RAM spike + network on every boot)
    {
        let urls = config.load().lists.urls.clone();
        let upd = Arc::clone(&updater);
        let has_list = paths.blocklist.exists();
        tokio::spawn(async move {
            if has_list {
                tracing::info!("existing blocklist found; skipping immediate update");
                return;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
            let outcome = upd.update(&urls).await;
            tracing::info!(?outcome, "initial list update");
        });
    }

    tracing::info!(
        version = aegis_core::VERSION,
        dev = args.dev,
        "aegisd starting"
    );
    proxy.run(listen_addrs).await?;
    Ok(())
}

fn host_from_url(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let host = rest.split('/').next()?.split('@').next_back()?;
    let host = host.split(':').next()?.trim();
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}
