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

# Tear down SMAppService-managed or previous job if present
launchctl bootout "system/${LABEL}" 2>/dev/null || true
rm -f "$PLIST"

cp "$TMP" "$PLIST"
chmod 644 "$PLIST"
rm -f "$TMP"

launchctl bootstrap system "$PLIST"
launchctl enable "system/${LABEL}"
launchctl kickstart -k "system/${LABEL}"

sleep 2
if launchctl print "system/${LABEL}" 2>/dev/null | grep -q 'pid ='; then
  echo "OK: aegisd running via classic LaunchDaemon"
  launchctl print "system/${LABEL}" 2>/dev/null | egrep 'pid =|state =|last exit|path =' | head -10
  exit 0
fi

# Still failed — surface status
echo "WARN: kickstart finished but pid not found" >&2
launchctl print "system/${LABEL}" 2>/dev/null | egrep 'state =|last exit|job state|path =' | head -15 || true
ps -ax | grep '[a]egisd' || true
exit 1
