#!/bin/bash
# Builds AAE Remote, the Android app, signed with AAE's Android key, and
# copies it to the path given (default: android/dist/AAE-Remote.apk).
#
#   android/build-remote.sh [output.apk]
#
# AAE_VERSION and AAE_BUILD_NUMBER set its version; the build number
# defaults to the commit count, so each build installs over the last.
set -euo pipefail

cd "$(dirname "$0")/.."
ROOT=$(pwd)
OUT="${1:-$ROOT/android/dist/AAE-Remote.apk}"
export AAE_BUILD_NUMBER="${AAE_BUILD_NUMBER:-$(git rev-list --count HEAD)}"
source android/signing.sh
echo "Building AAE Remote, signed with the $AAE_ANDROID_SIGNER key."
(cd android && ./gradlew -q :remote:assembleRelease)
mkdir -p "$(dirname "$OUT")"
cp android/remote/build/outputs/apk/release/remote-release.apk "$OUT"
echo "Built $OUT"
