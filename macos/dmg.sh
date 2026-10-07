#!/bin/bash
# Packs AAE.app in a disk image with an Applications shortcut.
#   macos/dmg.sh <AAE.app> <disk image> <volume name>
set -euo pipefail
APP="$1"
DMG="$2"
VOLUME="$3"
STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
mkdir -p "$(dirname "$DMG")"
hdiutil create -quiet -volname "$VOLUME" -srcfolder "$STAGE" -format UDZO -ov "$DMG"
