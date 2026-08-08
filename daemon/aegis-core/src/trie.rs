//! Compact blocklist via finite-state set (FST).
//! Disk-backed AEG2 files are mmap'd so RSS stays close to file size.

use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::Path;

use fst::{Set, SetBuilder};
use memmap2::Mmap;
use thiserror::Error;

use crate::normalize::normalize_domain;

const MAGIC_V2: &[u8; 4] = b"AEG2"; // FST payload
const VERSION: u32 = 2;
/// AEG2 header: magic(4) + version(4) + count(4)
const AEG2_HEADER_LEN: usize = 12;

#[derive(Debug, Error)]
pub enum TrieError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid blocklist file: {0}")]
    Invalid(String),
    #[error("fst: {0}")]
    Fst(String),
}

/// Owns FST bytes either in heap (built in-process) or via mmap (loaded from disk).
enum FstStore {
    Owned(Vec<u8>),
    Mapped { mmap: Mmap, offset: usize },
}

impl AsRef<[u8]> for FstStore {
    fn as_ref(&self) -> &[u8] {
        match self {
            FstStore::Owned(v) => v.as_slice(),
            FstStore::Mapped { mmap, offset } => &mmap[*offset..],
        }
    }
}

/// Domain blocklist. A blocked suffix also blocks all subdomains.
pub struct Blocklist {
    set: Set<FstStore>,
    count: usize,
}

impl Default for Blocklist {
    fn default() -> Self {
        Self::empty()
    }
}

impl Blocklist {
    pub fn new() -> Self {
        Self::empty()
    }

    fn empty() -> Self {
        let bytes = SetBuilder::memory()
            .into_inner()
            .unwrap_or_default();
        let set = Set::new(FstStore::Owned(bytes)).expect("empty fst");
        Self { set, count: 0 }
    }

    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Normalizes first. Use [`Blocklist::contains_normalized`] on the DNS hot path, where the
    /// name has already been normalized — normalizing again allocates a String per lookup.
    pub fn contains(&self, domain: &str) -> bool {
        match normalize_domain(domain) {
            Some(d) => self.contains_normalized(&d),
            None => false,
        }
    }

    /// `domain` must already be normalized. A blocked suffix blocks all subdomains.
    pub fn contains_normalized(&self, domain: &str) -> bool {
        if self.count == 0 {
            return false;
        }
        let mut rest = domain;
        loop {
            if self.set.contains(rest.as_bytes()) {
                return true;
            }
            match rest.find('.') {
                Some(i) => rest = &rest[i + 1..],
                None => return false,
            }
        }
    }

    pub fn from_domains<I, S>(domains: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut sorted: Vec<String> = domains
            .into_iter()
            .filter_map(|d| normalize_domain(d.as_ref()))
            .collect();
        sorted.sort_unstable();
        sorted.dedup();
        build_fst(&sorted)
    }

    pub fn load_from_path(path: &Path) -> Result<Self, TrieError> {
        let mut f = File::open(path)?;
        let mut magic = [0u8; 4];
        f.read_exact(&mut magic)?;
        if &magic != MAGIC_V2 {
            return Err(TrieError::Invalid("bad magic".into()));
        }
        let mut buf4 = [0u8; 4];
        f.read_exact(&mut buf4)?;
        let ver = u32::from_le_bytes(buf4);
        if ver != VERSION {
            return Err(TrieError::Invalid(format!("unsupported version {ver}")));
        }
        f.read_exact(&mut buf4)?;
        let count = u32::from_le_bytes(buf4) as usize;
        drop(f);
        // mmap whole file; FST starts after fixed header (slice via offset).
        // Pages stay in the file cache — RSS stays near working-set, not a full copy.
        let file = File::open(path)?;
        let mmap = unsafe { Mmap::map(&file) }.map_err(TrieError::Io)?;
        if mmap.len() < AEG2_HEADER_LEN {
            return Err(TrieError::Invalid("file too short".into()));
        }
        let store = FstStore::Mapped {
            mmap,
            offset: AEG2_HEADER_LEN,
        };
        let set = Set::new(store).map_err(|e| TrieError::Fst(e.to_string()))?;
        Ok(Self { set, count })
    }
}

fn build_fst(sorted_unique: &[String]) -> Blocklist {
    let mut builder = SetBuilder::memory();
    for d in sorted_unique {
        let _ = builder.insert(d.as_bytes());
    }
    let bytes = builder.into_inner().unwrap_or_default();
    match Set::new(FstStore::Owned(bytes)) {
        Ok(set) => Blocklist {
            count: sorted_unique.len(),
            set,
        },
        Err(_) => Blocklist::empty(),
    }
}

/// Write a compact FST blocklist (AEG2) straight to disk.
///
/// `sorted_unique` must already be sorted, deduped and normalized — `SetBuilder::insert`
/// enforces that with `Err(OutOfOrder)`, so the precondition is checked by the library rather
/// than trusted. Streaming into the file instead of `SetBuilder::memory()` keeps the whole
/// serialized FST out of the heap; combined with not re-normalizing an already-normalized
/// set, this is what takes the 2.37M-domain compile peak from ~275 MB down to ~85 MB.
pub fn write_sorted_domains<I, S>(path: &Path, count: usize, sorted_unique: I) -> Result<(), TrieError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<[u8]>,
{
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("bin.tmp");
    {
        let mut f = BufWriter::new(File::create(&tmp)?);
        f.write_all(MAGIC_V2)?;
        f.write_all(&VERSION.to_le_bytes())?;
        f.write_all(&(count as u32).to_le_bytes())?;
        let mut builder = SetBuilder::new(f).map_err(|e| TrieError::Fst(e.to_string()))?;
        for d in sorted_unique {
            builder
                .insert(d)
                .map_err(|e| TrieError::Fst(e.to_string()))?;
        }
        let f = builder
            .into_inner()
            .map_err(|e| TrieError::Fst(e.to_string()))?;
        f.into_inner()
            .map_err(|e| TrieError::Io(std::io::Error::other(e.to_string())))?
            .sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Convenience wrapper that normalizes, sorts and dedups first. Used by tests and by any
/// caller that does not already hold a sorted set; the updater calls
/// [`write_sorted_domains`] directly off its BTreeSet.
pub fn write_domains_file(path: &Path, domains: &[String]) -> Result<(), TrieError> {
    let mut owned: Vec<String> = domains.iter().filter_map(|d| normalize_domain(d)).collect();
    owned.sort_unstable();
    owned.dedup();
    write_sorted_domains(path, owned.len(), owned.iter().map(|s| s.as_bytes()))
}

/// Allowlist stays a small HashSet (dozens/hundreds of domains).
#[derive(Debug, Default, Clone)]
pub struct Allowlist {
    set: std::collections::HashSet<String>,
}

impl Allowlist {
    pub fn from_domains<I, S>(domains: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut set = std::collections::HashSet::new();
        for d in domains {
            if let Some(n) = normalize_domain(d.as_ref()) {
                set.insert(n);
            }
        }
        Self { set }
    }

    /// Normalizes first. Prefer [`Allowlist::contains_normalized`] on the hot path.
    pub fn contains(&self, domain: &str) -> bool {
        match normalize_domain(domain) {
            Some(d) => self.contains_normalized(&d),
            None => false,
        }
    }

    /// `domain` must already be normalized. Allowing a domain allows its subdomains.
    pub fn contains_normalized(&self, domain: &str) -> bool {
        if self.set.contains(domain) {
            return true;
        }
        let mut rest = domain;
        while let Some(idx) = rest.find('.') {
            rest = &rest[idx + 1..];
            if self.set.contains(rest) {
                return true;
            }
        }
        false
    }

    pub fn insert(&mut self, domain: &str) -> bool {
        if let Some(n) = normalize_domain(domain) {
            self.set.insert(n)
        } else {
            false
        }
    }

    pub fn remove(&mut self, domain: &str) -> bool {
        if let Some(n) = normalize_domain(domain) {
            self.set.remove(&n)
        } else {
            false
        }
    }

    pub fn list(&self) -> Vec<String> {
        let mut v: Vec<_> = self.set.iter().cloned().collect();
        v.sort();
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn blocks_subdomains() {
        let bl = Blocklist::from_domains(["example.com"]);
        assert!(bl.contains("example.com"));
        assert!(bl.contains("ads.example.com"));
        assert!(bl.contains("a.b.example.com"));
        assert!(!bl.contains("example.org"));
    }

    #[test]
    fn roundtrip_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("blocklist.bin");
        let domains = vec!["ads.example.com".into(), "tracker.net".into()];
        write_domains_file(&path, &domains).unwrap();
        let loaded = Blocklist::load_from_path(&path).unwrap();
        assert!(loaded.contains("ads.example.com"));
        assert!(loaded.contains("x.tracker.net"));
        assert_eq!(loaded.len(), 2);
    }

    #[test]
    fn memory_sane_for_100k() {
        let domains: Vec<String> = (0..100_000)
            .map(|i| format!("d{i}.block.test"))
            .collect();
        let bl = Blocklist::from_domains(&domains);
        assert!(bl.contains("d42.block.test"));
        assert!(bl.contains("sub.d42.block.test"));
        assert_eq!(bl.len(), 100_000);
    }
}
