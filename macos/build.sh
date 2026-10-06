#!/bin/bash
# Builds AAE.app: the Rust core, the Swift bindings, the app, and the helper APK.
#   macos/build.sh            release build into macos/build/AAE.app
#   macos/build.sh --debug    debug build, quicker to compile
set -euo pipefail

cd "$(dirname "$0")/.."
ROOT=$(pwd)
PROFILE=release
CARGO_FLAGS=(--release)
if [[ "${1:-}" == "--debug" ]]; then
    PROFILE=debug
    CARGO_FLAGS=()
fi
CARGO="${CARGO:-$(command -v cargo || echo "$HOME/.cargo/bin/cargo")}"

echo "Building the Rust core."
"$CARGO" build ${CARGO_FLAGS[@]+"${CARGO_FLAGS[@]}"} -p aae-ffi

echo "Generating the Swift bindings."
GEN="$ROOT/target/uniffi-swift"
rm -rf "$GEN"
"$CARGO" run ${CARGO_FLAGS[@]+"${CARGO_FLAGS[@]}"} -q -p aae-ffi --bin uniffi-bindgen -- \
    generate --library "target/$PROFILE/libaae_ffi.a" --language swift --out-dir "$GEN"
cp "$GEN/aae_ffi.swift" macos/Sources/AAE/Generated/
cp "$GEN/aae_ffiFFI.h" macos/Sources/aae_ffiFFI/include/

echo "Building the app."
(cd macos && AAE_RUST_LIB_DIR="$ROOT/target/$PROFILE" swift build -c "$PROFILE")
BIN=$(cd macos && swift build -c "$PROFILE" --show-bin-path)

if [[ -x android/gradlew ]] && command -v java >/dev/null; then
    echo "Building AAE's helper app."
    (cd android && ./gradlew -q :helper:assembleRelease) || echo "The helper app did not build; the app will look for it elsewhere."
fi

echo "Putting AAE.app together."
APP="$ROOT/macos/build/AAE.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN/AAE" "$APP/Contents/MacOS/AAE"
cp macos/Support/Info.plist "$APP/Contents/Info.plist"
HELPER=android/helper/build/outputs/apk/release/helper-release.apk
if [[ -f "$HELPER" ]]; then
    cp "$HELPER" "$APP/Contents/Resources/aae-helper.apk"
fi
codesign --force --sign - "$APP" >/dev/null
echo "Built $APP"
