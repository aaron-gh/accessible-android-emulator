#!/bin/bash
# Builds AAE's copy of eSpeak NG into android/espeak/build/aae-espeak.apk.
# Needs the Android SDK, an NDK and JDK 17 or later. eSpeak NG is GPL v3; its
# source is the submodule in android/third_party/espeak-ng.
#   android/build-espeak.sh --if-needed   only builds when there's no build
#                                         signed with the key in use
set -euo pipefail

cd "$(dirname "$0")"
ROOT=$(pwd)
source "$ROOT/signing.sh"
STAMP="$ROOT/espeak/build/signed-with"
if [[ "${1:-}" == "--if-needed" && -f "$ROOT/espeak/build/aae-espeak.apk" \
      && "$(cat "$STAMP" 2>/dev/null)" == "$AAE_ANDROID_SIGNER" ]]; then
    exit 0
fi
echo "Building AAE's eSpeak NG. The first build takes a few minutes."
SRC="$ROOT/third_party/espeak-ng"
if [[ ! -f "$SRC/android/build.gradle" ]]; then
    git -C "$ROOT/.." submodule update --init --depth 1 android/third_party/espeak-ng
fi
export ANDROID_HOME="${ANDROID_HOME:-$HOME/Library/Android/sdk}"

cd "$SRC/android"
./gradlew -q --init-script "$ROOT/espeak/aae.init.gradle" assembleRelease
mkdir -p "$ROOT/espeak/build"
cp build/outputs/apk/release/*.apk "$ROOT/espeak/build/aae-espeak.apk"
echo "$AAE_ANDROID_SIGNER" > "$STAMP"
echo "Built $ROOT/espeak/build/aae-espeak.apk"
