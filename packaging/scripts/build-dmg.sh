#!/usr/bin/env bash
# Build Aegis.app and package a drag-and-drop .dmg
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
DIST="$ROOT/dist"
APP="$DIST/Aegis.app"
STAGE="$DIST/dmg-stage"
# Read the version from Cargo.toml so the DMG name can never drift from the binary's
# CARGO_PKG_VERSION, which is what `aegis-ctl --version` and the UI report.
VERSION="$(awk -F'"' '/^version = /{print $2; exit}' "$ROOT/daemon/Cargo.toml")"
DMG="$DIST/Aegis-${VERSION}.dmg"
VOL="Aegis"

# Info.plist declares LSMinimumSystemVersion 14.0, which 2018-and-later Intel Macs satisfy —
# so an arm64-only build installs happily and then fails to launch. Ship universal.
TARGETS=(aarch64-apple-darwin x86_64-apple-darwin)

echo "==> Building release binaries (universal)"
cd "$ROOT/daemon"
for t in "${TARGETS[@]}"; do
  rustup target add "$t" >/dev/null 2>&1 || true
  cargo build -p aegisd -p aegis-ctl --release --target "$t"
done
LIPO_DIR="$ROOT/daemon/target/universal"
mkdir -p "$LIPO_DIR"
for b in aegisd aegis-ctl; do
  lipo -create -output "$LIPO_DIR/$b" \
    "$ROOT/daemon/target/aarch64-apple-darwin/release/$b" \
    "$ROOT/daemon/target/x86_64-apple-darwin/release/$b"
done

cd "$ROOT/app"
swift build -c release --arch arm64 --arch x86_64
BIN="$(swift build -c release --arch arm64 --arch x86_64 --show-bin-path)/Aegis"

echo "==> Assembling Aegis.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" \
         "$APP/Contents/Resources" \
         "$APP/Contents/Library/LaunchDaemons" \
         "$APP/Contents/Library/LaunchAgents"

cp "$BIN" "$APP/Contents/MacOS/Aegis"
cp "$LIPO_DIR/aegisd" "$APP/Contents/Resources/aegisd"
cp "$LIPO_DIR/aegis-ctl" "$APP/Contents/Resources/aegis-ctl"
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
# No --deep: it re-signs nested code and overwrites the identifiers set just above with a
# filename-derived default, which is a likely contributor to the EX_CONFIG/LWCR churn. The
# inside-out order below is what --deep was there to approximate anyway.
codesign --force --sign - --identifier com.aegis.daemon "$APP/Contents/Resources/aegisd"
codesign --force --sign - --identifier com.aegis.ctl "$APP/Contents/Resources/aegis-ctl"
codesign --force --sign - "$APP"

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
