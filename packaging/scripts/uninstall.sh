#!/usr/bin/env bash
# Full uninstall. Restores DNS FIRST — an uninstall that leaves the Mac pointing at a
# 127.0.0.1 that nothing is listening on is worse than no uninstall at all.
#
#   sudo bash uninstall.sh            # remove daemon + app, keep lists/config
#   sudo bash uninstall.sh --purge    # also delete config, blocklist and logs
set -uo pipefail

PURGE=0
[[ "${1:-}" == "--purge" ]] && PURGE=1

LABEL="com.aegis.daemon"
AGENT="com.aegis.failopen"
APP="/Applications/Aegis.app"
# Restore runs as the console user: the DNS backup lives in their home, not root's.
CONSOLE_USER="$(/usr/bin/stat -f %Su /dev/console)"
CONSOLE_HOME="$(/usr/bin/dscl . -read "/Users/${CONSOLE_USER}" NFSHomeDirectory 2>/dev/null | awk '{print $2}')"

echo "==> restaurando DNS do sistema"
if [[ -x "$APP/Contents/Resources/aegis-failopen.sh" ]]; then
  # Reuse the shipped watchdog: it knows how to read the JSON backup.
  sudo -u "$CONSOLE_USER" HOME="$CONSOLE_HOME" "$APP/Contents/Resources/aegis-failopen.sh" || true
fi
# Belt and braces: anything still on loopback goes back to DHCP.
while IFS= read -r svc; do
  [[ "$svc" == \** || -z "$svc" || "$svc" == *"asterisk"* ]] && continue
  out="$(/usr/sbin/networksetup -getdnsservers "$svc" 2>/dev/null || true)"
  if echo "$out" | grep -Eq '^(127\.0\.0\.1|::1)$'; then
    /usr/sbin/networksetup -setdnsservers "$svc" Empty || true
    echo "    $svc -> DHCP"
  fi
done < <(/usr/sbin/networksetup -listallnetworkservices 2>/dev/null)
/usr/bin/dscacheutil -flushcache 2>/dev/null || true
/usr/bin/killall -HUP mDNSResponder 2>/dev/null || true

echo "==> parando serviços"
launchctl bootout "system/${LABEL}" 2>/dev/null || true
sudo -u "$CONSOLE_USER" launchctl bootout "gui/$(id -u "$CONSOLE_USER")/${AGENT}" 2>/dev/null || true
rm -f "/Library/LaunchDaemons/${LABEL}.plist"

echo "==> removendo binários e app"
rm -rf /usr/local/libexec/aegis
rm -rf "$APP"
rm -f /var/run/aegis.sock /var/log/aegisd.log /var/log/aegisd.err

if [[ $PURGE -eq 1 ]]; then
  echo "==> purgando dados (config, blocklist, logs)"
  rm -rf "/Library/Application Support/Aegis"
  rm -rf "${CONSOLE_HOME}/Library/Application Support/Aegis"
  rm -rf "${CONSOLE_HOME}/.aegis"
  rm -f "${CONSOLE_HOME}/Library/Logs/aegis-failopen.log"
else
  echo "    dados mantidos em /Library/Application Support/Aegis (use --purge para apagar)"
fi

echo "OK: Aegis removido. Confira o DNS em Ajustes do Sistema → Rede."
