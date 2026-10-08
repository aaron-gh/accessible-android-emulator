//! Parts of Google's Cuttlefish image, which AAE downloads on this Mac and
//! extracts: software KeyMint and Gatekeeper, audio, boot control, the
//! minigbm allocator, the DRM composer, and the virtio and DMA-heap kernel
//! modules, all from the Android Open Source Project. They take the place of
//! hardware the VM doesn't have.
//!
//! The image is read without mounting it: Android's sparse format, the super
//! partition's metadata, EROFS (with the component bundle's dump.erofs), and
//! the ext4 file systems inside APEX packages. Which files to take follows
//! gbos-vm (MIT licence, see googlebook/NOTICE.md).

use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use super::image::{Extent, Super, Tools, cpio_read, lz4_decompress};
use crate::error::{Error, IoContext, Result};

pub const BUILD: u32 = 16373615;
pub const ZIP: &str = "aosp_cf_arm64_only_phone-img-16373615.zip";
pub const ZIP_SHA256: &str = "051caf8072ba9fb417e05999de2984752e44e13ce70b6c49c669f0a73db85c18";
pub const ZIP_SIZE: u64 = 1_101_175_103;
const PAGE: &str = "https://ci.android.com/builds/submitted/16373615/aosp_cf_arm64_only_phone-userdebug/latest/aosp_cf_arm64_only_phone-img-16373615.zip";

const KEYMINT: &str = "com.android.hardware.keymint.rust_nonsecure.apex";
const BOOT: &str = "com.android.hardware.boot.apex";
const GRALLOC: &str = "com.google.cf.gralloc.apex";
const COMPOSER: &str = "com.android.hardware.graphics.composer.drm_hwcomposer.apex";

enum From {
    /// The vendor partition.
    Vendor(&'static str),
    /// An APEX package in the vendor partition, whole.
    Apex(&'static str),
    /// A file inside an APEX package's payload.
    InApex(&'static str, &'static str),
}

/// Vendor-partition path in the VM, and where it comes from.
const FILES: [(&str, From); 17] = [
    (
        "apex/com.android.hardware.audio.apex",
        From::Apex("com.android.hardware.audio.apex"),
    ),
    (
        "apex/com.android.hardware.gatekeeper.nonsecure.apex",
        From::Apex("com.android.hardware.gatekeeper.nonsecure.apex"),
    ),
    (
        "bin/hw/android.hardware.security.keymint-service",
        From::InApex(
            KEYMINT,
            "bin/hw/android.hardware.security.keymint-service.nonsecure",
        ),
    ),
    (
        "lib64/vm_keymint/libcrypto.so",
        From::InApex(KEYMINT, "lib64/libcrypto.so"),
    ),
    (
        "etc/vintf/manifest/vm-android.hardware.security.keymint-service.xml",
        From::InApex(
            KEYMINT,
            "etc/vintf/android.hardware.security.keymint-service.xml",
        ),
    ),
    (
        "etc/vintf/manifest/vm-android.hardware.security.secureclock-service.xml",
        From::InApex(
            KEYMINT,
            "etc/vintf/android.hardware.security.secureclock-service.xml",
        ),
    ),
    (
        "etc/vintf/manifest/vm-android.hardware.security.sharedsecret-service.xml",
        From::InApex(
            KEYMINT,
            "etc/vintf/android.hardware.security.sharedsecret-service.xml",
        ),
    ),
    (
        "bin/hw/android.hardware.boot-service.android-desktop",
        From::InApex(BOOT, "bin/hw/android.hardware.boot-service.default"),
    ),
    (
        "lib64/libminigbm_gralloc.so",
        From::InApex(GRALLOC, "lib64/libminigbm_gralloc.so"),
    ),
    (
        "lib64/libminigbm_gralloc4_utils.so",
        From::InApex(GRALLOC, "lib64/libminigbm_gralloc4_utils.so"),
    ),
    (
        "lib64/android.hardware.graphics.allocator-V3-ndk.so",
        From::InApex(
            GRALLOC,
            "lib64/android.hardware.graphics.allocator-V3-ndk.so",
        ),
    ),
    (
        "lib64/android.hardware.graphics.common-V7-ndk.so",
        From::InApex(GRALLOC, "lib64/android.hardware.graphics.common-V7-ndk.so"),
    ),
    (
        "bin/hw/android.hardware.graphics.allocator-service.minigbm",
        From::InApex(
            GRALLOC,
            "bin/hw/android.hardware.graphics.allocator-service.minigbm",
        ),
    ),
    (
        "lib64/hw/mapper.minigbm.so",
        From::Vendor("lib64/hw/mapper.minigbm.so"),
    ),
    (
        "lib64/hw/gralloc.default.so",
        From::Vendor("lib64/hw/gralloc.default.so"),
    ),
    (
        "bin/hw/android.hardware.composer.hwc3-service.drm",
        From::InApex(
            COMPOSER,
            "bin/hw/android.hardware.composer.hwc3-service.drm",
        ),
    ),
    (
        "lib64/drm_hwcomposer_atom_reporter.so",
        From::InApex(COMPOSER, "lib64/drm_hwcomposer_atom_reporter.so"),
    ),
];

pub const AUDIO: [&str; 10] = [
    "audio_effects.xml",
    "audio_effects_config.xml",
    "audio_policy_configuration.xml",
    "audio_policy_volumes.xml",
    "default_volume_tables.xml",
    "bluetooth_with_le_audio_policy_configuration_7_0.xml",
    "primary_audio_policy_configuration.xml",
    "r_submix_audio_policy_configuration.xml",
    "surround_sound_configuration_5_0.xml",
    "usb_audio_policy_configuration.xml",
];

/// Kernel modules and module lists from the first vendor ramdisk.
const RAMDISK_MODULES: [&str; 8] = [
    "virtio_dma_buf.ko",
    "virtio-gpu.ko",
    "virtio_input.ko",
    "virtio_blk.ko",
    "modules.alias",
    "modules.dep",
    "modules.softdep",
    "modules.options",
];

fn fail(what: impl Into<String>) -> Error {
    Error::Vm(format!(
        "Google's Cuttlefish image can't be read: {}",
        what.into()
    ))
}

/// The address the Android CI page gives out for the image. It's signed and
/// short-lived, so it's looked up for each download.
pub fn download_url() -> Result<String> {
    let page = ureq::get(PAGE)
        .call()
        .map_err(|e| {
            Error::Download(format!(
                "Couldn't reach Android CI for Google's Cuttlefish image: {e}"
            ))
        })?
        .body_mut()
        .read_to_string()
        .map_err(|e| Error::Download(format!("Couldn't read Android CI's page: {e}")))?;
    let start = page.find("var JSVariables = ").ok_or_else(|| {
        Error::Download("Android CI's page has changed; AAE can't find the image on it.".into())
    })?;
    let rest = &page[start + "var JSVariables = ".len()..];
    let json = rest.split("};").next().unwrap_or_default().to_string() + "}";
    let value: serde_json::Value = serde_json::from_str(&json).map_err(|_| {
        Error::Download("Android CI's page has changed; AAE can't find the image on it.".into())
    })?;
    value["artifactUrl"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| Error::Download("Android CI's page has no address for the image.".into()))
}

/// Expands an Android sparse image into a (sparse) regular file.
fn unsparse(input: &mut impl Read, out: &Path) -> Result<()> {
    let io = |e: std::io::Error| fail(e.to_string());
    let mut header = [0u8; 28];
    input.read_exact(&mut header).map_err(io)?;
    let u16le = |b: &[u8], o: usize| u16::from_le_bytes([b[o], b[o + 1]]) as u64;
    let u32le = |b: &[u8], o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap()) as u64;
    if u32le(&header, 0) != 0xed26_ff3a || u16le(&header, 4) != 1 {
        return Err(fail("super.img isn't a sparse image"));
    }
    let (header_size, chunk_header, block, blocks, chunks) = (
        u16le(&header, 8),
        u16le(&header, 10),
        u32le(&header, 12),
        u32le(&header, 16),
        u32le(&header, 20),
    );
    std::io::copy(
        &mut input.by_ref().take(header_size - 28),
        &mut std::io::sink(),
    )
    .map_err(io)?;
    let mut file = std::fs::File::create(out).map_err(io)?;
    let mut buffer = vec![0u8; 8 << 20];
    for _ in 0..chunks {
        let mut ch = vec![0u8; chunk_header as usize];
        input.read_exact(&mut ch).map_err(io)?;
        let (kind, count) = (u16le(&ch, 0), u32le(&ch, 4));
        let mut n = count * block;
        match kind {
            0xcac1 => {
                while n > 0 {
                    let k = n.min(buffer.len() as u64) as usize;
                    input.read_exact(&mut buffer[..k]).map_err(io)?;
                    file.write_all(&buffer[..k]).map_err(io)?;
                    n -= k as u64;
                }
            }
            0xcac2 => {
                let mut fill = [0u8; 4];
                input.read_exact(&mut fill).map_err(io)?;
                if fill == [0; 4] {
                    file.seek(SeekFrom::Current(n as i64)).map_err(io)?;
                } else {
                    let pattern: Vec<u8> =
                        fill.iter().copied().cycle().take(buffer.len()).collect();
                    while n > 0 {
                        let k = n.min(pattern.len() as u64) as usize;
                        file.write_all(&pattern[..k]).map_err(io)?;
                        n -= k as u64;
                    }
                }
            }
            0xcac3 => {
                file.seek(SeekFrom::Current(n as i64)).map_err(io)?;
            }
            0xcac4 => {
                let mut crc = [0u8; 4];
                input.read_exact(&mut crc).map_err(io)?;
            }
            _ => return Err(fail("super.img has an unknown chunk")),
        }
    }
    file.set_len(blocks * block).map_err(io)?;
    Ok(())
}

/// A read-only ext4 file system in memory, as APEX packages hold. Supports
/// what they use: extents, linear directory reads, 32-byte group descriptors.
pub(crate) struct Ext4<'a> {
    data: &'a [u8],
    block: usize,
    inodes_per_group: usize,
    inode_size: usize,
    descriptors: usize,
    descriptor_size: usize,
}

impl<'a> Ext4<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Result<Self> {
        let sb = data
            .get(1024..2048)
            .ok_or_else(|| fail("an APEX payload is too short"))?;
        let u32le = |o: usize| u32::from_le_bytes(sb[o..o + 4].try_into().unwrap()) as usize;
        if u16::from_le_bytes([sb[56], sb[57]]) != 0xef53 {
            return Err(fail("an APEX payload isn't ext4"));
        }
        let block = 1024usize << u32le(24);
        let incompat = u32le(96);
        let descriptor_size = if incompat & 0x80 != 0 {
            u16::from_le_bytes([sb[254], sb[255]]) as usize
        } else {
            32
        };
        Ok(Ext4 {
            data,
            block,
            inodes_per_group: u32le(40),
            inode_size: u16::from_le_bytes([sb[88], sb[89]]) as usize,
            descriptors: (u32le(20) + 1) * block,
            descriptor_size,
        })
    }

    fn bytes(&self, at: usize, len: usize) -> Result<&'a [u8]> {
        self.data
            .get(at..at + len)
            .ok_or_else(|| fail("an APEX payload is cut short"))
    }

    fn inode(&self, number: usize) -> Result<&'a [u8]> {
        let (group, index) = (
            (number - 1) / self.inodes_per_group,
            (number - 1) % self.inodes_per_group,
        );
        let d = self.bytes(
            self.descriptors + group * self.descriptor_size,
            self.descriptor_size,
        )?;
        let mut table = u32::from_le_bytes(d[8..12].try_into().unwrap()) as usize;
        if self.descriptor_size >= 64 {
            table |= (u32::from_le_bytes(d[40..44].try_into().unwrap()) as usize) << 32;
        }
        self.bytes(
            table * self.block + index * self.inode_size,
            self.inode_size,
        )
    }

    /// The blocks of an extent tree node, as (file block, physical block, count).
    fn extents(&self, node: &[u8], out: &mut Vec<(usize, usize, usize)>) -> Result<()> {
        let u16at = |o: usize| u16::from_le_bytes([node[o], node[o + 1]]) as usize;
        let u32at = |o: usize| u32::from_le_bytes(node[o..o + 4].try_into().unwrap()) as usize;
        if u16at(0) != 0xf30a {
            return Err(fail("a file in an APEX payload has a damaged extent tree"));
        }
        let (entries, depth) = (u16at(2), u16at(6));
        for i in 0..entries {
            let e = 12 + i * 12;
            if depth == 0 {
                let len = u16at(e + 4);
                let start = (u16at(e + 6) << 32) | u32at(e + 8);
                // Uninitialised extents read as zeros.
                let start = if len > 32768 { usize::MAX } else { start };
                out.push((u32at(e), start, if len > 32768 { len - 32768 } else { len }));
            } else {
                let child = (u16at(e + 8) << 32) | u32at(e + 4);
                self.extents(self.bytes(child * self.block, self.block)?, out)?;
            }
        }
        Ok(())
    }

    fn read_inode(&self, number: usize) -> Result<Vec<u8>> {
        let inode = self.inode(number)?;
        let size = u32::from_le_bytes(inode[4..8].try_into().unwrap()) as usize
            | (u32::from_le_bytes(inode[108..112].try_into().unwrap()) as usize) << 32;
        let flags = u32::from_le_bytes(inode[32..36].try_into().unwrap());
        if flags & 0x80000 == 0 {
            return Err(fail("a file in an APEX payload doesn't use extents"));
        }
        let mut extents = Vec::new();
        self.extents(&inode[40..100], &mut extents)?;
        let mut out = vec![0u8; size];
        for (logical, physical, count) in extents {
            if physical == usize::MAX {
                continue;
            }
            let at = logical * self.block;
            if at >= size {
                continue;
            }
            let len = (count * self.block).min(size - at);
            out[at..at + len].copy_from_slice(self.bytes(physical * self.block, len)?);
        }
        Ok(out)
    }

    /// A file's contents, by its path from the root, such as "lib64/libc.so".
    pub(crate) fn read(&self, path: &str) -> Result<Vec<u8>> {
        let mut number = 2;
        for part in path.split('/').filter(|p| !p.is_empty()) {
            let dir = self.read_inode(number)?;
            let mut found = None;
            let mut pos = 0;
            while pos + 8 <= dir.len() {
                let inode = u32::from_le_bytes(dir[pos..pos + 4].try_into().unwrap()) as usize;
                let rec_len = u16::from_le_bytes([dir[pos + 4], dir[pos + 5]]) as usize;
                let name_len = dir[pos + 6] as usize;
                if rec_len < 8 {
                    break;
                }
                if inode != 0 && dir.get(pos + 8..pos + 8 + name_len) == Some(part.as_bytes()) {
                    found = Some(inode);
                    break;
                }
                pos += rec_len;
            }
            number = found.ok_or_else(|| fail(format!("{path} isn't in its APEX payload")))?;
        }
        self.read_inode(number)
    }
}

/// Extracts the parts AAE uses from the downloaded image into `out`
/// (`vendor/…` and `modules/…`).
pub(crate) fn extract(zip: &Path, out: &Path, tools: &Tools) -> Result<()> {
    let work = out.with_extension("part");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).context(|| format!("Creating {}", work.display()))?;
    let result = extract_into(zip, &work, tools);
    let raw = work.join("super.raw");
    let _ = std::fs::remove_file(&raw);
    result?;
    let _ = std::fs::remove_dir_all(out);
    std::fs::rename(&work, out).context(|| format!("Moving {}", out.display()))
}

fn extract_into(zip: &Path, work: &Path, tools: &Tools) -> Result<()> {
    let file = std::fs::File::open(zip).context(|| format!("Opening {}", zip.display()))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| fail(e.to_string()))?;
    let raw = work.join("super.raw");
    unsparse(
        &mut archive
            .by_name("super.img")
            .map_err(|e| fail(e.to_string()))?,
        &raw,
    )?;
    let mut vendor_boot = Vec::new();
    archive
        .by_name("vendor_boot.img")
        .map_err(|e| fail(e.to_string()))?
        .read_to_end(&mut vendor_boot)
        .map_err(|e| fail(e.to_string()))?;

    let mut image = std::fs::File::open(&raw).map_err(|e| fail(e.to_string()))?;
    let size = image.metadata().map_err(|e| fail(e.to_string()))?.len();
    let lp = Super::read(
        &mut image,
        Extent {
            offset: 0,
            len: size,
        },
    )?;
    let (vendor, _) = lp.partition("vendor_a")?;
    let (vendor_dlkm, _) = lp.partition("vendor_dlkm_a")?;
    let from_vendor = |path: &str| tools.erofs_file(&raw, vendor.offset, &format!("/{path}"));

    let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut packages: BTreeMap<&str, Vec<u8>> = BTreeMap::new();
    for (target, source) in &FILES {
        let data = match source {
            From::Vendor(path) => from_vendor(path)?,
            From::Apex(name) => from_vendor(&format!("apex/{name}"))?,
            From::InApex(name, path) => {
                if !packages.contains_key(name) {
                    let package = from_vendor(&format!("apex/{name}"))?;
                    let mut payload = Vec::new();
                    zip::ZipArchive::new(std::io::Cursor::new(package))
                        .and_then(|mut z| {
                            z.by_name("apex_payload.img")?.read_to_end(&mut payload)?;
                            Ok(())
                        })
                        .map_err(|e| fail(format!("{name}: {e}")))?;
                    packages.insert(name, payload);
                }
                Ext4::new(&packages[name])?.read(path)?
            }
        };
        files.insert(format!("vendor/{target}"), data);
    }
    for name in AUDIO {
        files.insert(
            format!("vendor/etc/{name}"),
            from_vendor(&format!("etc/{name}"))?,
        );
    }
    files.insert(
        "modules/system_heap.ko".into(),
        tools.erofs_file(&raw, vendor_dlkm.offset, "/lib/modules/system_heap.ko")?,
    );
    let ramdisk = cpio_read(&lz4_decompress(&first_vendor_ramdisk(&vendor_boot)?)?)?;
    for name in RAMDISK_MODULES {
        let (_, data) = ramdisk
            .get(&format!("lib/modules/{name}"))
            .ok_or_else(|| fail(format!("its ramdisk has no {name}")))?;
        files.insert(format!("modules/{name}"), data.clone());
    }
    for (path, data) in files {
        let target = work.join(&path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).context(|| format!("Creating {}", parent.display()))?;
        }
        std::fs::write(&target, data).context(|| format!("Writing {}", target.display()))?;
    }
    Ok(())
}

/// The first ramdisk of an Android v4 vendor_boot image, still compressed.
fn first_vendor_ramdisk(v: &[u8]) -> Result<Vec<u8>> {
    let u32le = |o: usize| u32::from_le_bytes(v[o..o + 4].try_into().unwrap()) as u64;
    if v.get(..8) != Some(b"VNDRBOOT") || u32le(8) != 4 {
        return Err(fail("its vendor_boot isn't the expected version"));
    }
    let page = u32le(12);
    let align = |n: u64| n.div_ceil(page) * page;
    let base = align(u32le(2096));
    let table = (base + align(u32le(24)) + align(u32le(2100))) as usize;
    let (size, offset) = (u32le(table) as usize, u32le(table + 4) as usize);
    v.get(base as usize + offset..base as usize + offset + size)
        .map(<[u8]>::to_vec)
        .ok_or_else(|| fail("its vendor_boot is cut short"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsparses_raw_fill_and_skip_chunks() {
        let mut img = Vec::new();
        let header = |img: &mut Vec<u8>, blocks: u32, chunks: u32| {
            img.extend_from_slice(&0xed26_ff3au32.to_le_bytes());
            img.extend_from_slice(&1u16.to_le_bytes());
            img.extend_from_slice(&0u16.to_le_bytes());
            img.extend_from_slice(&28u16.to_le_bytes());
            img.extend_from_slice(&12u16.to_le_bytes());
            img.extend_from_slice(&4u32.to_le_bytes());
            img.extend_from_slice(&blocks.to_le_bytes());
            img.extend_from_slice(&chunks.to_le_bytes());
            img.extend_from_slice(&0u32.to_le_bytes());
        };
        let chunk = |img: &mut Vec<u8>, kind: u16, count: u32, body: &[u8]| {
            img.extend_from_slice(&kind.to_le_bytes());
            img.extend_from_slice(&0u16.to_le_bytes());
            img.extend_from_slice(&count.to_le_bytes());
            img.extend_from_slice(&(12 + body.len() as u32).to_le_bytes());
            img.extend_from_slice(body);
        };
        header(&mut img, 4, 3);
        chunk(&mut img, 0xcac1, 1, b"abcd");
        chunk(&mut img, 0xcac2, 2, b"xy12");
        chunk(&mut img, 0xcac3, 1, b"");
        let dir = std::env::temp_dir().join(format!("aae-unsparse-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("raw");
        unsparse(&mut img.as_slice(), &out).unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), b"abcdxy12xy12\0\0\0\0");
        let _ = std::fs::remove_dir_all(dir);
    }
}
