use std::num::NonZeroUsize;
use std::time::{Duration, Instant};

use lru::LruCache;
use parking_lot::Mutex;

#[derive(Clone, Hash, Eq, PartialEq, Debug)]
pub struct CacheKey {
    pub name: String,
    pub qtype: u16,
    pub qclass: u16,
}

struct CacheEntry {
    inserted: Instant,
    /// Original TTL when stored (seconds).
    ttl_secs: u32,
    /// Wire-format DNS message (with original id; caller rewrites id).
    message: Vec<u8>,
    is_nxdomain: bool,
}

pub struct DnsCache {
    inner: Mutex<LruCache<CacheKey, CacheEntry>>,
    nxdomain_ttl_secs: u32,
}

impl DnsCache {
    pub fn new(capacity: usize, nxdomain_ttl_secs: u32) -> Self {
        let cap = NonZeroUsize::new(capacity.max(1)).unwrap();
        Self {
            inner: Mutex::new(LruCache::new(cap)),
            nxdomain_ttl_secs,
        }
    }

    pub fn get(&self, key: &CacheKey) -> Option<Vec<u8>> {
        let mut guard = self.inner.lock();
        let entry = guard.get(key)?;
        let max_age = if entry.is_nxdomain {
            self.nxdomain_ttl_secs
        } else {
            entry.ttl_secs.max(1)
        };
        let age = entry.inserted.elapsed().as_secs() as u32;
        if age >= max_age {
            guard.pop(key);
            return None;
        }
        let remaining = max_age.saturating_sub(age);
        Some(adjust_ttls(&entry.message, remaining))
    }

    pub fn insert(&self, key: CacheKey, message: Vec<u8>, ttl_secs: u32, is_nxdomain: bool) {
        let ttl = if is_nxdomain {
            ttl_secs.min(self.nxdomain_ttl_secs).max(1)
        } else {
            ttl_secs.max(1)
        };
        self.inner.lock().put(
            key,
            CacheEntry {
                inserted: Instant::now(),
                ttl_secs: ttl,
                message,
                is_nxdomain,
            },
        );
    }

    pub fn clear(&self) {
        self.inner.lock().clear();
    }
}

/// Best-effort: rewrite answer/authority/additional TTLs to `remaining`.
fn adjust_ttls(msg: &[u8], remaining: u32) -> Vec<u8> {
    let mut out = msg.to_vec();
    if out.len() < 12 {
        return out;
    }
    let qdcount = u16::from_be_bytes([out[4], out[5]]) as usize;
    let ancount = u16::from_be_bytes([out[6], out[7]]) as usize;
    let nscount = u16::from_be_bytes([out[8], out[9]]) as usize;
    let arcount = u16::from_be_bytes([out[10], out[11]]) as usize;

    let mut i = 12usize;
    // Skip questions
    for _ in 0..qdcount {
        if !skip_name(&out, &mut i) {
            return out;
        }
        i = i.saturating_add(4); // type + class
        if i > out.len() {
            return out;
        }
    }
    let total_rr = ancount + nscount + arcount;
    for _ in 0..total_rr {
        if !skip_name(&out, &mut i) {
            return out;
        }
        if i + 10 > out.len() {
            return out;
        }
        // type(2) class(2) ttl(4) rdlen(2)
        out[i + 4] = (remaining >> 24) as u8;
        out[i + 5] = (remaining >> 16) as u8;
        out[i + 6] = (remaining >> 8) as u8;
        out[i + 7] = remaining as u8;
        let rdlen = u16::from_be_bytes([out[i + 8], out[i + 9]]) as usize;
        i += 10 + rdlen;
        if i > out.len() {
            return out;
        }
    }
    let _ = Duration::from_secs(remaining as u64);
    out
}

fn skip_name(buf: &[u8], i: &mut usize) -> bool {
    loop {
        if *i >= buf.len() {
            return false;
        }
        let len = buf[*i];
        if len == 0 {
            *i += 1;
            return true;
        }
        if len & 0xC0 == 0xC0 {
            // compression pointer
            if *i + 1 >= buf.len() {
                return false;
            }
            *i += 2;
            return true;
        }
        *i += 1 + len as usize;
        if *i > buf.len() {
            return false;
        }
    }
}
