//! Assembles a Googlebook device's disk from Google's recovery image.
//!
//! The recovery image is Google's, unmodified, for a Dell Googlebook. Most of
//! it boots as it is. What changes is in a copy of it, never in the
//! original: the vendor partition gets virtual-device replacements for
//! hardware the VM doesn't have (from AAE's component bundle) and AAE's adb
//! setup; the ramdisk gets the virtio kernel modules; userdata is given room.
//!
//! Adapted from gbos-vm's image scripts (MIT licence, see googlebook/NOTICE.md).

use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use sha2::{Digest, Sha256};

use super::vendor;
use crate::error::{Error, IoContext, Result};

const SECTOR: u64 = 512;

fn fail(what: impl Into<String>) -> Error {
    Error::Vm(format!(
        "The Googlebook image can't be assembled: {}",
        what.into()
    ))
}

fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn u64le(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}
fn align(n: u64, to: u64) -> u64 {
    n.div_ceil(to) * to
}

/// The command-line tools from AAE's component bundle.
pub(crate) struct Tools {
    pub dir: PathBuf,
}

impl Tools {
    fn run(&self, tool: &str, args: &[&str], input: Option<&[u8]>) -> Result<Vec<u8>> {
        let path = self.dir.join(tool);
        let mut child = Command::new(&path)
            .args(args)
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context(|| format!("Running {}", path.display()))?;
        if let Some(input) = input {
            let mut stdin = child.stdin.take().expect("piped");
            let input = input.to_vec();
            std::thread::spawn(move || {
                let _ = stdin.write_all(&input);
            });
        }
        let out = child
            .wait_with_output()
            .context(|| format!("Running {}", path.display()))?;
        if !out.status.success() {
            return Err(fail(format!(
                "{tool} {}: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        Ok(out.stdout)
    }

    /// A file from an EROFS filesystem at `offset` in `image`.
    pub(crate) fn erofs_file(&self, image: &Path, offset: u64, path: &str) -> Result<Vec<u8>> {
        self.run(
            "dump.erofs",
            &[
                &format!("--offset={offset}"),
                &format!("--path={path}"),
                "--cat",
                &image.to_string_lossy(),
            ],
            None,
        )
    }

    /// An inode's number in an EROFS filesystem.
    fn erofs_nid(&self, image: &Path, offset: u64, path: &str) -> Result<u64> {
        let text = self.run(
            "dump.erofs",
            &[
                &format!("--offset={offset}"),
                &format!("--path={path}"),
                &image.to_string_lossy(),
            ],
            None,
        )?;
        let text = String::from_utf8_lossy(&text);
        text.split("NID: ")
            .nth(1)
            .and_then(|r| r.split_whitespace().next())
            .and_then(|n| n.parse().ok())
            .ok_or_else(|| fail(format!("no inode number for {path}")))
    }
}

/// Unpacks lz4 data: the legacy format the kernel uses for ramdisks, or the
/// frame format.
pub(crate) fn lz4_decompress(data: &[u8]) -> Result<Vec<u8>> {
    const LEGACY: u32 = 0x184c_2102;
    if data.get(..4) == Some(&0x184d_2204u32.to_le_bytes()) {
        let mut out = Vec::new();
        lz4_flex::frame::FrameDecoder::new(data)
            .read_to_end(&mut out)
            .map_err(|e| fail(format!("a compressed part is damaged: {e}")))?;
        return Ok(out);
    }
    let mut out = Vec::new();
    let mut pos = 0;
    let mut block = vec![0u8; 8 << 20];
    while pos + 4 <= data.len() {
        let word = u32le(data, pos);
        pos += 4;
        if word == LEGACY {
            continue;
        }
        let len = word as usize;
        let chunk = data
            .get(pos..pos + len)
            .ok_or_else(|| fail("a compressed part is cut short"))?;
        let n = lz4_flex::block::decompress_into(chunk, &mut block)
            .map_err(|e| fail(format!("a compressed part is damaged: {e}")))?;
        out.extend_from_slice(&block[..n]);
        pos += len;
    }
    Ok(out)
}

/// Compresses in lz4's legacy format, which the kernel unpacks ramdisks from.
fn lz4_legacy(data: &[u8]) -> Vec<u8> {
    let mut out = 0x184c_2102u32.to_le_bytes().to_vec();
    for chunk in data.chunks(8 << 20) {
        let compressed = lz4_flex::block::compress(chunk);
        out.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
        out.extend_from_slice(&compressed);
    }
    out
}

fn gunzip(data: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    flate2::read::MultiGzDecoder::new(data)
        .read_to_end(&mut out)
        .map_err(|e| fail(format!("a compressed part is damaged: {e}")))?;
    Ok(out)
}

/// A partition: its byte offset and length.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Extent {
    pub offset: u64,
    pub len: u64,
}

/// The partitions in the image's GPT, checked against their checksums.
fn gpt(file: &mut std::fs::File) -> Result<BTreeMap<String, Extent>> {
    let mut header = [0u8; 512];
    file.seek(SeekFrom::Start(SECTOR)).map_err(io)?;
    file.read_exact(&mut header).map_err(io)?;
    if &header[..8] != b"EFI PART" {
        return Err(fail("it has no GPT"));
    }
    let size = u32le(&header, 12) as usize;
    let mut copy = header[..size].to_vec();
    copy[16..20].fill(0);
    if crc32fast::hash(&copy) != u32le(&header, 16) {
        return Err(fail("its GPT header is damaged"));
    }
    let (lba, count, entry) = (u64le(&header, 72), u32le(&header, 80), u32le(&header, 84));
    let mut entries = vec![0u8; (count * entry) as usize];
    file.seek(SeekFrom::Start(lba * SECTOR)).map_err(io)?;
    file.read_exact(&mut entries).map_err(io)?;
    if crc32fast::hash(&entries) != u32le(&header, 88) {
        return Err(fail("its partition table is damaged"));
    }
    let mut parts = BTreeMap::new();
    for e in entries.chunks(entry as usize) {
        if e[..16].iter().all(|&b| b == 0) {
            continue;
        }
        let name: Vec<u16> = e[56..128]
            .chunks(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .take_while(|&c| c != 0)
            .collect();
        let (first, last) = (u64le(e, 32), u64le(e, 40));
        parts.insert(
            String::from_utf16_lossy(&name),
            Extent {
                offset: first * SECTOR,
                len: (last + 1 - first) * SECTOR,
            },
        );
    }
    Ok(parts)
}

fn io(e: std::io::Error) -> Error {
    fail(e.to_string())
}

fn read_at(file: &mut std::fs::File, offset: u64, len: u64) -> Result<Vec<u8>> {
    let mut buf = vec![0u8; len as usize];
    file.seek(SeekFrom::Start(offset)).map_err(io)?;
    file.read_exact(&mut buf).map_err(io)?;
    Ok(buf)
}

/// One copy of the super partition's logical-partition metadata.
struct LpSlot {
    position: u64,
    data: Vec<u8>,
    header_size: usize,
}

/// The super partition's metadata: where each logical partition is, and
/// every copy of the metadata (each slot and its backup).
pub(crate) struct Super {
    base: Extent,
    slots: Vec<LpSlot>,
    block_size: u64,
}

impl Super {
    pub(crate) fn read(file: &mut std::fs::File, base: Extent) -> Result<Super> {
        let geometry = read_at(file, base.offset + 4096, 4096)?;
        let mut check = geometry[..u32le(&geometry, 4) as usize].to_vec();
        check[8..40].fill(0);
        if Sha256::digest(&check)[..] != geometry[8..40] {
            return Err(fail("the super partition's geometry is damaged"));
        }
        let (size, count, block_size) = (
            u32le(&geometry, 40) as u64,
            u32le(&geometry, 44) as u64,
            u32le(&geometry, 48) as u64,
        );
        let mut slots = Vec::new();
        for slot in 0..count * 2 {
            let position = base.offset + 12288 + slot * size;
            let data = read_at(file, position, size)?;
            if u32le(&data, 0) != 0x414c_5030 {
                return Err(fail("the super partition's metadata is damaged"));
            }
            let header_size = u32le(&data, 8) as usize;
            let mut header = data[..header_size].to_vec();
            header[12..44].fill(0);
            let tables = &data[header_size..header_size + u32le(&data, 44) as usize];
            if Sha256::digest(&header)[..] != data[12..44]
                || Sha256::digest(tables)[..] != data[48..80]
            {
                return Err(fail("the super partition's metadata is damaged"));
            }
            slots.push(LpSlot {
                position,
                data,
                header_size,
            });
        }
        Ok(Super {
            base,
            slots,
            block_size,
        })
    }

    /// (offset of first entry, count, entry size) of one of the metadata's tables.
    fn table(slot: &LpSlot, at: usize) -> (usize, usize, usize) {
        let d = &slot.data;
        (
            slot.header_size + u32le(d, at) as usize,
            u32le(d, at + 4) as usize,
            u32le(d, at + 8) as usize,
        )
    }

    /// A single-extent logical partition: its extent, and where its extent
    /// entry is in each metadata copy.
    pub(crate) fn partition(&self, name: &str) -> Result<(Extent, Vec<usize>)> {
        let mut found = None;
        let mut entries = Vec::new();
        for slot in &self.slots {
            let (po, pc, ps) = Self::table(slot, 80);
            let (eo, _, es) = Self::table(slot, 92);
            for i in 0..pc {
                let p = po + i * ps;
                let pname = slot.data[p..p + 36]
                    .split(|&b| b == 0)
                    .next()
                    .unwrap_or(&[]);
                if pname != name.as_bytes() {
                    continue;
                }
                let (first, count) = (
                    u32le(&slot.data, p + 40) as usize,
                    u32le(&slot.data, p + 44),
                );
                if count != 1 {
                    return Err(fail(format!("{name} isn't in one piece")));
                }
                let e = eo + first * es;
                let sectors = u64le(&slot.data, e);
                let target = u64le(&slot.data, e + 12);
                found = Some(Extent {
                    offset: self.base.offset + target * SECTOR,
                    len: sectors * SECTOR,
                });
                entries.push(e);
            }
        }
        found
            .filter(|_| entries.len() == self.slots.len())
            .map(|f| (f, entries))
            .ok_or_else(|| fail(format!("it has no {name} partition")))
    }

    /// Every byte range in use by some logical partition.
    fn used(&self) -> Vec<(u64, u64)> {
        let mut ranges = Vec::new();
        for slot in &self.slots {
            let (eo, ec, es) = Self::table(slot, 92);
            for i in 0..ec {
                let e = eo + i * es;
                let (sectors, target) = (u64le(&slot.data, e), u64le(&slot.data, e + 12));
                if sectors > 0 {
                    let start = self.base.offset + target * SECTOR;
                    ranges.push((start, start + sectors * SECTOR));
                }
            }
        }
        ranges
    }

    /// Moves a partition to free space at the end of super, for when its new
    /// contents don't fit where it was. Copies the data first, then rewrites
    /// every metadata copy.
    fn relocate(
        &mut self,
        file: &mut std::fs::File,
        name: &str,
        contents: &Path,
    ) -> Result<Extent> {
        let (_, entries) = self.partition(name)?;
        let used = self.used();
        let mib = 1 << 20;
        let start = align(used.iter().map(|r| r.1).max().unwrap_or(0), mib);
        let size = std::fs::metadata(contents).map_err(io)?.len();
        let len = align(size, self.block_size);
        if start + len > self.base.offset + self.base.len
            || used.iter().any(|&(s, e)| start < e && start + len > s)
        {
            return Err(fail(format!("there's no room in super for the new {name}")));
        }
        file.seek(SeekFrom::Start(start)).map_err(io)?;
        std::io::copy(&mut std::fs::File::open(contents).map_err(io)?, file).map_err(io)?;
        file.write_all(&vec![0u8; (len - size) as usize])
            .map_err(io)?;
        for (slot, e) in self.slots.iter_mut().zip(entries) {
            let d = &mut slot.data;
            d[e..e + 8].copy_from_slice(&(len / SECTOR).to_le_bytes());
            d[e + 8..e + 12].copy_from_slice(&0u32.to_le_bytes());
            d[e + 12..e + 20].copy_from_slice(&((start - self.base.offset) / SECTOR).to_le_bytes());
            d[e + 20..e + 24].copy_from_slice(&0u32.to_le_bytes());
            let tables_len = u32le(d, 44) as usize;
            let tables = Sha256::digest(&d[slot.header_size..slot.header_size + tables_len]);
            d[48..80].copy_from_slice(&tables);
            d[12..44].fill(0);
            let header = Sha256::digest(&d[..slot.header_size]);
            d[12..44].copy_from_slice(&header);
            file.seek(SeekFrom::Start(slot.position)).map_err(io)?;
            file.write_all(d).map_err(io)?;
        }
        Ok(Extent { offset: start, len })
    }
}

/// Reads a newc cpio archive.
pub(crate) fn cpio_read(data: &[u8]) -> Result<BTreeMap<String, (u32, Vec<u8>)>> {
    let mut out = BTreeMap::new();
    let mut pos = 0usize;
    let hex = |b: &[u8]| u32::from_str_radix(std::str::from_utf8(b).unwrap_or("x"), 16).ok();
    while pos + 110 <= data.len() {
        while pos < data.len() && data[pos] == 0 {
            pos += 1;
        }
        if pos + 110 > data.len() {
            break;
        }
        if &data[pos..pos + 6] != b"070701" && &data[pos..pos + 6] != b"070702" {
            return Err(fail("a ramdisk is damaged"));
        }
        let field = |i: usize| hex(&data[pos + 6 + i * 8..pos + 14 + i * 8]);
        let (mode, size, namesize) = match (field(1), field(6), field(11)) {
            (Some(m), Some(s), Some(n)) => (m, s as usize, n as usize),
            _ => return Err(fail("a ramdisk is damaged")),
        };
        let name = String::from_utf8_lossy(&data[pos + 110..pos + 110 + namesize - 1]).into_owned();
        let start = (pos + 110 + namesize + 3) & !3;
        let blob = data
            .get(start..start + size)
            .ok_or_else(|| fail("a ramdisk is cut short"))?;
        pos = (start + size + 3) & !3;
        if name != "TRAILER!!!" {
            out.insert(name, (mode, blob.to_vec()));
        }
    }
    Ok(out)
}

/// Writes a newc cpio archive.
fn cpio_write(entries: &[(String, u32, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    let trailer = ("TRAILER!!!".to_string(), 0o100000u32, Vec::new());
    for (i, (name, mode, data)) in entries.iter().chain(std::iter::once(&trailer)).enumerate() {
        let mut header = String::from("070701");
        let name_len = name.len() + 1;
        for value in [
            300000 + i as u32,
            *mode,
            0,
            0,
            1,
            0,
            data.len() as u32,
            0,
            0,
            0,
            0,
            name_len as u32,
            0,
        ] {
            header.push_str(&format!("{value:08x}"));
        }
        out.extend_from_slice(header.as_bytes());
        out.extend_from_slice(name.as_bytes());
        out.push(0);
        while out.len() % 4 != 0 {
            out.push(0);
        }
        out.extend_from_slice(data);
        while out.len() % 4 != 0 {
            out.push(0);
        }
    }
    out
}

/// What Android's v4 boot images hold that the VM boots with.
struct Boot {
    kernel: Vec<u8>,
    /// The first vendor ramdisk ("platform") and the generic ramdisk, unpacked.
    vendor_ramdisk: Vec<u8>,
    init_ramdisk: Vec<u8>,
}

fn unpack(data: &[u8]) -> Result<Vec<u8>> {
    if data.starts_with(&[0x1f, 0x8b]) {
        gunzip(data)
    } else {
        lz4_decompress(data)
    }
}

fn read_boot(file: &mut std::fs::File, parts: &BTreeMap<String, Extent>) -> Result<Boot> {
    let part = |name: &str| {
        parts
            .get(name)
            .copied()
            .ok_or_else(|| fail(format!("it has no {name}")))
    };
    let boot = {
        let e = part("boot_a")?;
        read_at(file, e.offset, e.len)?
    };
    let init = {
        let e = part("init_boot_a")?;
        read_at(file, e.offset, e.len)?
    };
    let v = {
        let e = part("vendor_boot_a")?;
        read_at(file, e.offset, e.len)?
    };
    if &boot[..8] != b"ANDROID!"
        || &init[..8] != b"ANDROID!"
        || u32le(&boot, 40) != 4
        || u32le(&init, 40) != 4
    {
        return Err(fail("its boot images aren't the expected version"));
    }
    let payload = &boot[4096..4096 + u32le(&boot, 8) as usize];
    let kernel = if payload.starts_with(&[0x1f, 0x8b]) {
        gunzip(payload)?
    } else if payload.starts_with(&[0x02, 0x21, 0x4c, 0x18]) {
        lz4_decompress(payload)?
    } else {
        payload.to_vec()
    };
    if kernel.get(56..60) != Some(b"ARM\x64") {
        return Err(fail("its kernel isn't an arm64 kernel"));
    }
    let start = 4096 + align(u32le(&init, 8) as u64, 4096) as usize;
    let init_ramdisk = unpack(&init[start..start + u32le(&init, 12) as usize])?;
    if &v[..8] != b"VNDRBOOT" || u32le(&v, 8) != 4 {
        return Err(fail("its vendor boot image isn't the expected version"));
    }
    let page = u32le(&v, 12) as u64;
    let (ramdisks_size, header_size, dtb_size) = (
        u32le(&v, 24) as u64,
        u32le(&v, 2096) as u64,
        u32le(&v, 2100) as u64,
    );
    let base = align(header_size, page);
    let table = (base + align(ramdisks_size, page) + align(dtb_size, page)) as usize;
    let (size, offset) = (u32le(&v, table) as usize, u32le(&v, table + 4) as usize);
    let vendor_ramdisk = unpack(&v[base as usize + offset..base as usize + offset + size])?;
    Ok(Boot {
        kernel,
        vendor_ramdisk,
        init_ramdisk,
    })
}

/// Kernel modules the VM loads from the ramdisk, in order. The virtio PCI
/// ones are the image's own, signed for its kernel; the rest are from
/// Cuttlefish, built for the same kernel.
const IMAGE_MODULES: [&str; 3] = [
    "virtio_pci_legacy_dev.ko",
    "virtio_pci_modern_dev.ko",
    "virtio_pci.ko",
];
const BUNDLE_MODULES: [&str; 5] = [
    "virtio_dma_buf.ko",
    "virtio-gpu.ko",
    "virtio_input.ko",
    "virtio_blk.ko",
    "system_heap.ko",
];

fn make_initrd(
    tools: &Tools,
    image: &Path,
    boot: &Boot,
    system_dlkm: u64,
    modules: &Path,
) -> Result<Vec<u8>> {
    let mut extra: Vec<(String, u32, Vec<u8>)> = vec![
        ("lib".into(), 0o40755, Vec::new()),
        ("lib/modules".into(), 0o40755, Vec::new()),
    ];
    for name in IMAGE_MODULES {
        let data = tools.erofs_file(image, system_dlkm, &format!("/lib/modules/{name}"))?;
        if !data.starts_with(b"\x7fELF") {
            return Err(fail(format!("{name} isn't a kernel module")));
        }
        extra.push((format!("lib/modules/{name}"), 0o100644, data));
    }
    let bundle = |name: &str| {
        std::fs::read(modules.join(name))
            .context(|| format!("Reading {name} from AAE's Googlebook components"))
    };
    for name in BUNDLE_MODULES {
        extra.push((format!("lib/modules/{name}"), 0o100644, bundle(name)?));
    }
    for name in ["modules.alias", "modules.softdep", "modules.options"] {
        extra.push((format!("lib/modules/{name}"), 0o100644, bundle(name)?));
    }
    let mut dep = bundle("modules.dep")?;
    dep.extend_from_slice(b"\n/lib/modules/virtio_blk.ko:\n\n/lib/modules/system_heap.ko:\n");
    extra.push(("lib/modules/modules.dep".into(), 0o100644, dep));
    let load: String = IMAGE_MODULES
        .iter()
        .chain(BUNDLE_MODULES.iter())
        .map(|m| format!("{m}\n"))
        .collect();
    extra.push((
        "lib/modules/modules.load".into(),
        0o100644,
        load.clone().into_bytes(),
    ));
    extra.push((
        "lib/modules/modules.load.recovery".into(),
        0o100644,
        load.into_bytes(),
    ));

    // The vendor partition is changed, so its verified-boot check goes.
    let platform = cpio_read(&boot.vendor_ramdisk)?;
    let fstab_name = "first_stage_ramdisk/fstab.android-desktop";
    let (mode, fstab) = platform
        .get(fstab_name)
        .cloned()
        .ok_or_else(|| fail("its ramdisk has no fstab"))?;
    let fstab: String = String::from_utf8_lossy(&fstab)
        .lines()
        .map(|l| {
            if l.starts_with("vendor ") {
                l.replace(",avb=vbmeta", "")
            } else {
                l.to_string()
            }
        })
        .map(|l| l + "\n")
        .collect();
    extra.push((fstab_name.into(), mode, fstab.into_bytes()));

    let mut ramdisk = boot.vendor_ramdisk.clone();
    ramdisk.extend_from_slice(&boot.init_ramdisk);
    ramdisk.extend_from_slice(&cpio_write(&extra));
    Ok(lz4_legacy(&ramdisk))
}

/// An EROFS inode's mode, owner, time and short-name extended attributes.
pub(crate) struct Metadata {
    pub mode: u16,
    pub uid: u32,
    pub gid: u32,
    pub mtime: u64,
    pub xattrs: Vec<(String, Vec<u8>)>,
}

fn erofs_metadata(tools: &Tools, image: &Path, offset: u64, path: &str) -> Result<Metadata> {
    let nid = tools.erofs_nid(image, offset, path)?;
    let mut f = std::fs::File::open(image).map_err(io)?;
    let sb = read_at(&mut f, offset + 1024, 128)?;
    let block = 1u64 << sb[12];
    let (meta, xbase) = (u32le(&sb, 40) as u64, u32le(&sb, 44) as u64);
    let epoch = u64le(&sb, 24);
    let pos = offset + meta * block + nid * 32;
    let inode = read_at(&mut f, pos, 64)?;
    let format = u16::from_le_bytes([inode[0], inode[1]]);
    let xattr_count = u16::from_le_bytes([inode[2], inode[3]]) as u64;
    let mode = u16::from_le_bytes([inode[4], inode[5]]);
    let extended = format & 1 == 1;
    let (uid, gid) = if extended {
        (u32le(&inode, 24), u32le(&inode, 28))
    } else {
        (
            u16::from_le_bytes([inode[24], inode[25]]) as u32,
            u16::from_le_bytes([inode[26], inode[27]]) as u32,
        )
    };
    let mtime = if extended {
        u64le(&inode, 32)
    } else {
        epoch + u32le(&inode, 12) as u64
    };
    let mut xattrs = Vec::new();
    let mut entry = |f: &mut std::fs::File, at: u64| -> Result<u64> {
        let head = read_at(f, at, 4)?;
        let (name_len, index, value_len) = (
            head[0] as u64,
            head[1],
            u16::from_le_bytes([head[2], head[3]]) as u64,
        );
        let body = read_at(f, at + 4, name_len + value_len)?;
        let prefix = match index {
            1 => "user.",
            4 => "trusted.",
            6 => "security.",
            _ => return Err(fail("an unexpected file attribute")),
        };
        xattrs.push((
            format!(
                "{prefix}{}",
                String::from_utf8_lossy(&body[..name_len as usize])
            ),
            body[name_len as usize..].to_vec(),
        ));
        Ok((4 + name_len + value_len + 3) & !3)
    };
    if xattr_count > 0 {
        let mut p = pos + if extended { 64 } else { 32 };
        let head = read_at(&mut f, p, 12)?;
        let shared = head[4] as u64;
        for j in 0..shared {
            let id = u32le(&read_at(&mut f, p + 12 + j * 4, 4)?, 0) as u64;
            entry(&mut f, offset + xbase * block + 4 * id)?;
        }
        let end = p + 12 + (xattr_count - 1) * 4;
        p += 12 + shared * 4;
        while p < end {
            p += entry(&mut f, p)?;
        }
    }
    Ok(Metadata {
        mode,
        uid,
        gid,
        mtime,
        xattrs,
    })
}

/// A PAX tar archive with SELinux labels, for mkfs.erofs to lay over the
/// vendor partition.
struct Tar {
    out: Vec<u8>,
}

impl Tar {
    #[allow(clippy::too_many_arguments)]
    fn header(
        &mut self,
        name: &str,
        size: u64,
        mode: u32,
        uid: u32,
        gid: u32,
        mtime: u64,
        kind: u8,
    ) {
        let mut h = [0u8; 512];
        h[..name.len()].copy_from_slice(name.as_bytes());
        let octal = |h: &mut [u8; 512], at: usize, len: usize, value: u64| {
            let s = format!("{value:0w$o}\0", w = len - 1);
            h[at..at + len].copy_from_slice(s.as_bytes());
        };
        octal(&mut h, 100, 8, mode as u64);
        octal(&mut h, 108, 8, uid as u64);
        octal(&mut h, 116, 8, gid as u64);
        octal(&mut h, 124, 12, size);
        octal(&mut h, 136, 12, mtime);
        h[148..156].fill(b' ');
        h[156] = kind;
        h[257..263].copy_from_slice(b"ustar\0");
        h[263..265].copy_from_slice(b"00");
        let sum: u32 = h.iter().map(|&b| b as u32).sum();
        let s = format!("{sum:06o}\0 ");
        h[148..156].copy_from_slice(s.as_bytes());
        self.out.extend_from_slice(&h);
    }

    fn data(&mut self, data: &[u8]) {
        self.out.extend_from_slice(data);
        let pad = (512 - data.len() % 512) % 512;
        self.out.extend(std::iter::repeat_n(0u8, pad));
    }

    /// PAX extended attributes for the next entry.
    fn pax(&mut self, name: &str, records: &[(String, Vec<u8>)]) {
        if records.is_empty() {
            return;
        }
        let mut body = Vec::new();
        for (key, value) in records {
            // "<len> <key>=<value>\n", where len counts itself.
            let rest = key.len() + value.len() + 3;
            let mut len = rest + 1;
            while len.to_string().len() + rest > len {
                len += 1;
            }
            body.extend_from_slice(format!("{len} {key}=").as_bytes());
            body.extend_from_slice(value);
            body.push(b'\n');
        }
        let pax_name = format!("PaxHeader/{}", name.trim_end_matches('/'));
        self.header(
            &pax_name[..pax_name.len().min(99)],
            body.len() as u64,
            0o644,
            0,
            0,
            0,
            b'x',
        );
        self.data(&body);
    }

    fn dir(&mut self, name: &str, m: &Metadata) {
        let xattrs: Vec<(String, Vec<u8>)> = m
            .xattrs
            .iter()
            .map(|(k, v)| (format!("SCHILY.xattr.{k}"), v.clone()))
            .collect();
        self.pax(name, &xattrs);
        self.header(
            name,
            0,
            (m.mode & 0o7777) as u32,
            m.uid,
            m.gid,
            m.mtime,
            b'5',
        );
    }

    fn file(&mut self, name: &str, data: &[u8], label: &str, mode: u32) {
        self.pax(
            name,
            &[(
                "SCHILY.xattr.security.selinux".into(),
                format!("u:object_r:{label}:s0").into_bytes(),
            )],
        );
        self.header(name, data.len() as u64, mode, 0, 0, vendor::MTIME, b'0');
        self.data(data);
    }

    fn finish(mut self) -> Vec<u8> {
        self.out.extend(std::iter::repeat_n(0u8, 1024));
        self.out
    }
}

/// CRC32C as EROFS uses it for its superblock: seeded with all ones, not inverted.
fn erofs_crc(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = (crc >> 1) ^ if crc & 1 == 1 { 0x82f6_3b78 } else { 0 };
        }
    }
    crc
}

/// Builds the new vendor filesystem: the original with the overlay laid
/// over it. Returns its path.
fn make_vendor(
    tools: &Tools,
    image: &Path,
    vendor_part: Extent,
    sources: &[&Path],
    work: &Path,
) -> Result<PathBuf> {
    let original = |path: &str| tools.erofs_file(image, vendor_part.offset, path);
    let files = vendor::overlay(&original, sources, &|path| {
        // sepolicy-allow adds AAE's rules to the original policy.
        let input = work.join("original-policy");
        let output = work.join("policy");
        std::fs::write(&input, path).map_err(io)?;
        let mut args = vec![
            input.to_string_lossy().into_owned(),
            output.to_string_lossy().into_owned(),
        ];
        args.extend(vendor::POLICY_RULES.iter().map(|r| r.to_string()));
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        tools.run("sepolicy-allow", &args, None)?;
        std::fs::read(&output).map_err(io)
    })?;
    let mut tar = Tar { out: Vec::new() };
    // The root directory's inode can't grow, so its attributes are left out
    // here and its original reference to them restored afterwards.
    let mut root_meta = erofs_metadata(tools, image, vendor_part.offset, "/")?;
    root_meta.xattrs.clear();
    tar.dir("./", &root_meta);
    for dir in vendor::DIRS {
        let meta = match dir {
            "lib64/vm_keymint" => Metadata {
                mode: 0o40755,
                uid: 0,
                gid: 2000,
                mtime: vendor::MTIME,
                xattrs: vec![(
                    "security.selinux".into(),
                    b"u:object_r:vendor_file:s0".to_vec(),
                )],
            },
            _ => erofs_metadata(tools, image, vendor_part.offset, &format!("/{dir}"))?,
        };
        tar.dir(&format!("{dir}/"), &meta);
    }
    for f in &files {
        tar.file(&f.path, &f.data, &f.label, f.mode);
    }
    let overlay = work.join("overlay.tar");
    std::fs::write(&overlay, tar.finish()).map_err(io)?;

    // The original vendor filesystem, then the overlay imported into it.
    let vendor = work.join("vendor.erofs");
    let mut source = std::fs::File::open(image).map_err(io)?;
    let sb = read_at(&mut source, vendor_part.offset + 1024, 128)?;
    if u32le(&sb, 0) != 0xe0f5_e1e2 {
        return Err(fail("its vendor partition isn't EROFS"));
    }
    let size = u32le(&sb, 36) as u64 * 4096;
    {
        let mut out = std::fs::File::create(&vendor).map_err(io)?;
        source
            .seek(SeekFrom::Start(vendor_part.offset))
            .map_err(io)?;
        std::io::copy(&mut (&mut source).take(size), &mut out).map_err(io)?;
    }
    let vendor_s = vendor.to_string_lossy().into_owned();
    tools.run(
        "mkfs.erofs",
        &[
            "--incremental=data",
            "--tar=f",
            "-x0",
            "-b4096",
            &format!("-T{}", vendor::MTIME),
            "-zlz4hc",
            &vendor_s,
            &overlay.to_string_lossy(),
        ],
        None,
    )?;
    // The import clears the root directory's attributes and can't grow its
    // inode, so the original's shared-attribute reference is put back; then
    // the superblock's checksum, which covers the root inode, is recomputed.
    let root =
        |sb: &[u8]| u32le(sb, 40) as u64 * 4096 + u16::from_le_bytes([sb[14], sb[15]]) as u64 * 32;
    let old = read_at(&mut source, vendor_part.offset + root(&sb), 48)?;
    let mut out = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&vendor)
        .map_err(io)?;
    let new_sb = read_at(&mut out, 1024, 128)?;
    let at = root(&new_sb);
    let new = read_at(&mut out, at, 48)?;
    if old[2..4] != new[2..4] || old[0] & 1 == 1 || new[0] & 1 == 1 {
        return Err(fail(
            "the new vendor root directory isn't laid out as expected",
        ));
    }
    out.seek(SeekFrom::Start(at + 32)).map_err(io)?;
    out.write_all(&old[32..48]).map_err(io)?;
    let mut block = read_at(&mut out, 1024, 3072)?;
    block[4..8].fill(0);
    out.seek(SeekFrom::Start(1028)).map_err(io)?;
    out.write_all(&erofs_crc(&block).to_le_bytes())
        .map_err(io)?;
    drop(out);
    tools.run("fsck.erofs", &[&vendor_s], None)?;
    for f in &files {
        if tools.erofs_file(&vendor, 0, &format!("/{}", f.path))? != f.data {
            return Err(fail(format!(
                "{} didn't arrive intact in the new vendor partition",
                f.path
            )));
        }
    }
    Ok(vendor)
}

/// Moves userdata to fresh, sparse space after the old end of the image and
/// makes it 16 GB; Android formats it on first boot. Moves the backup GPT
/// to the new end.
fn expand_userdata(file: &mut std::fs::File) -> Result<()> {
    let old_size = file.metadata().map_err(io)?.len();
    let mut h = read_at(file, SECTOR, 512)?;
    let hs = u32le(&h, 12) as usize;
    let (entries_lba, n, es) = (
        u64le(&h, 72),
        u32le(&h, 80) as usize,
        u32le(&h, 84) as usize,
    );
    if entries_lba != 2 || n != 128 || es != 128 {
        return Err(fail("its partition table isn't laid out as expected"));
    }
    let mut entries = read_at(file, entries_lba * SECTOR, (n * es) as u64)?;
    let index = (0..n)
        .find(|&i| {
            let name: Vec<u16> = entries[i * es + 56..(i + 1) * es]
                .chunks(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .take_while(|&c| c != 0)
                .collect();
            String::from_utf16_lossy(&name) == "userdata"
        })
        .ok_or_else(|| fail("it has no userdata partition"))?;
    let start = align(old_size, 1 << 20) / SECTOR;
    let end = start + (16u64 << 30) / SECTOR - 1;
    for k in 0..n {
        if k == index || entries[k * es..k * es + 16].iter().all(|&b| b == 0) {
            continue;
        }
        if u64le(&entries, k * es + 40) >= start {
            return Err(fail("a partition lies past the image's end"));
        }
    }
    entries[index * es + 32..index * es + 40].copy_from_slice(&start.to_le_bytes());
    entries[index * es + 40..index * es + 48].copy_from_slice(&end.to_le_bytes());
    let backup_entries = end + 1;
    let last = backup_entries + (n * es) as u64 / SECTOR;
    h[32..40].copy_from_slice(&last.to_le_bytes());
    h[48..56].copy_from_slice(&end.to_le_bytes());
    h[88..92].copy_from_slice(&crc32fast::hash(&entries).to_le_bytes());
    let checksum = |h: &mut Vec<u8>| {
        h[16..20].fill(0);
        let crc = crc32fast::hash(&h[..hs]);
        h[16..20].copy_from_slice(&crc.to_le_bytes());
    };
    checksum(&mut h);
    let mut backup = h.clone();
    backup[24..32].copy_from_slice(&last.to_le_bytes());
    backup[32..40].copy_from_slice(&1u64.to_le_bytes());
    backup[72..80].copy_from_slice(&backup_entries.to_le_bytes());
    checksum(&mut backup);
    file.set_len((last + 1) * SECTOR).map_err(io)?;
    fn write(file: &mut std::fs::File, at: u64, data: &[u8]) -> Result<()> {
        file.seek(SeekFrom::Start(at)).map_err(io)?;
        file.write_all(data).map_err(io)
    }
    write(file, SECTOR, &h)?;
    write(file, 2 * SECTOR, &entries)?;
    write(file, backup_entries * SECTOR, &entries)?;
    write(file, last * SECTOR, &backup)?;
    // The protective MBR spans the disk.
    let mut mbr = read_at(file, 446, 16)?;
    if mbr[4] != 0xee {
        return Err(fail("its protective MBR isn't as expected"));
    }
    mbr[12..16].copy_from_slice(&(last.min(0xffff_ffff) as u32).to_le_bytes());
    write(file, 446, &mbr)?;
    file.flush().map_err(io)
}

/// Assembles `out/googlebook.raw`, `kernel.Image` and `initrd.img` from the
/// recovery image at `recovery`, AAE's components at `components` and the
/// parts extracted from Cuttlefish at `cuttlefish`. The recovery image is
/// read, never written: the disk is a copy-on-write clone.
pub(crate) fn assemble(
    recovery: &Path,
    components: &Path,
    cuttlefish: &Path,
    out: &Path,
    report: &mut dyn FnMut(&str),
) -> Result<()> {
    let tools = Tools {
        dir: components.join("tools"),
    };
    let work = out.join("work");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).context(|| format!("Creating {}", work.display()))?;

    report("Reading Google's image");
    let mut file =
        std::fs::File::open(recovery).context(|| format!("Opening {}", recovery.display()))?;
    let parts = gpt(&mut file)?;
    let super_ext = *parts
        .get("super")
        .ok_or_else(|| fail("it has no super partition"))?;
    let lp = Super::read(&mut file, super_ext)?;
    let (vendor_part, _) = lp.partition("vendor_a")?;
    let (system_dlkm, _) = lp.partition("system_dlkm_a")?;
    let boot = read_boot(&mut file, &parts)?;
    drop(file);

    report("Making the ramdisk");
    let initrd = make_initrd(
        &tools,
        recovery,
        &boot,
        system_dlkm.offset,
        &cuttlefish.join("modules"),
    )?;

    report("Making the vendor partition");
    let vendor = make_vendor(
        &tools,
        recovery,
        vendor_part,
        &[&components.join("guest/vendor"), &cuttlefish.join("vendor")],
        &work,
    )?;

    report("Writing the disk");
    let disk = work.join("googlebook.raw");
    let status = Command::new("/bin/cp")
        .arg("-c")
        .arg(recovery)
        .arg(&disk)
        .status()
        .context(|| "Copying Google's image".to_string())?;
    if !status.success() {
        return Err(fail(
            "the disk couldn't be cloned. It needs an APFS volume.",
        ));
    }
    let mut image = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&disk)
        .map_err(io)?;
    let size = std::fs::metadata(&vendor).map_err(io)?.len();
    if size <= vendor_part.len {
        image
            .seek(SeekFrom::Start(vendor_part.offset))
            .map_err(io)?;
        std::io::copy(&mut std::fs::File::open(&vendor).map_err(io)?, &mut image).map_err(io)?;
        image
            .write_all(&vec![0u8; (vendor_part.len - size) as usize])
            .map_err(io)?;
    } else {
        let mut lp = Super::read(&mut image, super_ext)?;
        lp.relocate(&mut image, "vendor_a", &vendor)?;
    }
    expand_userdata(&mut image)?;
    image.sync_all().map_err(io)?;
    drop(image);

    std::fs::write(work.join("kernel.Image"), &boot.kernel).map_err(io)?;
    std::fs::write(work.join("initrd.img"), &initrd).map_err(io)?;
    for name in ["googlebook.raw", "kernel.Image", "initrd.img"] {
        std::fs::rename(work.join(name), out.join(name)).map_err(io)?;
    }
    let _ = std::fs::remove_dir_all(&work);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpio_round_trips() {
        let entries = vec![
            ("a/b".to_string(), 0o100644, b"hello".to_vec()),
            ("a".to_string(), 0o40755, Vec::new()),
        ];
        let read = cpio_read(&cpio_write(&entries)).unwrap();
        assert_eq!(read["a/b"], (0o100644, b"hello".to_vec()));
        assert_eq!(read["a"].0, 0o40755);
    }

    #[test]
    fn erofs_crc_matches_crc32c_without_inversion() {
        // CRC32C("123456789") is 0xe3069283; EROFS's leaves out the final inversion.
        assert_eq!(!erofs_crc(b"123456789"), 0xe306_9283);
    }

    #[test]
    fn pax_record_lengths_count_themselves() {
        let mut tar = Tar { out: Vec::new() };
        tar.pax("f", &[("k".into(), b"v".to_vec())]);
        let body = &tar.out[512..];
        let text =
            std::str::from_utf8(&body[..body.iter().position(|&b| b == 0).unwrap()]).unwrap();
        let len: usize = text.split(' ').next().unwrap().parse().unwrap();
        assert_eq!(len, text.len());
    }
}
