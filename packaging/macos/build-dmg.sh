#!/usr/bin/env bash
# Build a .dmg holding the app bundle, the command-line tool, a README and an Applications shortcut.
#
#   build-dmg.sh VOLUME_NAME STAGE_DIR OUT.dmg
#
# STAGE_DIR is the assembled package directory (it contains the .app, the CLI and the licences). Runs on macOS.
set -euo pipefail

VOLUME="$1"
STAGE="$2"
OUT="$3"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
ROOT="$WORK/$VOLUME"
mkdir -p "$ROOT"
# ditto keeps the bundle's signature and extended attributes intact.
for item in "$STAGE"/*; do
  ditto "$item" "$ROOT/$(basename "$item")"
done
ln -s /Applications "$ROOT/Applications"
mkdir -p "$(dirname "$OUT")"
hdiutil create -volname "$VOLUME" -srcfolder "$ROOT" -ov -format UDZO "$OUT"
