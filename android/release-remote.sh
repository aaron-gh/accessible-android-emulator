#!/bin/bash
# Makes a stable release of AAE Remote: builds it as
# android/dist/AAE-Remote-<version>.apk, and points AAE Remote's update feed,
# appcast-android.json at the top of the repository, at it.
#
#   android/release-remote.sh [release-notes.txt]
#
# Then attach the APK to the GitHub release for this version, and commit and
# push appcast-android.json with the other feeds. Phones check it once a day.
set -euo pipefail

cd "$(dirname "$0")/.."
ROOT=$(pwd)
NOTES="${1:-}"
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
export AAE_BUILD_NUMBER="${AAE_BUILD_NUMBER:-$(git rev-list --count HEAD)}"
APK="$ROOT/android/dist/AAE-Remote-$VERSION.apk"
android/build-remote.sh "$APK"
URL="https://github.com/aaron-gh/accessible-android-emulator/releases/download/v$VERSION/AAE-Remote-$VERSION.apk"
python3 -I - "$ROOT/appcast-android.json" "$VERSION" "$AAE_BUILD_NUMBER" "$URL" "$APK" "$NOTES" <<'PY'
import hashlib, json, pathlib, sys
feed, version, build, url, apk, notes = sys.argv[1:]
digest = hashlib.sha256(pathlib.Path(apk).read_bytes()).hexdigest()
text = pathlib.Path(notes).read_text().strip() if notes else ""
pathlib.Path(feed).write_text(json.dumps({
    "version": version, "build": int(build), "url": url, "sha256": digest, "notes": text,
}, indent=2) + "\n")
PY
echo "Made $APK, version $VERSION, build $AAE_BUILD_NUMBER, and updated appcast-android.json."
echo "Attach it to the v$VERSION release, then commit and push appcast-android.json."
