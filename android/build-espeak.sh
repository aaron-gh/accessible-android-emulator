#!/bin/bash
# Builds AAE's copy of eSpeak NG into android/espeak/build/aae-espeak.apk.
# Needs the Android SDK, an NDK and JDK 17 or later. eSpeak NG is GPL v3; its
# source is the submodule in android/third_party/espeak-ng.
set -euo pipefail

cd "$(dirname "$0")"
ROOT=$(pwd)
SRC="$ROOT/third_party/espeak-ng"
if [[ ! -f "$SRC/android/build.gradle" ]]; then
    git -C "$ROOT/.." submodule update --init --depth 1 android/third_party/espeak-ng
fi
export ANDROID_HOME="${ANDROID_HOME:-$HOME/Library/Android/sdk}"

cd "$SRC/android"
./gradlew -q --init-script "$ROOT/espeak/aae.init.gradle" assembleRelease
mkdir -p "$ROOT/espeak/build"
cp build/outputs/apk/release/*.apk "$ROOT/espeak/build/aae-espeak.apk"
echo "Built $ROOT/espeak/build/aae-espeak.apk"
