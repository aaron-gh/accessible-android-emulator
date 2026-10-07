#!/bin/bash
# Builds AAE.app: the Rust core, the Swift bindings, the app, the aae command
# (which includes the MCP server), and the helper APK.
#   macos/build.sh            release build into macos/build/AAE.app
#   macos/build.sh --debug    debug build, quicker to compile
#
# Updates come through Sparkle. These settings shape the build:
#   AAE_FEED_URL          the update feed (default: appcast.xml in AAE's GitHub repository)
#   AAE_ED_PUBLIC_KEY     the public key updates are signed with (default:
#                         macos/Support/sparkle-public-key.txt; without one,
#                         AAE won't install updates)
#   AAE_VERSION           the version people see (default: the workspace's, in Cargo.toml)
#   AAE_BUILD_NUMBER      the build number Sparkle compares (default: the number
#                         of commits, which only goes up)
#   AAE_ALLOW_LOCAL_HTTP  1 lets the feed be plain http on this computer, for
#                         testing updates
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

echo "Building the Rust core and the aae command."
"$CARGO" build ${CARGO_FLAGS[@]+"${CARGO_FLAGS[@]}"} -p aae-ffi -p aae-cli

echo "Generating the Swift bindings."
GEN="$ROOT/target/uniffi-swift"
rm -rf "$GEN"
"$CARGO" run ${CARGO_FLAGS[@]+"${CARGO_FLAGS[@]}"} -q -p aae-ffi --bin uniffi-bindgen -- \
    generate --library "target/$PROFILE/libaae_ffi.a" --language swift --out-dir "$GEN"
# Both folders hold only generated files, which git ignores, so a fresh
# clone doesn't have them yet.
mkdir -p macos/Sources/AAE/Generated macos/Sources/aae_ffiFFI/include
cp "$GEN/aae_ffi.swift" macos/Sources/AAE/Generated/
cp "$GEN/aae_ffiFFI.h" macos/Sources/aae_ffiFFI/include/

echo "Building the app."
(cd macos && AAE_RUST_LIB_DIR="$ROOT/target/$PROFILE" swift build -c "$PROFILE")
BIN=$(cd macos && swift build -c "$PROFILE" --show-bin-path)

if [[ -x android/gradlew ]] && command -v java >/dev/null; then
    source android/signing.sh
    echo "Building AAE's helper app, signed with the $AAE_ANDROID_SIGNER key."
    (cd android && ./gradlew -q :helper:assembleRelease) || echo "The helper app did not build; the app will look for it elsewhere."
    android/build-espeak.sh --if-needed || echo "eSpeak NG did not build; the app will look for it elsewhere."
fi

echo "Putting AAE.app together."
APP="$ROOT/macos/build/AAE.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN/AAE" "$APP/Contents/MacOS/AAE"
# The aae command, for the terminal and for AI agents (aae mcp). It's in
# Helpers, as "aae" and "AAE" are the same name on a Mac's disk.
mkdir -p "$APP/Contents/Helpers"
cp "target/$PROFILE/aae" "$APP/Contents/Helpers/aae"
cp macos/Support/Info.plist "$APP/Contents/Info.plist"
PLIST="$APP/Contents/Info.plist"
VERSION="${AAE_VERSION:-$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)}"
BUILD_NUMBER="${AAE_BUILD_NUMBER:-$(git rev-list --count HEAD 2>/dev/null || echo 1)}"
FEED_URL="${AAE_FEED_URL:-https://raw.githubusercontent.com/aaron-gh/accessible-android-emulator/master/appcast.xml}"
PUBLIC_KEY="${AAE_ED_PUBLIC_KEY:-$(cat macos/Support/sparkle-public-key.txt 2>/dev/null || true)}"
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $VERSION" "$PLIST"
/usr/libexec/PlistBuddy -c "Set :CFBundleVersion $BUILD_NUMBER" "$PLIST"
/usr/libexec/PlistBuddy -c "Add :SUFeedURL string $FEED_URL" "$PLIST"
if [[ -n "$PUBLIC_KEY" ]]; then
    /usr/libexec/PlistBuddy -c "Add :SUPublicEDKey string $PUBLIC_KEY" "$PLIST"
fi
if [[ "${AAE_ALLOW_LOCAL_HTTP:-}" == 1 ]]; then
    /usr/libexec/PlistBuddy -c "Add :NSAppTransportSecurity dict" \
        -c "Add :NSAppTransportSecurity:NSAllowsLocalNetworking bool true" "$PLIST"
fi
mkdir -p "$APP/Contents/Frameworks"
SPARKLE=$(find macos/.build/artifacts -path '*Sparkle.xcframework/macos-*/Sparkle.framework' -maxdepth 6 | head -1)
cp -R "$SPARKLE" "$APP/Contents/Frameworks/"
HELPER=android/helper/build/outputs/apk/release/helper-release.apk
if [[ -f "$HELPER" ]]; then
    cp "$HELPER" "$APP/Contents/Resources/aae-helper.apk"
fi
if [[ -f android/espeak/build/aae-espeak.apk ]]; then
    cp android/espeak/build/aae-espeak.apk "$APP/Contents/Resources/aae-espeak.apk"
fi
# Sign from the inside out: Sparkle's helpers, its framework, then the app.
# The signature is what macOS remembers permissions by, so every build is
# signed with the same certificate when there is one: AAE_SIGN_IDENTITY, or
# else the self-made "AAE Code Signing" certificate in the login keychain.
# Without either, the build is signed for this Mac only, and its signature
# changes every build. A Developer ID certificate also gets the hardened
# runtime and timestamp that notarisation needs.
SIGN_ID="${AAE_SIGN_IDENTITY:-}"
if [[ -z "$SIGN_ID" ]]; then
    if security find-certificate -c "AAE Code Signing" >/dev/null 2>&1; then
        SIGN_ID="AAE Code Signing"
    else
        SIGN_ID="-"
    fi
fi
SIGN_FLAGS=(--force --sign "$SIGN_ID")
DEVELOPER_ID=0
if [[ "$SIGN_ID" == "Developer ID Application"* ]]; then
    DEVELOPER_ID=1
    SIGN_FLAGS+=(--options runtime --timestamp)
fi
FW="$APP/Contents/Frameworks/Sparkle.framework"
for part in "$FW"/Versions/B/XPCServices/*.xpc "$FW/Versions/B/Autoupdate" "$FW/Versions/B/Updater.app" "$FW"; do
    if [[ -e "$part" ]]; then
        codesign "${SIGN_FLAGS[@]}" "$part" >/dev/null
    fi
done
codesign "${SIGN_FLAGS[@]}" "$APP/Contents/Helpers/aae" >/dev/null
APP_SIGN_FLAGS=("${SIGN_FLAGS[@]}")
if [[ "$DEVELOPER_ID" == 1 ]]; then
    APP_SIGN_FLAGS+=(--entitlements macos/Support/AAE.entitlements)
fi
codesign "${APP_SIGN_FLAGS[@]}" "$APP" >/dev/null
if [[ "$SIGN_ID" == "-" ]]; then
    echo "Built $APP, signed for this Mac only."
else
    echo "Built $APP, signed with $SIGN_ID."
fi
