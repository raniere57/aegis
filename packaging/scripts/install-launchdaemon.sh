#!/usr/bin/env bash
# Install classic system LaunchDaemon (absolute path) — survives reboot better than
# a stale SMAppService registration after ad-hoc re-sign (EX_CONFIG / LWCR).
set -euo pipefail

APP="${AEGIS_APP:-/Applications/Aegis.app}"
BIN="$APP/Contents/Resources/aegisd"
LABEL="com.aegis.daemon"
PLIST="/Library/LaunchDaemons/${LABEL}.plist"

if [[ ! -x "$BIN" ]]; then
  echo "aegisd not found at $BIN" >&2
  exit 1
fi

codesign --force --sign - --identifier com.aegis.daemon "$BIN" 2>/dev/null || true

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
