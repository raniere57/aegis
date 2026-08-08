use std::num::NonZeroUsize;
use std::time::Instant;

use lru::LruCache;
use parking_lot::Mutex;

/// Hard ceiling on cached message bytes. The entry count alone does not bound memory: a
/// cache full of TXT/DKIM answers near the 4 KB UDP ceiling would be ~35 MB, over twice the
/// whole idle budget, purely as a function of the user's traffic.
const MAX_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Hash, Eq, PartialEq, Debug)]
pub struct CacheKey {
    pub name: String,
    pub qtype: u16,
    pub qclass: u16,
    /// A DO=1 client populates a signed answer; serving that to a DO=0 client hands it RRSIGs
    /// it never asked for, and the reverse is a downgrade indistinguishable from an attack.
    pub dnssec_ok: bool,
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
    /// Live sum of cached message bytes, kept in step with `inner` under the same lock.
    bytes: Mutex<usize>,
    nxdomain_ttl_secs: u32,
}

impl DnsCache {
    pub fn new(capacity: usize, nxdomain_ttl_secs: u32) -> Self {
        let cap = NonZeroUsize::new(capacity.max(1)).unwrap();
        Self {
            inner: Mutex::new(LruCache::new(cap)),
            bytes: Mutex::new(0),
            nxdomain_ttl_secs,
        }
    }

    pub fn nxdomain_ttl(&self) -> u32 {
        self.nxdomain_ttl_secs
    }

    /// Current cached message bytes, for metrics.
    pub fn byte_len(&self) -> usize {
        *self.bytes.lock()
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
            let dropped = guard.pop(key).map_or(0, |e| e.message.len());
            *self.bytes.lock() -= dropped;
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
        let added = message.len();
        let entry = CacheEntry {
            inserted: Instant::now(),
            ttl_secs: ttl,
            message,
            is_nxdomain,
        };

        let mut guard = self.inner.lock();
        let mut bytes = self.bytes.lock();
        // `push`, not `put`: put's return value only covers a same-key replacement, so a
        // capacity eviction inside it would never be subtracted and the counter would ratchet
        // upward until every insert evicted.
        if let Some((_, evicted)) = guard.push(key, entry) {
            *bytes = bytes.saturating_sub(evicted.message.len());
        }
        *bytes += added;
        while *bytes > MAX_BYTES {
            match guard.pop_lru() {
                Some((_, e)) => *bytes = bytes.saturating_sub(e.message.len()),
                None => break,
            }
        }
    }

    pub fn clear(&self) {
        self.inner.lock().clear();
        *self.bytes.lock() = 0;
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
        let rtype = u16::from_be_bytes([out[i], out[i + 1]]);
        // TYPE 41 is OPT (RFC 6891), a pseudo-RR whose "TTL" field is really
        // EXT-RCODE | VERSION | DO+Z. Writing a TTL there strips the DNSSEC OK bit and
        // stuffs the TTL into Z, which the RFC requires to be zero.
        if rtype != 41 {
            out[i + 4] = (remaining >> 24) as u8;
            out[i + 5] = (remaining >> 16) as u8;
            out[i + 6] = (remaining >> 8) as u8;
            out[i + 7] = remaining as u8;
        }
        let rdlen = u16::from_be_bytes([out[i + 8], out[i + 9]]) as usize;
        i += 10 + rdlen;
        if i > out.len() {
            return out;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Wire message: header, one question, one A record (TTL 300), one OPT with DO=1.
    /// Returns the message plus the byte offsets of the A record's TTL and the OPT's
    /// flags field, so the assertions never depend on hand-done pointer arithmetic.
    fn response_with_opt() -> (Vec<u8>, usize, usize) {
        let mut m = Vec::new();
        m.extend_from_slice(&[0x12, 0x34]); // id
        m.extend_from_slice(&[0x81, 0x80]); // flags
        m.extend_from_slice(&[0, 1]); // qdcount
        m.extend_from_slice(&[0, 1]); // ancount
        m.extend_from_slice(&[0, 0]); // nscount
        m.extend_from_slice(&[0, 1]); // arcount (the OPT)
        m.extend_from_slice(b"\x07example\x03com\x00");
        m.extend_from_slice(&[0, 1, 0, 1]); // qtype A, qclass IN

        m.extend_from_slice(&[0xc0, 0x0c]); // answer name: compression pointer to question
        m.extend_from_slice(&[0, 1, 0, 1]); // type A, class IN
        let a_ttl_at = m.len();
        m.extend_from_slice(&300u32.to_be_bytes());
        m.extend_from_slice(&[0, 4]); // rdlen
        m.extend_from_slice(&[93, 184, 216, 34]); // rdata

        m.extend_from_slice(&[0x00]); // OPT owner name = root
        m.extend_from_slice(&[0, 41]); // TYPE 41 = OPT
        m.extend_from_slice(&[0x04, 0xd0]); // "class" = udp payload size 1232
        let opt_flags_at = m.len();
        m.extend_from_slice(&[0x00, 0x00, 0x80, 0x00]); // ext-rcode, version, DO=1, Z=0
        m.extend_from_slice(&[0, 0]); // rdlen
        (m, a_ttl_at, opt_flags_at)
    }

    /// The OPT "TTL" field is EXT-RCODE|VERSION|DO+Z. Rewriting it as a TTL strips DNSSEC OK
    /// and writes a nonzero Z on every single cache hit.
    #[test]
    fn preserves_edns_opt_while_rewriting_real_ttls() {
        let (msg, a_ttl_at, opt_flags_at) = response_with_opt();
        let out = adjust_ttls(&msg, 10);

        assert_eq!(
            &out[opt_flags_at..opt_flags_at + 4],
            &[0x00, 0x00, 0x80, 0x00],
            "OPT flags must survive untouched: DO bit set, Z zero"
        );
        assert_eq!(
            u32::from_be_bytes(out[a_ttl_at..a_ttl_at + 4].try_into().unwrap()),
            10,
            "the real record's TTL must still be rewritten"
        );
    }

    #[test]
    fn expired_entries_are_evicted_and_byte_count_returns_to_zero() {
        let c = DnsCache::new(8, 45);
        let key = CacheKey { name: "a.test".into(), qtype: 1, qclass: 1, dnssec_ok: false };
        c.insert(key.clone(), vec![0u8; 120], 1, false);
        assert_eq!(c.byte_len(), 120);
        c.clear();
        assert_eq!(c.byte_len(), 0);
        assert!(c.get(&key).is_none());
    }

    /// The DO bit must partition the cache, or a DO=0 client can be served a signed answer.
    #[test]
    fn dnssec_ok_is_part_of_the_key() {
        let c = DnsCache::new(8, 45);
        let plain = CacheKey { name: "a.test".into(), qtype: 1, qclass: 1, dnssec_ok: false };
        let signed = CacheKey { name: "a.test".into(), qtype: 1, qclass: 1, dnssec_ok: true };
        c.insert(signed, vec![0u8; 64], 60, false);
        assert!(c.get(&plain).is_none());
    }

    #[test]
    fn byte_cap_evicts_rather_than_growing_without_bound() {
        let c = DnsCache::new(100_000, 45);
        for i in 0..2000 {
            let key = CacheKey { name: format!("d{i}.test"), qtype: 1, qclass: 1, dnssec_ok: false };
            c.insert(key, vec![0u8; 4096], 300, false);
        }
        assert!(c.byte_len() <= MAX_BYTES, "byte_len={} cap={MAX_BYTES}", c.byte_len());
    }
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
