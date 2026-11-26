#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TARGET_DIR="/Applications"

if [[ "${1:-}" == "--user" ]]; then
    TARGET_DIR="$HOME/Applications"
    mkdir -p "$TARGET_DIR"
fi

chmod +x "$ROOT_DIR/scripts/build-dmg.sh"
"$ROOT_DIR/scripts/build-dmg.sh" --app-only

echo "==> Installing Murmur.app to $TARGET_DIR..."
rm -rf "$TARGET_DIR/Murmur.app"
cp -R "$ROOT_DIR/dist/Murmur.app" "$TARGET_DIR/Murmur.app"

echo "Installed: $TARGET_DIR/Murmur.app"
