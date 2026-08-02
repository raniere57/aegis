use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{params, Connection};

pub struct ListMetaDb {
    conn: Connection,
}

/// Last known state of one list URL.
pub struct ListSourceStat {
    pub url: String,
    pub domain_count: i64,
    pub last_success_unix: Option<i64>,
    pub last_error: Option<String>,
}

impl ListMetaDb {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path).context("open meta.sqlite")?;
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS list_sources (
                url TEXT PRIMARY KEY NOT NULL,
                etag TEXT,
                last_modified TEXT,
                last_success_unix INTEGER,
                last_error TEXT,
                domain_count INTEGER DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS meta_kv (
                key TEXT PRIMARY KEY NOT NULL,
                value TEXT NOT NULL
            );
            "#,
        )?;
        Ok(Self { conn })
    }

    pub fn get_etag(&self, url: &str) -> Result<Option<(Option<String>, Option<String>)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT etag, last_modified FROM list_sources WHERE url = ?1")?;
        let mut rows = stmt.query(params![url])?;
        if let Some(row) = rows.next()? {
            let etag: Option<String> = row.get(0)?;
            let lm: Option<String> = row.get(1)?;
            Ok(Some((etag, lm)))
        } else {
            Ok(None)
        }
    }

    pub fn upsert_success(
        &self,
        url: &str,
        etag: Option<&str>,
        last_modified: Option<&str>,
        domain_count: i64,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        self.conn.execute(
            r#"
            INSERT INTO list_sources (url, etag, last_modified, last_success_unix, last_error, domain_count)
            VALUES (?1, ?2, ?3, ?4, NULL, ?5)
            ON CONFLICT(url) DO UPDATE SET
                etag = excluded.etag,
                last_modified = excluded.last_modified,
                last_success_unix = excluded.last_success_unix,
                last_error = NULL,
                domain_count = excluded.domain_count
            "#,
            params![url, etag, last_modified, now, domain_count],
        )?;
        Ok(())
    }

    pub fn set_error(&self, url: &str, err: &str) -> Result<()> {
        self.conn.execute(
            r#"
            INSERT INTO list_sources (url, last_error)
            VALUES (?1, ?2)
            ON CONFLICT(url) DO UPDATE SET last_error = excluded.last_error
            "#,
            params![url, err],
        )?;
        Ok(())
    }

    pub fn set_kv(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO meta_kv (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn get_kv(&self, key: &str) -> Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT value FROM meta_kv WHERE key = ?1")?;
        let mut rows = stmt.query(params![key])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row.get(0)?))
        } else {
            Ok(None)
        }
    }

    /// Per-URL stats: (url, domain_count, last_success_unix, last_error).
    pub fn stats_for_urls(&self, urls: &[String]) -> Result<Vec<ListSourceStat>> {
        let mut out = Vec::with_capacity(urls.len());
        let mut stmt = self.conn.prepare(
            "SELECT domain_count, last_success_unix, last_error FROM list_sources WHERE url = ?1",
        )?;
        for url in urls {
            let mut rows = stmt.query(params![url])?;
            let stat = match rows.next()? {
                Some(row) => ListSourceStat {
                    url: url.clone(),
                    domain_count: row.get(0)?,
                    last_success_unix: row.get(1)?,
                    last_error: row.get(2)?,
                },
                None => ListSourceStat {
                    url: url.clone(),
                    domain_count: 0,
                    last_success_unix: None,
                    last_error: None,
                },
            };
            out.push(stat);
        }
        Ok(out)
    }
}
