use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use aegis_core::dns::FilterState;
use aegis_core::trie::{write_sorted_domains, Blocklist};
use anyhow::{bail, Context, Result};
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
    /// Held for the duration of an update. A plain bool + manual reset leaked the flag on any
    /// panic, wedging every future update at "already running" until the daemon restarted.
    busy: tokio::sync::Mutex<()>,
    client: reqwest::Client,
}

impl Updater {
    pub fn new(
        meta_path: PathBuf,
        blocklist_path: PathBuf,
        filter: Arc<FilterState>,
    ) -> Result<Self> {
        let client = reqwest::Client::builder()
            // 60s total is not enough for a 20 MB list on a slow link (it demands a sustained
            // 340 KB/s); bound the connect separately so a black-holed host fails fast.
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(300))
            // Only https, and bound the chain: root fetches these URLs, and the URL field is
            // user-editable in the UI.
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() >= 3 {
                    attempt.error("too many redirects")
                } else if attempt.url().scheme() != "https" {
                    attempt.error("refusing to follow a non-https redirect")
                } else {
                    attempt.follow()
                }
            }))
            .user_agent(format!("aegis/{}", aegis_core::VERSION))
            .build()?;
        Ok(Self {
            meta_path,
            blocklist_path,
            filter,
            busy: tokio::sync::Mutex::new(()),
            client,
        })
    }

    pub async fn update(&self, urls: &[String]) -> UpdateOutcome {
        let Ok(_guard) = self.busy.try_lock() else {
            return UpdateOutcome::Failed {
                message: "update already running".into(),
            };
        };
        let result = self.update_inner(urls).await;
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
            write_sorted_domains(&self.blocklist_path, 0, std::iter::empty::<&[u8]>())?;
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

        // Conditional pass, genuinely concurrent (the old chunks(4) loop awaited one at a
        // time). Bodies fetched here are KEPT — the old code threw them away and then
        // re-downloaded every list unconditionally, doubling the bytes on the wire.
        let mut tasks = tokio::task::JoinSet::new();
        for (url, etag, lm) in headers {
            let client = self.client.clone();
            tasks.spawn(async move {
                let res = fetch_conditional(&client, &url, etag, lm).await;
                (url, res)
            });
        }
        let mut fetched = Vec::new();
        while let Some(joined) = tasks.join_next().await {
            match joined {
                Ok(pair) => fetched.push(pair),
                Err(e) => warn!(error = %e, "list fetch task failed"),
            }
        }

        let mut fetch_errors = 0usize;
        let mut failed_urls: Vec<String> = Vec::new();
        let mut not_modified: Vec<String> = Vec::new();
        let mut bodies: Vec<(String, String, Option<String>, Option<String>)> = Vec::new();

        for (url, res) in fetched {
            match res {
                Ok(FetchResult::NotModified) => not_modified.push(url),
                Ok(FetchResult::Body {
                    text,
                    etag,
                    last_modified,
                }) => bodies.push((url, text, etag, last_modified)),
                Err(e) => {
                    fetch_errors += 1;
                    warn!(url = %url, error = %e, "list fetch failed");
                    if let Err(db_err) = ListMetaDb::open(&self.meta_path)
                        .and_then(|db| db.set_error(&url, &format!("{e:#}")))
                    {
                        warn!(error = %db_err, "could not record list error");
                    }
                    failed_urls.push(url);
                }
            }
        }

        if bodies.is_empty() && fetch_errors == 0 {
            info!("lists not modified (all 304)");
            // All 304 with no errors is a healthy outcome, so clear any stale error banner.
            self.filter.last_update_error.store(Arc::new(None));
            return Ok(UpdateOutcome::NotModified);
        }

        // Something changed, so we need the full union — but only the 304s still lack a body.
        for url in not_modified {
            match fetch_unconditional(&self.client, &url).await {
                Ok(text) => bodies.push((url, text, None, None)),
                Err(e) => {
                    fetch_errors += 1;
                    warn!(url = %url, error = %e, "list refetch failed");
                    if let Ok(db) = ListMetaDb::open(&self.meta_path) {
                        let _ = db.set_error(&url, &format!("{e:#}"));
                    }
                    failed_urls.push(url);
                }
            }
        }

        // A list that previously contributed domains and failed this run means the rebuild is
        // missing a whole feed. The 50%-shrink heuristic misses this whenever the failing list
        // is the smaller one — e.g. Multi (182k) + TIF, TIF dies, 182k*2 clears the bar.
        if !failed_urls.is_empty() {
            if let Ok(db) = ListMetaDb::open(&self.meta_path) {
                if let Ok(stats) = db.stats_for_urls(&failed_urls) {
                    if let Some(s) = stats.iter().find(|s| s.domain_count > 0) {
                        bail!(
                            "a lista {} falhou e antes contribuía com {} domínios; \
                             mantendo a blocklist anterior.",
                            s.url,
                            s.domain_count
                        );
                    }
                }
            }
        }

        let mut all = BTreeSet::new();
        for (url, text, etag, lm) in &bodies {
            let domains = normalize_list_text(text);
            let count = domains.len() as i64;
            for d in domains {
                all.insert(d);
            }
            if let Ok(db) = ListMetaDb::open(&self.meta_path) {
                let _ = db.upsert_success(url, etag.as_deref(), lm.as_deref(), count);
            }
        }
        // Free the raw list text before building the FST — for ultimate.txt this is tens of MB.
        drop(bodies);

        if all.is_empty() && fetch_errors > 0 {
            bail!("all list fetches failed; keeping previous blocklist");
        }

        let prev_count = self
            .filter
            .domain_count
            .load(std::sync::atomic::Ordering::Relaxed);
        let new_count = all.len();
        // Every URL can answer 200 and still yield nothing parseable — a captive-portal HTML
        // interstitial, or a format the parser does not recognize. fetch_errors is 0 in that
        // case, so this floor must NOT be gated on it: without it we write an empty FST, clear
        // last_update_error, and report a clean update while the filter is silently a no-op.
        if new_count == 0 && prev_count > 0 {
            bail!(
                "as {} lista(s) baixaram, mas nenhum domínio foi reconhecido; \
                 mantendo a blocklist anterior ({prev_count} domínios).",
                urls.len()
            );
        }
        // A large shrink with no failed download is a real upstream change and is adopted.
        // Gated on fetch_errors so one dead URL cannot freeze updates forever.
        if fetch_errors > 0 && prev_count > 100 && new_count * 2 < prev_count {
            bail!(
                "{fetch_errors} de {} lista(s) falharam ao baixar, então a lista reconstruída \
                 ficou pequena demais ({new_count} < 50% dos {prev_count} anteriores); \
                 mantendo a anterior. Veja o erro de cada lista em Ajustes → Listas.",
                urls.len()
            );
        }

        // Stream straight off the BTreeSet: it is already sorted, unique and normalized, so
        // there is no reason to collect it into a Vec or to let the FST buffer in memory.
        write_sorted_domains(&self.blocklist_path, new_count, all.iter().map(|s| s.as_bytes()))?;
        drop(all);
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

