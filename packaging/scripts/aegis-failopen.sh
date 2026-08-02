#!/bin/bash
# Fail-open watchdog: if DNS points at Aegis but aegisd is down, restore DNS.
# Runs as LaunchAgent (user session) — must never leave the Mac without working DNS.
set -u

SOCK="/var/run/aegis.sock"
HOME_SOCK="${HOME}/.aegis/aegis.sock"
BACKUP="${HOME}/Library/Application Support/Aegis/dns-backup.json"

daemon_up() {
  for s in "$SOCK" "$HOME_SOCK"; do
    [[ -S "$s" ]] || continue
    # Probe with a tiny RPC if nc/python available; else file existence is weak — try bash /dev/tcp no for unix.
    if command -v nc >/dev/null 2>&1; then
      if echo '{"id":"w","method":"ping","params":{}}' | nc -U -w 1 "$s" 2>/dev/null | grep -q '"ok"'; then
        return 0
      fi
    elif command -v python3 >/dev/null 2>&1; then
      python3 - "$s" <<'PY' 2>/dev/null && return 0
import socket, sys
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.settimeout(1.0)
try:
    s.connect(sys.argv[1])
    s.sendall(b'{"id":"w","method":"ping","params":{}}\n')
    data = s.recv(256)
    sys.exit(0 if b'"ok"' in data else 1)
except Exception:
    sys.exit(1)
PY
    fi
  done
  return 1
}

dns_points_local() {
  while IFS= read -r svc; do
    [[ "$svc" == *"asterisk"* ]] && continue
    [[ "$svc" == \** ]] && continue
    [[ -z "$svc" ]] && continue
    out="$(/usr/sbin/networksetup -getdnsservers "$svc" 2>/dev/null || true)"
    if echo "$out" | grep -Eq '^(127\.0\.0\.1|::1)$'; then
      return 0
    fi
  done < <(/usr/sbin/networksetup -listallnetworkservices 2>/dev/null)
  return 1
}

restore_dns() {
  if [[ -f "$BACKUP" ]] && command -v python3 >/dev/null 2>&1; then
    python3 - <<PY
import json, subprocess
data = json.load(open(r"""$BACKUP"""))
for svc in data.get("services", []):
    name = svc["name"]
    servers = svc.get("servers") or []
    if not servers:
        subprocess.run(["/usr/sbin/networksetup", "-setdnsservers", name, "Empty"], check=False)
    else:
        subprocess.run(["/usr/sbin/networksetup", "-setdnsservers", name, *servers], check=False)
PY
  else
    while IFS= read -r svc; do
      [[ "$svc" == *"asterisk"* ]] && continue
      [[ "$svc" == \** ]] && continue
      [[ -z "$svc" ]] && continue
      /usr/sbin/networksetup -setdnsservers "$svc" Empty 2>/dev/null || true
    done < <(/usr/sbin/networksetup -listallnetworkservices 2>/dev/null)
  fi
  /usr/bin/dscacheutil -flushcache 2>/dev/null || true
  /usr/bin/killall -HUP mDNSResponder 2>/dev/null || true
  echo "$(date -u +%Y-%m-%dT%H:%M:%SZ) fail-open: restored DNS (aegisd unreachable)" >> "${HOME}/Library/Logs/aegis-failopen.log"
}

if daemon_up; then
  exit 0
fi

if dns_points_local; then
  restore_dns
fi

exit 0
