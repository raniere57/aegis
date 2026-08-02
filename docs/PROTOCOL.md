# Aegis — Control Protocol

Unix domain socket, UTF-8, **one JSON object per line**.

Default socket:

- Privileged: `/var/run/aegis.sock`
- Dev: `~/.aegis/aegis.sock`

## Request

```json
{"id":"1","method":"ping","params":{}}
```

## Response

```json
{"id":"1","ok":true,"result":{"pong":true}}
```

Error:

```json
{"id":"1","ok":false,"error":{"code":"busy","message":"update already running"}}
```

## Methods

| Method | Params | Result |
|--------|--------|--------|
| `ping` | `{}` | `{ "pong": true, "version": "…" }` |
| `status` | `{}` | enabled, uptime_secs, listen, domain_count, list_updated_at, last_update_error, filtering |
| `metrics` | `{}` | queries, blocked, cache_hit, cache_miss, upstream_errors, upstream_ok |
| `set_enabled` | `{ "enabled": bool }` | `{ "enabled": bool }` |
| `reload_config` | `{}` | `{ "reloaded": true }` |
| `reload_lists` | `{}` | `{ "domains": number }` |
| `update_lists` | `{}` | `{ "started": true }` or `{ "started": false, "reason": "…" }` |
| `get_config` | `{}` | config object (safe subset) |
| `patch_config` | partial config | `{ "saved": true }` |
| `allowlist.add` | `{ "domain": "…" }` | `{ "domains": […] }` |
| `allowlist.remove` | `{ "domain": "…" }` | `{ "domains": […] }` |
| `allowlist.list` | `{}` | `{ "domains": […] }` |
| `lists.add_url` | `{ "url": "…" }` | `{ "urls": […] }` |
| `lists.remove_url` | `{ "url": "…" }` | `{ "urls": […] }` |
| `lists.list` | `{}` | `{ "urls": […], "auto_update": bool, "interval_hours": number }` |

v0.1 is request/response only (no push events). The MenuBarExtra polls `status` / `metrics` when opened.
