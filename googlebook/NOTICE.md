# Notices for AAE's Googlebook components

The component bundle built by `googlebook/build.sh` contains software from these projects, under their own licences.

## gbos-vm

AAE's Googlebook support follows gbos-vm (https://github.com/skylartaylor/gbos-vm), which first ran Googlebook OS in a VM on Apple silicon. `patches/virglrenderer-android-interop.patch` and `patches/mesa-android-mapper5.patch` are gbos-vm's, unchanged. AAE's Cuttlefish extraction and image assembly (`crates/aae-core/src/googlebook/`) are adapted from its scripts.

    MIT License

    Copyright (c) 2026 Skylar Taylor-Barrick

    Permission is hereby granted, free of charge, to any person obtaining a copy
    of this software and associated documentation files (the "Software"), to deal
    in the Software without restriction, including without limitation the rights
    to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
    copies of the Software, and to permit persons to whom the Software is
    furnished to do so, subject to the following conditions:

    The above copyright notice and this permission notice shall be included in all
    copies or substantial portions of the Software.

    THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
    IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
    FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
    AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
    LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
    OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
    SOFTWARE.

## Built from source

| Component | Where | Licence | Source |
|---|---|---|---|
| Mesa 26.2.4 (VirGL GLES, Venus Vulkan), libdrm | `guest/vendor/lib64` | MIT | https://archive.mesa3d.org/mesa-26.2.4.tar.xz, with gbos-vm's mapper patch for Venus |
| virglrenderer (UTM's fork, commit 5d26f605) | `host/` | MIT | https://github.com/utmapp/virglrenderer, with gbos-vm's interop patch |
| erofs-utils 1.9.4 | `tools/` | GPL-2.0-or-later | https://git.kernel.org/pub/scm/linux/kernel/git/xiang/erofs-utils.git |
| lz4 1.10.0 library (in erofs-utils) | `tools/` | BSD-2-Clause | https://github.com/lz4/lz4 |
| libsepol 3.11 (in sepolicy-allow) | `tools/` | LGPL-2.1-or-later | https://github.com/SELinuxProject/selinux |

The source of erofs-utils, libsepol and lz4, with this folder's build script, is published with each bundle as `aae-googlebook-<version>-sources.tar.gz`.

## Downloaded on the user's Mac, not in the bundle

- Google's Googlebook recovery image (dl.google.com), under Google's terms.
- Google's Cuttlefish image, Android CI build 16373615 (ci.android.com), from which AAE extracts software KeyMint and Gatekeeper, audio, boot control, the minigbm allocator, the DRM composer and virtio kernel modules: Android Open Source Project, Apache License 2.0; the kernel modules GPL-2.0.
- UTM 5.0.6 (https://github.com/utmapp/UTM), Apache License 2.0, which contains QEMU (GPL-2.0) and other libraries under their own licences.
