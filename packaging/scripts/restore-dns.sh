#!/usr/bin/env bash
# Restore system DNS from Aegis backup (safety net).
set -euo pipefail
BACKUP="${HOME}/Library/Application Support/Aegis/dns-backup.json"
if [[ ! -f "$BACKUP" ]]; then
  echo "No backup at $BACKUP — setting DNS to Empty (DHCP) on all services"
  while IFS= read -r svc; do
    [[ "$svc" == *"asterisk"* ]] && continue
    [[ "$svc" == \** ]] && continue
    [[ -z "$svc" ]] && continue
    networksetup -setdnsservers "$svc" Empty || true
  done < <(networksetup -listallnetworkservices)
  exit 0
fi

python3 - <<'PY' "$BACKUP"
import json, sys, subprocess
path = sys.argv[1]
data = json.load(open(path))
for svc in data.get("services", []):
    name = svc["name"]
    servers = svc.get("servers") or []
    if not servers:
        subprocess.run(["networksetup", "-setdnsservers", name, "Empty"], check=False)
    else:
        subprocess.run(["networksetup", "-setdnsservers", name, *servers], check=False)
print("DNS restored from", path)
PY
dscacheutil -flushcache 2>/dev/null || true
killall -HUP mDNSResponder 2>/dev/null || true
