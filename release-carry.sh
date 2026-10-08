#!/bin/bash
# Attaches the Windows installer and AAE Remote that the update feeds
# currently offer to another GitHub release, for a release made for the Mac
# only, so each release has every platform's download.
#
#   ./release-carry.sh v0.6.0
#
# Downloads both from the addresses in appcast-windows.xml and
# appcast-android.json, checks the installer's update signature and length
# and AAE Remote's SHA-256 against the feeds, and uploads them unchanged,
# under their own names. The feeds aren't changed.
#
# Settings:
#   AAE_ED_KEY_FILE   the private update key, base64, if it isn't in the
#                     login Keychain as the "aae" account
set -euo pipefail

cd "$(dirname "$0")"
TAG="${1:?Usage: ./release-carry.sh <tag>, for example v0.6.0}"
gh release view "$TAG" >/dev/null

# Prints the installer's address, signature and length, then AAE Remote's
# address and SHA-256, one per line.
FEEDS=$(python3 -I - appcast-windows.xml appcast-android.json <<'PY'
import json, re, sys
windows = open(sys.argv[1]).read()
item = re.search(r'<enclosure url="([^"]+)" sparkle:edSignature="([^"]+)" length="(\d+)"', windows)
if not item:
    sys.exit(f"{sys.argv[1]} has no release.")
android = json.load(open(sys.argv[2]))
print("\n".join([*item.groups(), android["url"], android["sha256"]]))
PY
)
{ read -r SETUP_URL; read -r SETUP_SIGNATURE; read -r SETUP_LENGTH; read -r APK_URL; read -r APK_SHA256; } <<<"$FEEDS"

DIR=$(mktemp -d)
trap 'rm -rf "$DIR"' EXIT
SETUP="$DIR/$(basename "$SETUP_URL")"
APK="$DIR/$(basename "$APK_URL")"
echo "Downloading $(basename "$SETUP") and $(basename "$APK")."
curl -fsSL -o "$SETUP" "$SETUP_URL"
curl -fsSL -o "$APK" "$APK_URL"

if [[ "$(shasum -a 256 "$APK" | cut -d' ' -f1)" != "$APK_SHA256" ]]; then
    echo "$(basename "$APK") doesn't match the SHA-256 in appcast-android.json." >&2
    exit 1
fi

SIGN_UPDATE=$(find macos/.build/artifacts -path '*/Sparkle/bin/sign_update' 2>/dev/null | head -1)
if [[ -z "$SIGN_UPDATE" ]]; then
    echo "Sparkle's sign_update isn't here yet. Build the Mac app once with macos/build.sh, which fetches it." >&2
    exit 1
fi
KEY_ARGS=(--account aae)
if [[ -n "${AAE_ED_KEY_FILE:-}" ]]; then
    KEY_ARGS=(--ed-key-file "$AAE_ED_KEY_FILE")
fi
# Ed25519 signatures are deterministic: the same file and key give the feed's signature.
SIGNATURE=$("$SIGN_UPDATE" ${KEY_ARGS[@]+"${KEY_ARGS[@]}"} "$SETUP")
if [[ "$SIGNATURE" != "sparkle:edSignature=\"$SETUP_SIGNATURE\" length=\"$SETUP_LENGTH\"" ]]; then
    echo "$(basename "$SETUP") doesn't match the signature and length in appcast-windows.xml." >&2
    exit 1
fi

gh release upload "$TAG" "$SETUP" "$APK"
echo "Attached $(basename "$SETUP") and $(basename "$APK") to $TAG."
