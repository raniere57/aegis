use std::collections::BTreeSet;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use aegis_core::dns::FilterState;
use aegis_core::trie::{write_domains_file, Blocklist};
use anyhow::{bail, Context, Result};
use parking_lot::Mutex;
use tracing::{info, warn};

use crate::meta::ListMetaDb;
use crate::normalize_list::normalize_list_text;

const MAX_DOWNLOAD_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone)]
pub enum UpdateOutcome {
    NotModified,
    Updated { domains: usize },
    Failed { message: String },
}

pub struct Updater {
    meta_path: PathBuf,
    blocklist_path: PathBuf,
    filter: Arc<FilterState>,
    busy: Mutex<bool>,
    client: reqwest::Client,
}

impl Updater {
    pub fn new(
        meta_path: PathBuf,
        blocklist_path: PathBuf,
        filter: Arc<FilterState>,
    ) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .user_agent(format!("aegis/{}", aegis_core::VERSION))
            .build()?;
        Ok(Self {
            meta_path,
            blocklist_path,
            filter,
            busy: Mutex::new(false),
            client,
        })
    }

    pub fn try_begin(&self) -> bool {
        let mut g = self.busy.lock();
        if *g {
            return false;
        }
        *g = true;
        true
    }

    pub fn end(&self) {
        *self.busy.lock() = false;
    }

    pub async fn update(&self, urls: &[String]) -> UpdateOutcome {
        if !self.try_begin() {
            return UpdateOutcome::Failed {
                message: "update already running".into(),
            };
        }
        let result = self.update_inner(urls).await;
        self.end();
        match result {
            Ok(o) => o,
            Err(e) => {
                let message = format!("{e:#}");
                self.filter
                    .last_update_error
                    .store(Arc::new(Some(message.clone())));
                UpdateOutcome::Failed { message }
            }
        }
    }

    async fn update_inner(&self, urls: &[String]) -> Result<UpdateOutcome> {
        if urls.is_empty() {
            write_domains_file(&self.blocklist_path, &[])?;
            self.filter.swap_blocklist(Blocklist::new());
            return Ok(UpdateOutcome::Updated { domains: 0 });
        }

        // Read conditional headers without holding DB across await
        let headers: Vec<(String, Option<String>, Option<String>)> = {
            let db = ListMetaDb::open(&self.meta_path)?;
            urls.iter()
                .map(|url| {
                    let (etag, lm) = db
                        .get_etag(url)
                        .ok()
                        .flatten()
                        .unwrap_or((None, None));
                    (url.clone(), etag, lm)
                })
                .collect()
        };

        let mut any_modified = false;
        let mut fetch_errors = 0usize;
        let mut bodies: Vec<(String, String, Option<String>, Option<String>)> = Vec::new();

        for chunk in headers.chunks(4) {
            let mut futs = Vec::new();
            for (url, etag, lm) in chunk {
                let client = self.client.clone();
                let url = url.clone();
                let etag = etag.clone();
                let lm = lm.clone();
                futs.push(async move { fetch_conditional(&client, &url, etag, lm).await });
            }
            for ((url, _, _), res) in chunk.iter().zip(futures_join(futs).await) {
                match res {
                    Ok(FetchResult::NotModified) => {}
                    Ok(FetchResult::Body {
                        text,
                        etag,
                        last_modified,
                    }) => {
                        any_modified = true;
                        bodies.push((url.clone(), text, etag, last_modified));
                    }
                    Err(e) => {
                        fetch_errors += 1;
                        warn!(url = %url, error = %e, "list fetch failed");
                        if let Ok(db) = ListMetaDb::open(&self.meta_path) {
                            let _ = db.set_error(url, &format!("{e:#}"));
                        }
                    }
                }
            }
        }

        if !any_modified && fetch_errors == 0 {
            info!("lists not modified (all 304)");
            return Ok(UpdateOutcome::NotModified);
        }

        // Rebuild full union with unconditional fetches when anything changed
        let mut all = BTreeSet::new();
        for url in urls {
            match fetch_unconditional(&self.client, url).await {
                Ok(text) => {
                    let domains = normalize_list_text(&text);
                    let count = domains.len() as i64;
                    for d in domains {
                        all.insert(d);
                    }
                    if let Ok(db) = ListMetaDb::open(&self.meta_path) {
                        // Prefer etag from earlier body if present
                        let (etag, lm) = bodies
                            .iter()
                            .find(|(u, _, _, _)| u == url)
                            .map(|(_, _, e, l)| (e.clone(), l.clone()))
                            .unwrap_or((None, None));
                        let _ = db.upsert_success(
                            url,
                            etag.as_deref(),
                            lm.as_deref(),
                            count,
                        );
                    }
                }
                Err(e) => {
                    warn!(url = %url, error = %e, "list refetch failed");
                    fetch_errors += 1;
                    if let Ok(db) = ListMetaDb::open(&self.meta_path) {
                        let _ = db.set_error(url, &format!("{e:#}"));
                    }
                }
            }
        }

        if all.is_empty() && fetch_errors > 0 {
            bail!("all list fetches failed; keeping previous blocklist");
        }

        let prev_count = self
            .filter
            .domain_count
            .load(std::sync::atomic::Ordering::Relaxed);
        let new_count = all.len();
        // Only refuse a shrink when a fetch actually failed. If every list downloaded fine,
        // the upstream really did get smaller — otherwise one dead URL freezes updates forever.
        if fetch_errors > 0 && prev_count > 100 && new_count * 2 < prev_count {
            bail!(
                "{fetch_errors} of {} lists failed to download, so the rebuilt list is too small \
                 ({new_count} < 50% of previous {prev_count}); keeping previous. Check each list's \
                 error in Ajustes → Listas.",
                urls.len()
            );
        }

        let domains: Vec<String> = all.into_iter().collect();
        write_domains_file(&self.blocklist_path, &domains)?;
        // Drop the huge String set before mmap-loading the compact FST.
        drop(domains);
        let bl = Blocklist::load_from_path(&self.blocklist_path)?;
        self.filter.swap_blocklist(bl);
        self.filter.last_update_error.store(Arc::new(None));
        if let Ok(db) = ListMetaDb::open(&self.meta_path) {
            let _ = db.set_kv("last_compile_count", &new_count.to_string());
        }
        info!(domains = new_count, "blocklist updated");
        Ok(UpdateOutcome::Updated {
            domains: new_count,
        })
    }

    pub fn load_existing(&self) -> Result<()> {
        if self.blocklist_path.exists() {
            let bl = Blocklist::load_from_path(&self.blocklist_path)?;
            // Rewrite legacy AEGS dumps as compact FST (AEG2) so next boots stay lean.
            if let Ok(mut f) = File::open(&self.blocklist_path) {
                let mut magic = [0u8; 4];
                if f.read_exact(&mut magic).is_ok() && &magic == b"AEGS" {
                    let _ = bl.write_to_path(&self.blocklist_path);
                }
            }
            self.filter.swap_blocklist(bl);
        }
        Ok(())
    }

    pub fn blocklist_path(&self) -> &Path {
        &self.blocklist_path
    }
}

enum FetchResult {
    NotModified,
    Body {
        text: String,
        etag: Option<String>,
        last_modified: Option<String>,
    },
}

async fn fetch_conditional(
    client: &reqwest::Client,
    url: &str,
    etag: Option<String>,
    last_modified: Option<String>,
) -> Result<FetchResult> {
    let mut req = client.get(url);
    if let Some(e) = etag {
        req = req.header(reqwest::header::IF_NONE_MATCH, e);
    }
    if let Some(lm) = last_modified {
        req = req.header(reqwest::header::IF_MODIFIED_SINCE, lm);
    }
    let resp = req.send().await.context("http get")?;
    if resp.status() == reqwest::StatusCode::NOT_MODIFIED {
        return Ok(FetchResult::NotModified);
    }
    if !resp.status().is_success() {
        bail!("HTTP {}", resp.status());
    }
    let etag = resp
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let last_modified = resp
        .headers()
        .get(reqwest::header::LAST_MODIFIED)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let bytes = resp.bytes().await?;
    if bytes.len() > MAX_DOWNLOAD_BYTES {
        bail!("download too large: {} bytes", bytes.len());
    }
    let text = String::from_utf8_lossy(&bytes).into_owned();
    Ok(FetchResult::Body {
        text,
        etag,
        last_modified,
    })
}

async fn fetch_unconditional(client: &reqwest::Client, url: &str) -> Result<String> {
    let resp = client.get(url).send().await?;
    if !resp.status().is_success() {
        bail!("HTTP {}", resp.status());
    }
    let bytes = resp.bytes().await?;
    if bytes.len() > MAX_DOWNLOAD_BYTES {
        bail!("download too large");
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

async fn futures_join<T>(futs: Vec<impl std::future::Future<Output = T>>) -> Vec<T> {
    let mut out = Vec::with_capacity(futs.len());
    for f in futs {
        out.push(f.await);
    }
    out
}
