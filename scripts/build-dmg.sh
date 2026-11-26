#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST_DIR="$ROOT_DIR/dist"
APP_DIR="$DIST_DIR/Murmur.app"
DMG_PATH="$DIST_DIR/Murmur-macOS.dmg"

UNIVERSAL=0
APP_ONLY=0
for arg in "$@"; do
    case "$arg" in
        --universal) UNIVERSAL=1 ;;
        --app-only) APP_ONLY=1 ;;
        *)
            echo "Unknown option: $arg" >&2
            echo "Usage: $0 [--universal] [--app-only]" >&2
            exit 1
            ;;
    esac
done

if [[ "$(uname -s)" != "Darwin" ]]; then
    echo "Error: build-dmg.sh must be run on macOS." >&2
    exit 1
fi

cd "$ROOT_DIR"

echo "==> Building Murmur release binary..."
if [[ "$UNIVERSAL" -eq 1 ]]; then
    cargo build --release --target aarch64-apple-darwin
    cargo build --release --target x86_64-apple-darwin
    BIN_PATH="$DIST_DIR/murmur-universal"
    mkdir -p "$DIST_DIR"
    lipo -create \
        "$ROOT_DIR/target/aarch64-apple-darwin/release/murmur" \
        "$ROOT_DIR/target/x86_64-apple-darwin/release/murmur" \
        -output "$BIN_PATH"
else
    cargo build --release
    BIN_PATH="$ROOT_DIR/target/release/murmur"
fi

echo "==> Assembling Murmur.app bundle..."
rm -rf "$APP_DIR"
mkdir -p "$APP_DIR/Contents/MacOS" "$APP_DIR/Contents/Resources"
cp "$BIN_PATH" "$APP_DIR/Contents/MacOS/murmur"
chmod +x "$APP_DIR/Contents/MacOS/murmur"
cp "$ROOT_DIR/packaging/Info.plist" "$APP_DIR/Contents/Info.plist"

if [[ "$UNIVERSAL" -eq 1 ]]; then
    rm -f "$BIN_PATH"
fi

echo "==> Generating AppIcon.icns from assets/murmur.svg..."
ICONSET_DIR="$DIST_DIR/AppIcon.iconset"
BASE_PNG="$DIST_DIR/murmur_1024.png"
rm -rf "$ICONSET_DIR" "$BASE_PNG"
mkdir -p "$ICONSET_DIR"

if command -v rsvg-convert >/dev/null 2>&1; then
    rsvg-convert -w 1024 -h 1024 "$ROOT_DIR/assets/murmur.svg" -o "$BASE_PNG"
elif command -v qlmanage >/dev/null 2>&1; then
    qlmanage -t -s 1024 -o "$DIST_DIR" "$ROOT_DIR/assets/murmur.svg" >/dev/null 2>&1 || true
    if [[ -f "$DIST_DIR/murmur.svg.png" ]]; then
        mv "$DIST_DIR/murmur.svg.png" "$BASE_PNG"
    fi
fi

if [[ -f "$BASE_PNG" ]] && command -v sips >/dev/null 2>&1 && command -v iconutil >/dev/null 2>&1; then
    for sz in 16 32 128 256 512; do
        sips -z "$sz" "$sz" "$BASE_PNG" --out "$ICONSET_DIR/icon_${sz}x${sz}.png" >/dev/null
        sz2=$((sz * 2))
        sips -z "$sz2" "$sz2" "$BASE_PNG" --out "$ICONSET_DIR/icon_${sz}x${sz}@2x.png" >/dev/null
    done
    iconutil -c icns "$ICONSET_DIR" -o "$APP_DIR/Contents/Resources/AppIcon.icns" || true
else
    echo "    (Skipping .icns generation; rsvg-convert/sips/iconutil not all available)"
fi
rm -rf "$ICONSET_DIR" "$BASE_PNG"

echo "==> Ad-hoc signing Murmur.app..."
if command -v codesign >/dev/null 2>&1; then
    codesign --force --deep --sign - "$APP_DIR"
fi

if [[ "$APP_ONLY" -eq 1 ]]; then
    echo "Done: $APP_DIR"
    exit 0
fi

echo "==> Creating drag-to-Applications DMG..."
STAGING_DIR="$DIST_DIR/dmg-staging"
rm -rf "$STAGING_DIR" "$DMG_PATH"
mkdir -p "$STAGING_DIR"
cp -R "$APP_DIR" "$STAGING_DIR/Murmur.app"
ln -s /Applications "$STAGING_DIR/Applications"

hdiutil create \
    -volname "Murmur" \
    -srcfolder "$STAGING_DIR" \
    -ov \
    -format UDZO \
    "$DMG_PATH"

rm -rf "$STAGING_DIR"
echo "Done: $DMG_PATH"
