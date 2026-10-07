#!/bin/bash
# Makes a release of AAE: builds the app, packs it in a disk image with an
# Applications shortcut, signs the disk image with AAE's update key, and adds
# it to the update feed, appcast.xml at the top of the repository.
#
#   macos/release.sh [release-notes.html]
#
# Then publish the disk image at the download address, and commit and push
# appcast.xml; apps already installed find the update there.
#
# Settings:
#   AAE_ED_KEY_FILE       the private update key, base64, if it isn't in the
#                         login Keychain as the "aae" account
#   AAE_SIGN_IDENTITY     the certificate to sign with (default: "AAE Code Signing"
#                         from the login keychain); a Developer ID one is
#                         also used for the disk image, for notarisation
#   AAE_NOTARY_PROFILE    a notarytool keychain profile; with it, the disk
#                         image is notarised and stapled
#   AAE_DOWNLOAD_URL      where the disk image will be published (default:
#                         the GitHub release for this version)
#   AAE_APPCAST           the feed to add the release to (default: appcast.xml)
#   AAE_VERSION, AAE_BUILD_NUMBER, AAE_FEED_URL, AAE_ALLOW_LOCAL_HTTP: as for build.sh
set -euo pipefail

cd "$(dirname "$0")/.."
ROOT=$(pwd)
NOTES="${1:-}"

macos/build.sh
APP="$ROOT/macos/build/AAE.app"
VERSION=$(/usr/libexec/PlistBuddy -c "Print :CFBundleShortVersionString" "$APP/Contents/Info.plist")
BUILD_NUMBER=$(/usr/libexec/PlistBuddy -c "Print :CFBundleVersion" "$APP/Contents/Info.plist")
if ! /usr/libexec/PlistBuddy -c "Print :SUPublicEDKey" "$APP/Contents/Info.plist" >/dev/null 2>&1; then
    echo "The app has no public update key, so it couldn't check updates. Put it in macos/Support/sparkle-public-key.txt." >&2
    exit 1
fi

echo "Making the disk image."
DIST="$ROOT/macos/dist"
DMG="$DIST/AAE-$VERSION.dmg"
macos/dmg.sh "$APP" "$DMG" "AAE $VERSION"
if [[ "${AAE_SIGN_IDENTITY:-}" == "Developer ID Application"* ]]; then
    codesign --force --sign "$AAE_SIGN_IDENTITY" --timestamp "$DMG"
fi
if [[ -n "${AAE_NOTARY_PROFILE:-}" ]]; then
    echo "Notarising. This takes a few minutes."
    xcrun notarytool submit "$DMG" --keychain-profile "$AAE_NOTARY_PROFILE" --wait
    xcrun stapler staple "$DMG"
fi

echo "Signing the update."
SIGN_UPDATE=$(find macos/.build/artifacts -path '*/Sparkle/bin/sign_update' | head -1)
# AAE's key is the "aae" account in the login Keychain, made by generate_keys --account aae.
KEY_ARGS=(--account aae)
if [[ -n "${AAE_ED_KEY_FILE:-}" ]]; then
    KEY_ARGS=(--ed-key-file "$AAE_ED_KEY_FILE")
fi
# Prints: sparkle:edSignature="…" length="…"
SIGNATURE=$("$SIGN_UPDATE" ${KEY_ARGS[@]+"${KEY_ARGS[@]}"} "$DMG")

URL="${AAE_DOWNLOAD_URL:-https://github.com/aaron-gh/accessible-android-emulator/releases/download/v$VERSION}/AAE-$VERSION.dmg"
APPCAST="${AAE_APPCAST:-$ROOT/appcast.xml}"
python3 -I - "$APPCAST" "$VERSION" "$BUILD_NUMBER" "$URL" "$SIGNATURE" "$NOTES" <<'PY'
import email.utils, html, pathlib, sys
appcast, version, build, url, signature, notes = sys.argv[1:]
path = pathlib.Path(appcast)
if not path.exists():
    path.write_text("""<?xml version="1.0" encoding="utf-8"?>
<rss version="2.0" xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
    <channel>
        <title>AAE Updates</title>
        <description>Updates for AAE, the Accessible Android Emulator.</description>
        <language>en</language>
    </channel>
</rss>
""")
description = ""
if notes:
    description = f"\n            <description><![CDATA[{pathlib.Path(notes).read_text()}]]></description>"
item = f"""        <item>
            <title>Version {html.escape(version)}</title>
            <pubDate>{email.utils.formatdate()}</pubDate>
            <sparkle:version>{html.escape(build)}</sparkle:version>
            <sparkle:shortVersionString>{html.escape(version)}</sparkle:shortVersionString>
            <sparkle:minimumSystemVersion>13.0</sparkle:minimumSystemVersion>{description}
            <enclosure url="{html.escape(url)}" {signature} type="application/octet-stream" />
        </item>
"""
text = path.read_text()
if f"<sparkle:version>{build}</sparkle:version>" in text:
    sys.exit(f"The feed already has build {build}. Commit first, or set AAE_BUILD_NUMBER.")
# Newest first: straight after the channel's own details.
marker = "        <language>en</language>\n"
if marker not in text:
    sys.exit("The feed isn't laid out as this script expects.")
path.write_text(text.replace(marker, marker + item, 1))
PY
echo "Made $DMG, version $VERSION, build $BUILD_NUMBER."
echo "Publish it at $URL, then commit and push $(basename "$APPCAST")."
