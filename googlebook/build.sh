#!/bin/bash
# Builds the component bundle for AAE's Googlebook devices:
#   host/    aae-vm (runs UTM's QEMU library with the hypervisor entitlement), and
#            virglrenderer with the macOS buffer-sharing patch, as a framework that
#            loads ahead of UTM's
#   guest/   Mesa (VirGL GLES and Venus Vulkan) built for Android, for the VM's
#            vendor partition
#   tools/   mkfs.erofs, dump.erofs, fsck.erofs and sepolicy-allow, which AAE uses
#            to assemble the disk on the user's Mac
# AAE downloads UTM, Google's Cuttlefish image and Google's Googlebook recovery
# image on the user's Mac itself; none of them is in the bundle.
#
# Output, to publish together as release googlebook-components-<version>:
#   aae-googlebook-<version>.tar.gz (and .sha256)
#   aae-googlebook-<version>-sources.tar.gz: the source of the GPL and LGPL
#     programs in the bundle (erofs-utils, libsepol), and of lz4, which is
#     linked into erofs-utils, with this script
# Needs: Xcode command line tools, Homebrew (autoconf, automake, libtool, bison,
# pkg-config), Python 3.10 or later, and the Android NDK (newest installed, or
# ANDROID_NDK).
#   AAE_GOOGLEBOOK_DOWNLOADS  a folder with the downloads already in it
#   AAE_GOOGLEBOOK_BUILD      where to build (default target/googlebook)
#   JOBS                      parallel jobs (default 4; Mesa needs a lot of memory)
set -euo pipefail

cd "$(dirname "$0")/.."
ROOT=$(pwd)
HERE="$ROOT/googlebook"
VERSION=1
BUILD="${AAE_GOOGLEBOOK_BUILD:-$ROOT/target/googlebook}"
DOWNLOADS="${AAE_GOOGLEBOOK_DOWNLOADS:-$BUILD/downloads}"
JOBS="${JOBS:-4}"
export MACOSX_DEPLOYMENT_TARGET=13.0

UTM_URL="https://github.com/utmapp/UTM/releases/download/v5.0.6/UTM.dmg"
UTM_SHA256="6a722486a660e0ab2cf5826bbeaee0f5963999029366709f5a3048d73b1d7cb1"
MESA_URL="https://archive.mesa3d.org/mesa-26.2.4.tar.xz"
MESA_SHA256="bce5f7fbebb934373b86c999a064d52fb5065878dc57f287f95346648ec832e9"
EPOXY_REPO="https://github.com/utmapp/libepoxy.git"
EPOXY_COMMIT="bf98587477fe68d07b93319ece7b40a7d0e2eabe"
VIRGL_REPO="https://github.com/utmapp/virglrenderer.git"
VIRGL_COMMIT="5d26f605f50f8e22002ec6db5fb775e1992d4e96"
LZ4_URL="https://github.com/lz4/lz4/archive/refs/tags/v1.10.0.tar.gz"
LZ4_SHA256="537512904744b35e232912055ccf8ec66d768639ff3abe5788d90d792ec5f48b"
EROFS_URL="https://git.kernel.org/pub/scm/linux/kernel/git/xiang/erofs-utils.git/snapshot/erofs-utils-1.9.4.tar.gz"
EROFS_SHA256="7d135aa2550326a5acf20f53c518aea5a8900015ce50700044e40f818c31dd80"
LIBSEPOL_URL="https://github.com/SELinuxProject/selinux/releases/download/3.11/libsepol-3.11.tar.gz"
LIBSEPOL_SHA256="79f3d2c88f44b7eb5cf54d9792e03232297e17f97a179163f2750099a00f164d"

say() { printf '\n== %s\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }
# Downloads a file once and checks it.
fetch() { # url name sha256
    local dest="$DOWNLOADS/$2"
    if [ ! -f "$dest" ]; then
        mkdir -p "$DOWNLOADS"
        curl -fL --retry 5 -C - -o "$dest.part" "$1"
        mv "$dest.part" "$dest"
    fi
    [ "$(shasum -a 256 "$dest" | cut -d' ' -f1)" = "$3" ] || die "checksum mismatch for $dest"
}
# Checks out one commit of a repository.
checkout() { # url commit dir
    if [ ! -d "$3/.git" ]; then git init -q "$3"; git -C "$3" remote add origin "$1"; fi
    if [ "$(git -C "$3" rev-parse HEAD 2>/dev/null || true)" != "$2" ]; then
        git -C "$3" fetch -q --depth 1 origin "$2"
        git -C "$3" checkout -q --force FETCH_HEAD
    fi
}
# Gives a pinned checkout exactly this patch.
apply_patch() { # repo patch
    if ! git -C "$1" apply --reverse --check "$2" 2>/dev/null; then
        git -C "$1" checkout -q --force HEAD -- .
        git -C "$1" clean -qfd
        git -C "$1" apply "$2"
    fi
}

[ "$(uname -m)" = arm64 ] || die "build on an Apple silicon Mac"
for tool in git clang codesign install_name_tool pkg-config autoconf automake glibtoolize; do
    command -v "$tool" >/dev/null || die "missing $tool"
done
BISON="$(brew --prefix bison)/bin"
[ -x "$BISON/bison" ] || die "missing bison 3 (brew install bison)"
ANDROID_SDK="${ANDROID_HOME:-$HOME/Library/Android/sdk}"
ANDROID_NDK="${ANDROID_NDK:-$(ls -d "$ANDROID_SDK"/ndk/[0-9]* 2>/dev/null | sort -V | tail -1)}"
NDK_BIN="$ANDROID_NDK/toolchains/llvm/prebuilt/darwin-x86_64/bin"
[ -x "$NDK_BIN/aarch64-linux-android35-clang" ] || die "no Android NDK with an API 35 arm64 compiler (set ANDROID_NDK)"

SRC="$BUILD/src"; OBJ="$BUILD/obj"; STAGE="$BUILD/aae-googlebook-$VERSION"
mkdir -p "$SRC" "$OBJ" "$BUILD/pkgconfig" "$BUILD/android-pkgconfig"

say "Python build environment"
if [ ! -x "$BUILD/env/bin/meson" ]; then
    PY=
    for c in python3.14 python3.13 python3.12 python3.11 python3.10; do
        command -v "$c" >/dev/null && { PY=$c; break; }
    done
    [ -n "$PY" ] || die "missing Python 3.10 or later"
    "$PY" -m venv "$BUILD/env"
    "$BUILD/env/bin/pip" -q install meson==1.12.1 ninja==1.13.2 mako pyyaml packaging
fi
export PATH="$BUILD/env/bin:$BISON:$PATH" CCACHE_DISABLE=1

say "UTM 5.0.6 (its frameworks to build against)"
UTM="$BUILD/UTM.app"
if [ ! -d "$UTM" ]; then
    fetch "$UTM_URL" UTM-5.0.6.dmg "$UTM_SHA256"
    mnt="$(mktemp -d)"
    hdiutil attach -quiet -nobrowse -readonly -mountpoint "$mnt" "$DOWNLOADS/UTM-5.0.6.dmg"
    cp -R "$mnt/UTM.app" "$UTM"
    hdiutil detach -quiet "$mnt"; rmdir "$mnt"
fi
FW="$UTM/Contents/Frameworks"

say "Mesa source"
fetch "$MESA_URL" mesa-26.2.4.tar.xz "$MESA_SHA256"
MESA="$SRC/mesa-26.2.4"
[ -d "$MESA" ] || tar -xf "$DOWNLOADS/mesa-26.2.4.tar.xz" -C "$SRC"

say "virglrenderer with the macOS buffer-sharing patch"
checkout "$EPOXY_REPO" "$EPOXY_COMMIT" "$SRC/libepoxy"
[ -f "$OBJ/libepoxy/build.ninja" ] || meson setup "$OBJ/libepoxy" "$SRC/libepoxy" \
    -Dtests=false -Dglx=no -Degl=yes -Dx11=false >/dev/null
ninja -j "$JOBS" -C "$OBJ/libepoxy" include/epoxy/gl_generated.h include/epoxy/egl_generated.h \
    include/epoxy/gl_angle_ext_generated.h include/epoxy/egl_angle_ext_generated.h >/dev/null
cat > "$BUILD/pkgconfig/epoxy.pc" <<PC
Name: epoxy
Description: UTM's epoxy with matching generated headers
Version: 1.5.10
epoxy_has_egl=1
epoxy_has_glx=0
Cflags: -I$MESA/include -I$SRC/libepoxy/include -I$OBJ/libepoxy/include
Libs: -F$FW -framework epoxy.0
PC
cat > "$BUILD/pkgconfig/vulkan.pc" <<PC
Name: vulkan
Description: UTM's Vulkan loader
Version: 1.4.0
Libs: -F$FW -framework vulkan.1
PC
checkout "$VIRGL_REPO" "$VIRGL_COMMIT" "$SRC/virglrenderer"
apply_patch "$SRC/virglrenderer" "$HERE/patches/virglrenderer-android-interop.patch"
[ -f "$OBJ/virglrenderer/build.ninja" ] || PKG_CONFIG_PATH="$BUILD/pkgconfig" meson setup \
    "$OBJ/virglrenderer" "$SRC/virglrenderer" --buildtype=release \
    -Dvenus=true -Dneptune=true -Dvulkan-dload=false -Dplatforms=egl -Dtests=false -Dvtest=false \
    -Dcheck-gl-errors=false >/dev/null
ninja -j "$JOBS" -C "$OBJ/virglrenderer" >/dev/null

say "Mesa for Android (VirGL GLES, and Venus Vulkan with the mapper patch)"
cat > "$BUILD/android-aarch64.cross" <<CROSS
[constants]
ndk = '$ANDROID_NDK/toolchains/llvm/prebuilt/darwin-x86_64'
[binaries]
c = ndk / 'bin/aarch64-linux-android35-clang'
cpp = [ndk / 'bin/aarch64-linux-android35-clang++', '-fno-exceptions', '-fno-unwind-tables', '-fno-asynchronous-unwind-tables', '--start-no-unused-arguments', '-static-libstdc++', '--end-no-unused-arguments']
ar = ndk / 'bin/llvm-ar'
strip = ndk / 'bin/llvm-strip'
c_ld = 'lld'
cpp_ld = 'lld'
pkg-config = '$(command -v pkg-config)'
[host_machine]
system = 'android'
cpu_family = 'aarch64'
cpu = 'armv8'
endian = 'little'
[properties]
needs_exe_wrapper = true
pkg_config_libdir = '$BUILD/android-pkgconfig'
CROSS
COMMON=(--cross-file "$BUILD/android-aarch64.cross" --buildtype=release -Dforce_fallback_for=libdrm,expat
    -Dallow-fallback-for=libdrm -Dplatforms=android -Dplatform-sdk-version=35 -Dandroid-stub=true
    -Dandroid-libbacktrace=disabled -Dllvm=disabled -Dglx=disabled -Dgbm=disabled -Dvideo-codecs=
    -Dgallium-va=disabled -Dzstd=disabled)
[ -f "$OBJ/mesa-gles/build.ninja" ] || meson setup "$OBJ/mesa-gles" "$MESA" "${COMMON[@]}" \
    -Dgallium-drivers=virgl -Dvulkan-drivers= -Degl=enabled -Dgles1=enabled -Dgles2=enabled -Dopengl=true \
    -Degl-lib-suffix=_virgl -Dgles-lib-suffix=_virgl >"$OBJ/mesa-gles.configure.log" 2>&1 \
    || { tail -20 "$OBJ/mesa-gles.configure.log"; die "Mesa GLES configure failed"; }
ninja -j "$JOBS" -C "$OBJ/mesa-gles" >"$OBJ/mesa-gles.log" 2>&1 || { tail -20 "$OBJ/mesa-gles.log"; die "Mesa GLES build failed"; }
VENUS="$SRC/mesa-26.2.4-venus"
PATCH_ID="$(shasum -a 256 "$HERE/patches/mesa-android-mapper5.patch" | cut -d' ' -f1)"
if [ "$(cat "$VENUS/.aae-patch" 2>/dev/null)" != "$PATCH_ID" ]; then
    rm -rf "$VENUS" "$OBJ/mesa-venus"
    cp -c -R "$MESA" "$VENUS" 2>/dev/null || cp -R "$MESA" "$VENUS"
    patch -s -p1 -d "$VENUS" < "$HERE/patches/mesa-android-mapper5.patch"
    echo "$PATCH_ID" > "$VENUS/.aae-patch"
fi
[ -f "$OBJ/mesa-venus/build.ninja" ] || meson setup "$OBJ/mesa-venus" "$VENUS" "${COMMON[@]}" \
    --wrap-mode=nodownload -Dgallium-drivers= -Dvulkan-drivers=virtio -Degl=disabled -Dgles1=disabled \
    -Dgles2=disabled -Dopengl=false >"$OBJ/mesa-venus.configure.log" 2>&1 \
    || { tail -20 "$OBJ/mesa-venus.configure.log"; die "Mesa Venus configure failed"; }
ninja -j "$JOBS" -C "$OBJ/mesa-venus" src/virtio/vulkan/libvulkan_virtio.so >"$OBJ/mesa-venus.log" 2>&1 \
    || { tail -20 "$OBJ/mesa-venus.log"; die "Mesa Venus build failed"; }

say "lz4 and erofs-utils, linked statically"
fetch "$LZ4_URL" lz4-1.10.0.tar.gz "$LZ4_SHA256"
[ -d "$SRC/lz4-1.10.0" ] || tar -xzf "$DOWNLOADS/lz4-1.10.0.tar.gz" -C "$SRC"
make -s -C "$SRC/lz4-1.10.0/lib" liblz4.a >/dev/null
mkdir -p "$OBJ/lz4/lib" "$OBJ/lz4/include"
cp "$SRC/lz4-1.10.0/lib/liblz4.a" "$OBJ/lz4/lib/"
cp "$SRC/lz4-1.10.0/lib/"lz4*.h "$OBJ/lz4/include/"
fetch "$EROFS_URL" erofs-utils-1.9.4.tar.gz "$EROFS_SHA256"
EROFS="$SRC/erofs-utils-1.9.4"
if [ ! -x "$EROFS/mkfs/mkfs.erofs" ]; then
    [ -d "$EROFS" ] || tar -xzf "$DOWNLOADS/erofs-utils-1.9.4.tar.gz" -C "$SRC"
    # pkg-config is kept from Homebrew's libraries, so only the static lz4 and
    # the system's zlib go in.
    (cd "$EROFS" && ./autogen.sh && PKG_CONFIG_LIBDIR=/nonexistent \
        liblz4_CFLAGS="-I$OBJ/lz4/include" liblz4_LIBS="-L$OBJ/lz4/lib -llz4" ./configure \
        --disable-multithreading --disable-lzma --without-libcurl --without-openssl --without-libxml2 \
        --without-json-c --without-libnl3 --without-uuid && make -j "$JOBS") >"$OBJ/erofs-utils.log" 2>&1 \
        || { tail -20 "$OBJ/erofs-utils.log"; die "erofs-utils build failed"; }
fi

say "sepolicy-allow"
fetch "$LIBSEPOL_URL" libsepol-3.11.tar.gz "$LIBSEPOL_SHA256"
[ -d "$SRC/libsepol-3.11" ] || tar -xzf "$DOWNLOADS/libsepol-3.11.tar.gz" -C "$SRC"
[ -f "$SRC/libsepol-3.11/src/libsepol.a" ] || make -s -C "$SRC/libsepol-3.11/src" libsepol.a \
    CFLAGS="-O2 -Wno-error -D_DARWIN_C_SOURCE" >"$OBJ/libsepol.log" 2>&1 || { tail -20 "$OBJ/libsepol.log"; die "libsepol build failed"; }

say "Putting the bundle together"
rm -rf "$STAGE"
mkdir -p "$STAGE/host/Frameworks/virglrenderer.1.framework/Versions/A" "$STAGE/guest/vendor/lib64/egl" \
    "$STAGE/guest/vendor/lib64/hw" "$STAGE/tools"
# virglrenderer takes the place of UTM's through DYLD_FRAMEWORK_PATH, so
# UTM's QEMU library is used unchanged.
VIRGL="$STAGE/host/Frameworks/virglrenderer.1.framework/Versions/A/virglrenderer.1"
cp "$OBJ/virglrenderer/src/libvirglrenderer.1.dylib" "$VIRGL"
install_name_tool -id @rpath/virglrenderer.1.framework/Versions/A/virglrenderer.1 "$VIRGL" 2>/dev/null
cp "$OBJ/virglrenderer/server/virgl_render_server" "$STAGE/host/virgl_render_server"
install_name_tool -change @rpath/libvirglrenderer.1.dylib @rpath/virglrenderer.1.framework/Versions/A/virglrenderer.1 \
    "$STAGE/host/virgl_render_server" 2>/dev/null
codesign --force --sign - "$VIRGL" "$STAGE/host/virgl_render_server" 2>/dev/null
clang -O2 -Wall "$HERE/aae-vm.c" -o "$STAGE/host/aae-vm"
codesign --force --sign - --entitlements "$HERE/hypervisor.entitlements" "$STAGE/host/aae-vm"
for pair in src/egl/libEGL_virgl.so:lib64/egl src/mesa/glapi/es1api/libGLESv1_CM_virgl.so:lib64/egl \
            src/mesa/glapi/es2api/libGLESv2_virgl.so:lib64/egl src/gallium/targets/dri/libgallium_dri.so:lib64 \
            subprojects/libdrm-2.4.133/libdrm.so:lib64; do
    "$NDK_BIN/llvm-strip" --strip-unneeded -o "$STAGE/guest/vendor/${pair#*:}/$(basename "${pair%%:*}")" \
        "$OBJ/mesa-gles/${pair%%:*}"
done
cp "$OBJ/mesa-venus/src/virtio/vulkan/libvulkan_virtio.so" "$STAGE/guest/vendor/lib64/hw/vulkan.virtio.so"
cp "$EROFS/mkfs/mkfs.erofs" "$EROFS/dump/dump.erofs" "$EROFS/fsck/fsck.erofs" "$STAGE/tools/"
clang -O2 -I"$SRC/libsepol-3.11/include" "$HERE/sepolicy-allow.c" "$SRC/libsepol-3.11/src/libsepol.a" \
    -o "$STAGE/tools/sepolicy-allow"
for tool in "$STAGE"/tools/*; do
    otool -L "$tool" | grep -q /opt/homebrew && die "$tool links a Homebrew library"
    codesign --force --sign - "$tool" 2>/dev/null
done
cp "$HERE/NOTICE.md" "$STAGE/NOTICE.md"
(cd "$STAGE" && find . -type f ! -name manifest.json | sort | while read -r f; do
    printf '%s  %s\n' "$(shasum -a 256 "$f" | cut -d' ' -f1)" "${f#./}"
done) > "$STAGE/files.sha256"
printf '{"version": %s, "mesa": "26.2.4", "virglrenderer": "%s", "utm": "5.0.6"}\n' \
    "$VERSION" "$VIRGL_COMMIT" > "$STAGE/manifest.json"
tar -czf "$BUILD/aae-googlebook-$VERSION.tar.gz" -C "$BUILD" "aae-googlebook-$VERSION"
shasum -a 256 "$BUILD/aae-googlebook-$VERSION.tar.gz" | cut -d' ' -f1 > "$BUILD/aae-googlebook-$VERSION.tar.gz.sha256"
SOURCES="$BUILD/aae-googlebook-$VERSION-sources"
rm -rf "$SOURCES"; mkdir -p "$SOURCES/googlebook"
cp "$DOWNLOADS/erofs-utils-1.9.4.tar.gz" "$DOWNLOADS/libsepol-3.11.tar.gz" "$DOWNLOADS/lz4-1.10.0.tar.gz" "$SOURCES/"
cp -R "$HERE/" "$SOURCES/googlebook/"
tar -czf "$SOURCES.tar.gz" -C "$BUILD" "aae-googlebook-$VERSION-sources"
say "Built $BUILD/aae-googlebook-$VERSION.tar.gz ($(du -h "$BUILD/aae-googlebook-$VERSION.tar.gz" | cut -f1)) and its sources"
