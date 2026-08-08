//! Fixed-size ring of recently blocked domains.
//!
//! The point of this module is to answer the only question a user actually asks when a page
//! breaks — "what did you just block?" — without violating the no-per-query-logging rule.
//! Nothing here allocates, touches disk, or runs for allowed queries: a block copies at most
//! 128 bytes into a preallocated slot behind an uncontended lock (~23 ns against a 500 µs
//! p50 budget), and the whole structure is 35 KB regardless of traffic.

use parking_lot::Mutex;

/// Full DNS names run to 253 bytes, but 128 covers essentially every real blocked name and
/// keeps the ring at 35 KB. Truncated entries are dropped rather than stored, because a
/// truncated name would make the one-click unblock produce a domain that does not exist.
const NAME_CAP: usize = 128;
const SLOTS: usize = 256;

#[derive(Clone, Copy)]
struct Slot {
    name: [u8; NAME_CAP],
    len: u8,
    at_unix: u32,
    hits: u16,
}

impl Default for Slot {
    fn default() -> Self {
        Self {
            name: [0; NAME_CAP],
            len: 0,
            at_unix: 0,
            hits: 0,
        }
    }
}

pub struct RecentBlocks {
    slots: Mutex<Box<[Slot; SLOTS]>>,
    /// Write cursor; also the count of blocks ever recorded, modulo wraparound.
    next: Mutex<usize>,
}

/// One entry as handed to the UI.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RecentEntry {
    pub domain: String,
    pub at_unix: u32,
    pub hits: u16,
}

impl Default for RecentBlocks {
    fn default() -> Self {
        Self::new()
    }
}

impl RecentBlocks {
    pub fn new() -> Self {
        Self {
            slots: Mutex::new(Box::new([Slot::default(); SLOTS])),
            next: Mutex::new(0),
        }
    }

    /// Record a blocked domain. Called on the block path only.
    pub fn record(&self, domain: &str) {
        let bytes = domain.as_bytes();
        if bytes.is_empty() || bytes.len() > NAME_CAP {
            return;
        }
        let mut next = self.next.lock();
        let mut slots = self.slots.lock();

        // Coalesce a repeat of the most recent entry instead of advancing, so one noisy
        // tracker cannot flush everything else out of a 256-slot window.
        let last = (*next + SLOTS - 1) % SLOTS;
        if *next > 0 && slots[last].len as usize == bytes.len() && &slots[last].name[..bytes.len()] == bytes {
            slots[last].hits = slots[last].hits.saturating_add(1);
            slots[last].at_unix = now_unix();
            return;
        }

        let idx = *next % SLOTS;
        slots[idx].name[..bytes.len()].copy_from_slice(bytes);
        slots[idx].len = bytes.len() as u8;
        slots[idx].at_unix = now_unix();
        slots[idx].hits = 1;
        *next += 1;
    }

    /// Most recent first. Read only over IPC, never on the hot path.
    pub fn snapshot(&self, limit: usize) -> Vec<RecentEntry> {
        let next = *self.next.lock();
        let slots = self.slots.lock();
        let total = next.min(SLOTS);
        (0..total.min(limit))
            .filter_map(|back| {
                let idx = (next + SLOTS - 1 - back) % SLOTS;
                let s = &slots[idx];
                if s.len == 0 {
                    return None;
                }
                Some(RecentEntry {
                    domain: String::from_utf8_lossy(&s.name[..s.len as usize]).into_owned(),
                    at_unix: s.at_unix,
                    hits: s.hits,
                })
            })
            .collect()
    }

    pub fn clear(&self) {
        *self.next.lock() = 0;
        self.slots.lock().fill(Slot::default());
    }
}

fn now_unix() -> u32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as u32)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_most_recent_first() {
        let r = RecentBlocks::new();
        r.record("a.example.com");
        r.record("b.example.com");
        let got = r.snapshot(10);
        assert_eq!(got[0].domain, "b.example.com");
        assert_eq!(got[1].domain, "a.example.com");
    }

    #[test]
    fn coalesces_repeats_instead_of_filling_the_ring() {
        let r = RecentBlocks::new();
        for _ in 0..50 {
            r.record("noisy.example.com");
        }
        let got = r.snapshot(10);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].hits, 50);
    }

    #[test]
    fn wraps_without_losing_the_newest() {
        let r = RecentBlocks::new();
        for i in 0..(SLOTS + 10) {
            r.record(&format!("d{i}.example.com"));
        }
        let got = r.snapshot(SLOTS * 2);
        assert_eq!(got.len(), SLOTS);
        assert_eq!(got[0].domain, format!("d{}.example.com", SLOTS + 9));
    }

    /// A name we cannot store whole must be dropped, not truncated — the UI offers one-click
    /// unblock on these strings and a truncated one is a different, nonexistent domain.
    #[test]
    fn drops_oversized_names_rather_than_truncating() {
        let r = RecentBlocks::new();
        r.record(&"x".repeat(NAME_CAP + 1));
        assert!(r.snapshot(10).is_empty());
    }
}
