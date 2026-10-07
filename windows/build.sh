#!/bin/bash
# Builds AAE for Windows, on the Mac: the app, the aae command, and the
# Android parts, cross-compiled with MinGW-w64 and zipped up with NVDA's
# controller client.
#   windows/build.sh    makes windows/dist/AAE-<version>-windows-x64.zip
#
#   AAE_ZIP_NAME     the zip's name, without .zip (default AAE-<version>-windows-x64)
#   AAE_BUILD_LABEL  what the app calls this build, after its version, such
#                    as "development build 42, from 1a2b3c4"
#
# Needs MinGW-w64 (brew install mingw-w64) and Rust's Windows target, which
# this adds if it's missing.
set -euo pipefail

cd "$(dirname "$0")/.."
CARGO="${CARGO:-$(command -v cargo || echo "$HOME/.cargo/bin/cargo")}"
RUSTUP="${RUSTUP:-$(command -v rustup || echo "$HOME/.cargo/bin/rustup")}"
TARGET=x86_64-pc-windows-gnu

if ! command -v x86_64-w64-mingw32-gcc >/dev/null; then
    echo "MinGW-w64 is needed to build for Windows. Install it with: brew install mingw-w64" >&2
    exit 1
fi
if ! "$RUSTUP" target list --installed | grep -qx "$TARGET"; then
    echo "Adding Rust's Windows target."
    "$RUSTUP" target add "$TARGET"
fi

echo "Building the Windows app and the aae command."
"$CARGO" build --release --target "$TARGET" -p aae-windows -p aae-cli

if [[ -x android/gradlew ]] && command -v java >/dev/null; then
    source android/signing.sh
    echo "Building AAE's helper app, signed with the $AAE_ANDROID_SIGNER key."
    (cd android && ./gradlew -q :helper:assembleRelease) || echo "The helper app did not build."
    android/build-espeak.sh --if-needed || echo "eSpeak NG did not build."
fi

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
NAME="${AAE_ZIP_NAME:-AAE-$VERSION-windows-x64}"
STAGE="windows/build/$NAME"
rm -rf "$STAGE"
mkdir -p "$STAGE" windows/dist
BIN="target/$TARGET/release"
cp "$BIN/AccessibleAndroidEmulator.exe" "$BIN/aae.exe" "$STAGE/"
HELPER=android/helper/build/outputs/apk/release/helper-release.apk
if [[ -f "$HELPER" ]]; then
    cp "$HELPER" "$STAGE/aae-helper.apk"
else
    echo "Warning: AAE's helper app isn't built, so the zip doesn't have it." >&2
fi
if [[ -f android/espeak/build/aae-espeak.apk ]]; then
    cp android/espeak/build/aae-espeak.apk "$STAGE/aae-espeak.apk"
else
    echo "Warning: AAE's eSpeak NG isn't built, so the zip doesn't have it." >&2
fi
# NV Access's NVDA controller client, which AAE speaks through when NVDA is
# running. Pinned, and checked against its known checksum.
NVDA_VERSION=2026.2
NVDA_SHA256=510736f021aefa33378076fa342a4524b586bc1c6b096db9479af42b28aac649
NVDA_ZIP="windows/build/cache/nvda_${NVDA_VERSION}_controllerClient.zip"
if [[ ! -f "$NVDA_ZIP" ]]; then
    echo "Downloading NVDA's controller client $NVDA_VERSION."
    mkdir -p windows/build/cache
    curl -fsSL -o "$NVDA_ZIP.part" \
        "https://download.nvaccess.org/releases/$NVDA_VERSION/nvda_${NVDA_VERSION}_controllerClient.zip"
    mv "$NVDA_ZIP.part" "$NVDA_ZIP"
fi
if [[ "$(shasum -a 256 "$NVDA_ZIP" | cut -d' ' -f1)" != "$NVDA_SHA256" ]]; then
    echo "NVDA's controller client doesn't match its checksum. Delete $NVDA_ZIP and try again." >&2
    exit 1
fi
unzip -qjo "$NVDA_ZIP" x64/nvdaControllerClient.dll -d "$STAGE"
unzip -qpo "$NVDA_ZIP" license.txt | sed 's/$/\r/' > "$STAGE/nvdaControllerClient-LICENSE.txt"

cp LICENSE "$STAGE/LICENSE.txt"
# Windows line endings, so Notepad shows it properly.
sed 's/$/\r/' windows/README.txt > "$STAGE/README.txt"

ZIP="$PWD/windows/dist/$NAME.zip"
rm -f "$ZIP"
(cd windows/build && zip -qr "$ZIP" "$NAME")
echo "Built $ZIP"
