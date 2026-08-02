# Aegis — Performance Budget

Targets for v1 on Apple Silicon.

| Metric | Target |
|--------|--------|
| RSS idle (~100k domains) | ≤ 15 MB |
| RSS idle (~1M domains) | ≤ 25 MB |
| RSS idle (~3.4M domains, FST+mmap) | ≤ 15 MB typical |
| CPU idle | ~0% |
| p50 block / cache hit | < 0.5 ms |
| p99 block | < 2 ms |
| Update 304 | < 1 s wall |
| Full compile 1M domains | background; DNS p99 ≤ 2× baseline |

## Measurement

```bash
# Dev daemon on 53553
cargo run -p aegisd -- --dev

# Hot path
dig @127.0.0.1 -p 53553 example.com +time=1
aegis-ctl --dev metrics

# RSS
ps -o rss= -p $(pgrep aegisd)
```

Bench crate: `benches/hotpath` (criterion) for trie lookup and cache.

## Rules

- No SQLite / HTTP / disk on the DNS hot path
- Cap cache at 8192 entries by default
- Cap inflight upstream at 256
- Atomic blocklist swap only (mmap + ArcSwap)
