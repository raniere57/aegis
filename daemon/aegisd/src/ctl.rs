use std::sync::Arc;
use std::sync::atomic::Ordering;

use aegis_core::cache::DnsCache;
use aegis_core::config::Config;
use aegis_core::dns::FilterState;
use aegis_core::ipc::{encode_line, parse_line, Request, Response};
use aegis_core::metrics::Metrics;
use aegis_core::paths::AegisPaths;
use aegis_core::trie::{Allowlist, Blocklist};
use aegis_core::VERSION;
use aegis_lists::{ListMetaDb, UpdateOutcome, Updater};
use anyhow::Result;
use arc_swap::ArcSwap;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

pub async fn serve_control(
    paths: AegisPaths,
    config: Arc<ArcSwap<Config>>,
    metrics: Arc<Metrics>,
    filter: Arc<FilterState>,
    updater: Arc<Updater>,
    cache: Arc<DnsCache>,
) -> Result<()> {
    let _ = std::fs::remove_file(&paths.socket);
    if let Some(parent) = paths.socket.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let listener = UnixListener::bind(&paths.socket)?;
    // The control socket can rewrite upstream DNS servers, so it must not be world-writable.
    // Restrict to root + the `admin` group (gid 80 on macOS), which is who runs the GUI app.
    // ponytail: group-based ACL; switch to LOCAL_PEERCRED uid checks if non-admin users need it.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        const ADMIN_GID: u32 = 80;
        let _ = std::os::unix::fs::chown(&paths.socket, None, Some(ADMIN_GID));
        let _ = std::fs::set_permissions(&paths.socket, std::fs::Permissions::from_mode(0o660));
    }
    tracing::info!(path = %paths.socket.display(), "control socket listening");

    loop {
        let (stream, _) = listener.accept().await?;
        let paths = paths.clone();
        let config = Arc::clone(&config);
        let metrics = Arc::clone(&metrics);
        let filter = Arc::clone(&filter);
        let updater = Arc::clone(&updater);
        let cache = Arc::clone(&cache);
        tokio::spawn(async move {
            if let Err(e) = handle_client(stream, paths, config, metrics, filter, updater, cache).await
            {
                tracing::debug!(error = %e, "control client error");
            }
        });
    }
}

async fn handle_client(
    stream: UnixStream,
    paths: AegisPaths,
    config: Arc<ArcSwap<Config>>,
    metrics: Arc<Metrics>,
    filter: Arc<FilterState>,
    updater: Arc<Updater>,
    cache: Arc<DnsCache>,
) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let req = match parse_line(&line) {
            Ok(r) => r,
            Err(e) => {
                let resp = Response::err("0", "bad_request", e.to_string());
                writer.write_all(encode_line(&resp)?.as_bytes()).await?;
                continue;
            }
        };
        let resp = dispatch(req, &paths, &config, &metrics, &filter, &updater, &cache).await;
        writer.write_all(encode_line(&resp)?.as_bytes()).await?;
    }
    Ok(())
}

async fn dispatch(
    req: Request,
    paths: &AegisPaths,
    config: &Arc<ArcSwap<Config>>,
    metrics: &Arc<Metrics>,
    filter: &Arc<FilterState>,
    updater: &Arc<Updater>,
    cache: &Arc<DnsCache>,
) -> Response {
    let id = req.id.clone();
    match req.method.as_str() {
        "ping" => Response::ok(id, json!({"pong": true, "version": VERSION})),
        "metrics" => {
            let s = metrics.snapshot();
            Response::ok(id, serde_json::to_value(s).unwrap_or(json!({})))
        }
        "status" => {
            let snap = metrics.snapshot();
            let err = filter.last_update_error.load();
            Response::ok(
                id,
                json!({
                    "enabled": config.load().daemon.enabled,
                    "filtering": snap.filtering,
                    "uptime_secs": snap.uptime_secs,
                    "listen": config.load().daemon.listen,
                    "domain_count": filter.domain_count.load(Ordering::Relaxed),
                    "list_updated_at": filter.list_updated_at_unix.load(Ordering::Relaxed),
                    "last_update_error": err.as_ref().as_ref(),
                    "version": VERSION,
                    "socket": paths.socket.display().to_string(),
                }),
            )
        }
        "set_enabled" => {
            let enabled = req
                .params
                .get("enabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let mut cfg = (**config.load()).clone();
            cfg.daemon.enabled = enabled;
            if let Err(e) = cfg.save(&paths.config) {
                return Response::err(id, "io", e.to_string());
            }
            config.store(Arc::new(cfg));
            metrics.filtering.store(enabled, Ordering::Relaxed);
            Response::ok(id, json!({"enabled": enabled}))
        }
        "reload_config" => match Config::load(&paths.config) {
            Ok(cfg) => {
                let allow = Allowlist::from_domains(&cfg.allowlist.domains);
                filter.allowlist.store(Arc::new(allow));
                metrics
                    .filtering
                    .store(cfg.daemon.enabled, Ordering::Relaxed);
                config.store(Arc::new(cfg));
                Response::ok(id, json!({"reloaded": true}))
            }
            Err(e) => Response::err(id, "io", e.to_string()),
        },
        "reload_lists" => match Blocklist::load_from_path(&paths.blocklist) {
            Ok(bl) => {
                let n = bl.len();
                filter.swap_blocklist(bl);
                Response::ok(id, json!({"domains": n}))
            }
            Err(e) => Response::err(id, "io", e.to_string()),
        },
        "update_lists" => {
            let urls = config.load().lists.urls.clone();
            let updater = Arc::clone(updater);
            tokio::spawn(async move {
                let _ = updater.update(&urls).await;
            });
            Response::ok(id, json!({"started": true}))
        }
        "get_config" => {
            let cfg = config.load();
            Response::ok(
                id,
                json!({
                    "daemon": {"enabled": cfg.daemon.enabled, "listen": cfg.daemon.listen},
                    "upstream": cfg.upstream,
                    "cache": cfg.cache,
                    "lists": cfg.lists,
                    "allowlist": cfg.allowlist,
                }),
            )
        }
        "patch_config" => match patch_config(paths, config, filter, metrics, &req.params) {
            Ok(()) => Response::ok(id, json!({"saved": true})),
            Err(e) => Response::err(id, "bad_request", e),
        },
        "allowlist.add" => {
            let Some(domain) = req.params.get("domain").and_then(|v| v.as_str()) else {
                return Response::err(id, "bad_request", "missing domain");
            };
            mutate_allowlist(paths, config, filter, |a| {
                a.insert(domain);
            });
            let domains = filter.allowlist.load().list();
            Response::ok(id, json!({"domains": domains}))
        }
        "allowlist.remove" => {
            let Some(domain) = req.params.get("domain").and_then(|v| v.as_str()) else {
                return Response::err(id, "bad_request", "missing domain");
            };
            mutate_allowlist(paths, config, filter, |a| {
                a.remove(domain);
            });
            let domains = filter.allowlist.load().list();
            Response::ok(id, json!({"domains": domains}))
        }
        "allowlist.list" => {
            let domains = filter.allowlist.load().list();
            Response::ok(id, json!({"domains": domains}))
        }
        "lists.add_url" => {
            let Some(url) = req.params.get("url").and_then(|v| v.as_str()) else {
                return Response::err(id, "bad_request", "missing url");
            };
            let mut cfg = (**config.load()).clone();
            if !cfg.lists.urls.iter().any(|u| u == url) {
                cfg.lists.urls.push(url.to_string());
            }
            let _ = cfg.save(&paths.config);
            config.store(Arc::new(cfg));
            Response::ok(id, lists_payload(paths, config, filter))
        }
        "lists.remove_url" => {
            let Some(url) = req.params.get("url").and_then(|v| v.as_str()) else {
                return Response::err(id, "bad_request", "missing url");
            };
            let mut cfg = (**config.load()).clone();
            cfg.lists.urls.retain(|u| u != url);
            let _ = cfg.save(&paths.config);
            config.store(Arc::new(cfg));
            Response::ok(id, lists_payload(paths, config, filter))
        }
        "lists.list" => Response::ok(id, lists_payload(paths, config, filter)),
        "cache.clear" => {
            cache.clear();
            Response::ok(id, json!({"cleared": true}))
        }
        other => Response::err(id, "unknown_method", format!("unknown method: {other}")),
    }
}

fn lists_payload(
    paths: &AegisPaths,
    config: &Arc<ArcSwap<Config>>,
    filter: &Arc<FilterState>,
) -> Value {
    let cfg = config.load();
    let unique = filter.domain_count.load(Ordering::Relaxed);
    let mut sources = Vec::new();
    let mut sum: i64 = 0;
    if let Ok(db) = ListMetaDb::open(&paths.meta_db) {
        if let Ok(rows) = db.stats_for_urls(&cfg.lists.urls) {
            for s in rows {
                sum += s.domain_count;
                sources.push(json!({
                    "url": s.url,
                    "domain_count": s.domain_count,
                    "last_success_unix": s.last_success_unix,
                    "last_error": s.last_error,
                }));
            }
        }
    }
    if sources.is_empty() {
        for url in &cfg.lists.urls {
            sources.push(json!({
                "url": url,
                "domain_count": 0,
                "last_success_unix": Value::Null,
                "last_error": Value::Null,
            }));
        }
    }
    json!({
        "urls": cfg.lists.urls,
        "auto_update": cfg.lists.auto_update,
        "interval_hours": cfg.lists.interval_hours,
        "list_count": cfg.lists.urls.len(),
        "unique_domains": unique,
        "sum_domain_counts": sum,
        "sources": sources,
    })
}

fn mutate_allowlist(
    paths: &AegisPaths,
    config: &Arc<ArcSwap<Config>>,
    filter: &Arc<FilterState>,
    f: impl FnOnce(&mut Allowlist),
) {
    let mut allow = (**filter.allowlist.load()).clone();
    f(&mut allow);
    let domains = allow.list();
    filter.allowlist.store(Arc::new(allow));
    let mut cfg = (**config.load()).clone();
    cfg.allowlist.domains = domains;
    let _ = cfg.save(&paths.config);
    config.store(Arc::new(cfg));
}

fn patch_config(
    paths: &AegisPaths,
    config: &Arc<ArcSwap<Config>>,
    filter: &Arc<FilterState>,
    metrics: &Arc<Metrics>,
    params: &Value,
) -> Result<(), String> {
    let mut cfg = (**config.load()).clone();
    if let Some(en) = params.pointer("/daemon/enabled").and_then(|v| v.as_bool()) {
        cfg.daemon.enabled = en;
        metrics.filtering.store(en, Ordering::Relaxed);
    }
    if let Some(servers) = params.pointer("/upstream/servers").and_then(|v| v.as_array()) {
        cfg.upstream.servers = servers
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
    }
    if let Some(ms) = params.pointer("/upstream/timeout_ms").and_then(|v| v.as_u64()) {
        cfg.upstream.timeout_ms = ms;
    }
    if let Some(au) = params.pointer("/lists/auto_update").and_then(|v| v.as_bool()) {
        cfg.lists.auto_update = au;
    }
    if let Some(h) = params
        .pointer("/lists/interval_hours")
        .and_then(|v| v.as_u64())
    {
        cfg.lists.interval_hours = h.clamp(1, 168);
    }
    if let Some(urls) = params.pointer("/lists/urls").and_then(|v| v.as_array()) {
        cfg.lists.urls = urls
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
    }
    if let Some(domains) = params
        .pointer("/allowlist/domains")
        .and_then(|v| v.as_array())
    {
        cfg.allowlist.domains = domains
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
        filter
            .allowlist
            .store(Arc::new(Allowlist::from_domains(&cfg.allowlist.domains)));
    }
    cfg.save(&paths.config).map_err(|e| e.to_string())?;
    config.store(Arc::new(cfg));
    Ok(())
}

// silence unused import warning for UpdateOutcome in some builds
#[allow(dead_code)]
fn _use_outcome(o: UpdateOutcome) {
    let _ = o;
}
