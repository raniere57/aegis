#!/usr/bin/env bash
# Build Aegis.app and package a drag-and-drop .dmg
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
DIST="$ROOT/dist"
APP="$DIST/Aegis.app"
STAGE="$DIST/dmg-stage"
DMG="$DIST/Aegis-0.1.0.dmg"
VOL="Aegis"

echo "==> Building release binaries"
cd "$ROOT/daemon"
cargo build -p aegisd -p aegis-ctl --release

cd "$ROOT/app"
swift build -c release
BIN="$(swift build -c release --show-bin-path)/Aegis"

echo "==> Assembling Aegis.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" \
         "$APP/Contents/Resources" \
         "$APP/Contents/Library/LaunchDaemons" \
         "$APP/Contents/Library/LaunchAgents"

cp "$BIN" "$APP/Contents/MacOS/Aegis"
cp "$ROOT/daemon/target/release/aegisd" "$APP/Contents/Resources/aegisd"
cp "$ROOT/daemon/target/release/aegis-ctl" "$APP/Contents/Resources/aegis-ctl"
cp "$ROOT/packaging/scripts/aegis-failopen.sh" "$APP/Contents/Resources/aegis-failopen.sh"
cp "$ROOT/packaging/scripts/install-launchdaemon.sh" "$APP/Contents/Resources/install-launchdaemon.sh"
cp "$ROOT/app/Aegis/Resources/Info.plist" "$APP/Contents/Info.plist"
cp "$ROOT/app/Aegis/Resources/AppIcon.icns" "$APP/Contents/Resources/AppIcon.icns"
cp "$ROOT/packaging/launchd/com.aegis.daemon.plist" "$APP/Contents/Library/LaunchDaemons/"
cp "$ROOT/packaging/launchd/com.aegis.failopen.plist" "$APP/Contents/Library/LaunchAgents/"
chmod +x "$APP/Contents/MacOS/Aegis" \
         "$APP/Contents/Resources/aegisd" \
         "$APP/Contents/Resources/aegis-ctl" \
         "$APP/Contents/Resources/aegis-failopen.sh" \
         "$APP/Contents/Resources/install-launchdaemon.sh"

# Sign nested binaries first, then the bundle (adhoc). Avoid linker-signed-only aegisd → launchd EX_CONFIG.
codesign --force --sign - --identifier com.aegis.daemon "$APP/Contents/Resources/aegisd"
codesign --force --sign - --identifier com.aegis.ctl "$APP/Contents/Resources/aegis-ctl"
codesign --force --deep --sign - "$APP"

echo "==> Creating DMG"
rm -rf "$STAGE" "$DMG" "$DIST/Aegis-tmp.dmg"
mkdir -p "$STAGE"
cp -R "$APP" "$STAGE/Aegis.app"
ln -s /Applications "$STAGE/Applications"

# Optional README inside DMG
cat > "$STAGE/LEIA-ME.txt" <<'EOF'
Aegis — filtro DNS para macOS

1. Arraste Aegis.app para Applications
2. Abra o app (pode precisar: clique direito → Abrir, na primeira vez)
3. Ative o filtro no menu da barra

Modo desenvolvimento do daemon (sem root):
  Aegis.app/Contents/Resources/aegisd --dev

Restaurar DNS se algo der errado:
  https://github.com/ — use packaging/scripts/restore-dns.sh no repo

Lista padrão: HaGeZi Multi Normal (hosts)
EOF

hdiutil create \
  -volname "$VOL" \
  -srcfolder "$STAGE" \
  -ov \
  -format UDZO \
  "$DMG"

rm -rf "$STAGE"
echo "==> Pronto: $DMG"
ls -lh "$DMG"
