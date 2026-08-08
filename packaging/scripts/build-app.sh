#!/usr/bin/env bash
# Build aegisd (release) and the Swift MenuBar app, assemble Aegis.app skeleton.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
DIST="$ROOT/dist"
APP="$DIST/Aegis.app"

echo "==> Building aegisd (release)"
cd "$ROOT/daemon"
cargo build -p aegisd -p aegis-ctl --release

echo "==> Building Swift Aegis"
cd "$ROOT/app"
swift build -c release

BIN="$(swift build -c release --show-bin-path)/Aegis"
mkdir -p "$APP/Contents/MacOS" \
         "$APP/Contents/Resources" \
         "$APP/Contents/Library/LaunchDaemons" \
         "$APP/Contents/Library/LaunchAgents"

cp "$BIN" "$APP/Contents/MacOS/Aegis"
cp "$ROOT/daemon/target/release/aegisd" "$APP/Contents/Resources/aegisd"
cp "$ROOT/daemon/target/release/aegis-ctl" "$APP/Contents/Resources/aegis-ctl"
cp "$ROOT/packaging/scripts/aegis-failopen.sh" "$APP/Contents/Resources/aegis-failopen.sh"
cp "$ROOT/packaging/scripts/install-launchdaemon.sh" "$APP/Contents/Resources/install-launchdaemon.sh"
cp "$ROOT/packaging/scripts/uninstall.sh" "$APP/Contents/Resources/uninstall.sh"
cp "$ROOT/app/Aegis/Resources/Info.plist" "$APP/Contents/Info.plist"
cp "$ROOT/packaging/launchd/com.aegis.daemon.plist" "$APP/Contents/Library/LaunchDaemons/"
cp "$ROOT/packaging/launchd/com.aegis.failopen.plist" "$APP/Contents/Library/LaunchAgents/"

chmod +x "$APP/Contents/MacOS/Aegis" \
         "$APP/Contents/Resources/aegisd" \
         "$APP/Contents/Resources/aegis-ctl" \
         "$APP/Contents/Resources/aegis-failopen.sh" \
         "$APP/Contents/Resources/install-launchdaemon.sh" \
         "$APP/Contents/Resources/uninstall.sh"

codesign --force --sign - --identifier com.aegis.daemon "$APP/Contents/Resources/aegisd"
codesign --force --sign - --identifier com.aegis.ctl "$APP/Contents/Resources/aegis-ctl"
codesign --force --deep --sign - "$APP"

echo "==> Built $APP"
echo "Dev tip: run daemon without privileges:"
echo "  $ROOT/daemon/target/release/aegisd --dev"
echo "  $ROOT/daemon/target/release/aegis-ctl --dev ping"
