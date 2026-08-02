# Aegis — Architecture

## Overview

Aegis is a native macOS DNS filter: SwiftUI `MenuBarExtra` UI + Rust DNS daemon.

```
MenuBarExtra  --Unix socket-->  aegisd (LaunchDaemon)
                                    |
                                    +-- allowlist
                                    +-- radix blocklist (mmap)
                                    +-- LRU cache
                                    +-- upstream DNS53
                                    +-- list updater (background)
```

## Processes

| Process | Role | Lifetime |
|---------|------|----------|
| `Aegis.app` | UI, SMAppService, System DNS toggle | User session |
| `aegisd` | DNS proxy, lists, metrics, IPC | Always (launchd) |

## Hot path

1. Parse DNS question
2. Normalize QNAME
3. Bypass `localhost` / `*.local`
4. Allowlist → pass
5. Blocklist → NXDOMAIN
6. Cache → hit
7. Upstream → store + return

**Forbidden on hot path:** SQLite, HTTP, TOML parse, per-query logging.

## Fail-open

If the blocklist is missing, corrupt, or empty, queries are forwarded. Losing blocking temporarily is better than losing internet.

## Data paths

| Path | Content |
|------|---------|
| `/Library/Application Support/Aegis/config.toml` | Daemon config |
| `/Library/Application Support/Aegis/blocklist.bin` | Compiled trie |
| `/Library/Application Support/Aegis/meta.sqlite` | List ETags / timestamps |
| `/Library/Application Support/Aegis/dns-backup.json` | Pre-activation DNS |
| `/var/run/aegis.sock` | Control socket |

Dev / unprivileged mode uses `~/.aegis/` and `127.0.0.1:53553` (not 5353 — that port is mDNS).

## Crates

- `aegis-core` — config, metrics, cache, trie, DNS proxy, IPC types
- `aegis-lists` — fetch, normalize, compile, SQLite meta
- `aegisd` — daemon binary
- `aegis-ctl` — CLI for debug / scripting
