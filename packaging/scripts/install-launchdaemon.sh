#!/usr/bin/env bash
# Install classic system LaunchDaemon (absolute path) — survives reboot better than
# a stale SMAppService registration after ad-hoc re-sign (EX_CONFIG / LWCR).
set -euo pipefail

APP="${AEGIS_APP:-/Applications/Aegis.app}"
SRC="$APP/Contents/Resources/aegisd"
LABEL="com.aegis.daemon"
PLIST="/Library/LaunchDaemons/${LABEL}.plist"

if [[ ! -x "$SRC" ]]; then
  echo "aegisd not found at $SRC" >&2
  exit 1
fi

# /Applications is mode 775 group admin, so launchd must not exec the daemon from inside the
# app bundle: any admin-group process could overwrite the binary that runs as uid 0. Copy it
# to a root:wheel directory instead. Not /Library/Application Support/Aegis — that is where
# the daemon writes config and blocklist, and the root-exec'd binary must not sit in a
# root-mutable data directory.
DEST_DIR="/usr/local/libexec/aegis"
install -d -o root -g wheel -m 755 "$DEST_DIR"
install -o root -g wheel -m 755 "$SRC" "$DEST_DIR/aegisd"
BIN="$DEST_DIR/aegisd"

# Deliberately NOT re-signing here: `codesign --force --sign -` as root would launder a
# swapped binary into a loadable one, which is the escalation step, not a mitigation.

TMP="$(mktemp)"
cat > "$TMP" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>${LABEL}</string>
	<key>ProgramArguments</key>
	<array>
		<string>${BIN}</string>
	</array>
	<key>RunAtLoad</key>
	<true/>
	<key>KeepAlive</key>
	<true/>
	<key>ProcessType</key>
	<string>Interactive</string>
	<key>ThrottleInterval</key>
	<integer>30</integer>
	<key>SoftResourceLimits</key>
	<dict>
		<key>NumberOfFiles</key>
		<integer>4096</integer>
	</dict>
	<key>StandardOutPath</key>
	<string>/var/log/aegisd.log</string>
	<key>StandardErrorPath</key>
	<string>/var/log/aegisd.err</string>
</dict>
</plist>
EOF

# Tear down whatever is holding the label. This MUST be verified, not hoped for: when the job
# was registered by SMAppService the launchd record lives in the smd domain, `bootout` fails,
# and a later `kickstart` happily restarts the OLD executable inside the app bundle. That is
# how a machine ends up running a binary from weeks ago while this script prints "OK".
launchctl bootout "system/${LABEL}" 2>/dev/null || true
rm -f "$PLIST"

# Give launchd a moment, then make sure nothing is still holding port 53.
for _ in $(seq 1 20); do
  pgrep -f '[a]egisd' >/dev/null 2>&1 || break
  sleep 0.25
done
if pgrep -f '[a]egisd' >/dev/null 2>&1; then
  echo "==> processo aegisd antigo ainda vivo; encerrando"
  pkill -f '[a]egisd' 2>/dev/null || true
  sleep 1
  pkill -9 -f '[a]egisd' 2>/dev/null || true
  sleep 1
fi

cp "$TMP" "$PLIST"
chmod 644 "$PLIST"
rm -f "$TMP"

launchctl bootstrap system "$PLIST"
launchctl enable "system/${LABEL}"
launchctl kickstart -k "system/${LABEL}"

sleep 2

# Validate that the RUNNING pid is the binary we just installed. Checking only for the presence
# of a pid is what made the previous version of this script report success while an orphaned
# copy from the app bundle served DNS.
PID="$(launchctl print "system/${LABEL}" 2>/dev/null | awk -F'= ' '/^\tpid = /{print $2; exit}')"
if [[ -n "${PID:-}" ]]; then
  RUNNING="$(ps -o comm= -p "$PID" 2>/dev/null || true)"
  if [[ "$RUNNING" == "$BIN" ]]; then
    echo "OK: aegisd (pid $PID) rodando a partir de $BIN"
    exit 0
  fi
  echo "ERRO: o label ${LABEL} está rodando o pid $PID a partir de:" >&2
  echo "        ${RUNNING:-<desconhecido>}" >&2
  echo "      esperado: $BIN" >&2
  echo "      Um registro do SMAppService provavelmente está vencendo o plist clássico." >&2
  echo "      Abra o Aegis, desmarque 'Iniciar no login' em Ajustes -> Avançado e rode isto de novo." >&2
  exit 1
fi

echo "ERRO: nenhum processo subiu para ${LABEL}" >&2
launchctl print "system/${LABEL}" 2>/dev/null | egrep 'state =|last exit|job state|path =' | head -15 || true
exit 1
