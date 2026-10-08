#!/bin/bash
# Makes a release of AAE's Googlebook components: builds them with build.sh,
# puts the bundle and its sources archive in googlebook/dist, and pins the
# bundle's version, size and SHA-256 in crates/aae-core/src/googlebook/install.rs,
# so AAE builds from then on install it.
#
#   googlebook/release.sh
#
# Raise VERSION in googlebook/build.sh first whenever the components change:
# installed copies only fetch a bundle with a new version. Then publish both
# files in googlebook/dist as GitHub release googlebook-components-<version>,
# and commit install.rs. The release is separate from AAE's own: it's needed
# only when the components change, and each new one makes every Mac assemble
# its Googlebook disk again.
#
# Settings: as for build.sh.
set -euo pipefail

cd "$(dirname "$0")/.."
ROOT=$(pwd)
VERSION=$(sed -n 's/^VERSION=\([0-9][0-9]*\)$/\1/p' googlebook/build.sh)
[ -n "$VERSION" ] || { echo "googlebook/build.sh has no VERSION." >&2; exit 1; }
BUILD="${AAE_GOOGLEBOOK_BUILD:-$ROOT/target/googlebook}"

googlebook/build.sh

DIST="$ROOT/googlebook/dist"
rm -rf "$DIST"; mkdir -p "$DIST"
BUNDLE="aae-googlebook-$VERSION.tar.gz"
SOURCES="aae-googlebook-$VERSION-sources.tar.gz"
cp "$BUILD/$BUNDLE" "$BUILD/$SOURCES" "$DIST/"
SHA256=$(shasum -a 256 "$DIST/$BUNDLE" | cut -d' ' -f1)
SIZE=$(stat -f %z "$DIST/$BUNDLE")

INSTALL="$ROOT/crates/aae-core/src/googlebook/install.rs"
python3 -I - "$INSTALL" "$VERSION" "$SHA256" "$SIZE" <<'PY'
import re, sys
path, version, sha, size = sys.argv[1:]
text = open(path).read()
for name, value in [("COMPONENTS_VERSION: u32", version), ("COMPONENTS_SHA256: &str", f'"{sha}"'),
                    ("COMPONENTS_SIZE: u64", f"{int(size):_}")]:
    text, n = re.subn(rf"(const {re.escape(name)} = )[^;]+;", rf"\g<1>{value};", text)
    if n != 1:
        sys.exit(f"{path} has no {name.split(':')[0]}.")
open(path, "w").write(text)
PY

TAG="googlebook-components-$VERSION"
echo "Made $DIST/$BUNDLE ($SIZE bytes, SHA-256 $SHA256) and $DIST/$SOURCES."
echo "Pinned them in crates/aae-core/src/googlebook/install.rs."
echo "Publish both as release $TAG, for example:"
echo "  gh release create $TAG --latest=false --title \"Googlebook components $VERSION\" googlebook/dist/*"
echo "then commit install.rs."
