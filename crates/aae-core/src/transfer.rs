//! Exporting a device to one file, and importing it on another computer
//! running AAE with the same kind of processor.
//!
//! The file is a zip of the device's folder, with `aae-device.json` saying
//! what it is. The emulator's disk overlays name their base images relative
//! to the folder, so it can move. Files the emulator makes while running,
//! and the quick-start snapshot, which is tied to this computer's emulator,
//! are left out; named snapshots go with it.

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::device::{Device, DeviceStore};
use crate::error::{Error, Result};
use crate::sdk::Sdk;

const MANIFEST: &str = "aae-device.json";
const FOLDER: &str = "device/";
/// The ending exported files get.
pub const EXTENSION: &str = "aaedevice";

/// What an exported file holds.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub format: u32,
    pub name: String,
    pub api: u32,
    #[serde(default)]
    pub release: Option<String>,
    pub tag: String,
    pub abi: String,
    pub sysdir: String,
    /// The AAE that made it.
    pub aae: String,
}

/// Files the emulator makes while a device runs, and the quick-start
/// snapshot: not exported.
fn left_out(relative: &str) -> bool {
    let name = relative.rsplit('/').next().unwrap_or(relative);
    relative.starts_with("snapshots/default_boot")
        || relative.starts_with("tmpAdbCmds")
        || name.ends_with(".lock")
        || matches!(
            name,
            "hardware-qemu.ini"
                | "emulator.log"
                | "emu-launch-params.txt"
                | "bootcompleted.ini"
                | "read-snapshot.txt"
                | "emulator-user.ini"
                | "version_num.cache"
        )
}

fn files(dir: &Path) -> Result<Vec<(PathBuf, String, u64)>> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(folder) = stack.pop() {
        let entries = std::fs::read_dir(&folder)
            .map_err(|e| Error::Message(format!("{} can't be read: {e}", folder.display())))?;
        for entry in entries.flatten() {
            let path = entry.path();
            let relative = path
                .strip_prefix(dir)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            if left_out(&relative) {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                stack.push(path);
            } else if meta.is_file() {
                out.push((path, relative, meta.len()));
            }
        }
    }
    out.sort_by(|a, b| a.1.cmp(&b.1));
    Ok(out)
}

/// The path with the export ending, if it hasn't one.
pub fn file_for(path: &Path) -> PathBuf {
    match path.extension().and_then(|e| e.to_str()) {
        Some(e) if e.eq_ignore_ascii_case(EXTENSION) => path.to_path_buf(),
        _ => {
            let mut name = path.as_os_str().to_owned();
            name.push(format!(".{EXTENSION}"));
            PathBuf::from(name)
        }
    }
}

/// Exports a stopped device to `path` (see [`file_for`]). `progress` gets
/// bytes done and total. Returns the file written.
pub fn export(device: &Device, path: &Path, mut progress: impl FnMut(u64, u64)) -> Result<PathBuf> {
    use zip::write::SimpleFileOptions;
    if device.runtime().is_some() {
        return Err(Error::MustStop(device.meta.name.clone(), "exported"));
    }
    let path = file_for(path);
    let list = files(&device.dir)?;
    let total: u64 = list.iter().map(|(_, _, size)| size).sum();
    let partial = path.with_extension(format!("{EXTENSION}.part"));
    let file = std::fs::File::create(&partial)
        .map_err(|e| Error::Message(format!("{} can't be written: {e}", partial.display())))?;
    let mut zip = zip::ZipWriter::new(std::io::BufWriter::new(file));
    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .compression_level(Some(1))
        .large_file(true);
    let failed = |e: &dyn std::fmt::Display| Error::Message(format!("Exporting failed: {e}"));
    let manifest = Manifest {
        format: 1,
        name: device.meta.name.clone(),
        api: device.meta.api,
        release: device.meta.release.clone(),
        tag: device.meta.tag.clone(),
        abi: device.meta.abi.clone(),
        sysdir: device.meta.sysdir.clone(),
        aae: env!("CARGO_PKG_VERSION").into(),
    };
    zip.start_file(MANIFEST, options).map_err(|e| failed(&e))?;
    zip.write_all(&serde_json::to_vec_pretty(&manifest).map_err(|e| failed(&e))?)
        .map_err(|e| failed(&e))?;
    let mut done = 0u64;
    let mut buffer = vec![0u8; 1 << 20];
    for (source, relative, _) in &list {
        zip.start_file(format!("{FOLDER}{relative}"), options)
            .map_err(|e| failed(&e))?;
        let mut input = std::fs::File::open(source).map_err(|e| failed(&e))?;
        loop {
            let read = input.read(&mut buffer).map_err(|e| failed(&e))?;
            if read == 0 {
                break;
            }
            zip.write_all(&buffer[..read]).map_err(|e| failed(&e))?;
            done += read as u64;
            progress(done, total);
        }
    }
    zip.finish().map_err(|e| failed(&e))?;
    std::fs::rename(&partial, &path).map_err(|e| failed(&e))?;
    Ok(path)
}

/// Reads an exported file's manifest.
pub fn manifest(path: &Path) -> Result<Manifest> {
    let file = std::fs::File::open(path)
        .map_err(|e| Error::Message(format!("{} can't be read: {e}", path.display())))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|_| not_export(path))?;
    let mut entry = zip.by_name(MANIFEST).map_err(|_| not_export(path))?;
    let mut text = String::new();
    entry
        .read_to_string(&mut text)
        .map_err(|_| not_export(path))?;
    serde_json::from_str(&text).map_err(|_| not_export(path))
}

fn not_export(path: &Path) -> Error {
    Error::Message(format!(
        "{} isn't a device exported from AAE.",
        path.display()
    ))
}

/// Imports an exported device as `name`, or its original name (numbered if
/// taken). Its Android version must be installed and match this computer's
/// architecture. `progress` gets bytes done and total.
pub fn import(
    sdk: &Sdk,
    store: &DeviceStore,
    path: &Path,
    name: Option<&str>,
    mut progress: impl FnMut(u64, u64),
) -> Result<Device> {
    let manifest = manifest(path)?;
    let host_arm = cfg!(target_arch = "aarch64");
    if manifest.abi.starts_with("arm64") != host_arm {
        return Err(Error::Message(format!(
            "{} was made on a computer with {} processor, and this one has {}. Devices only move between computers with the same kind.",
            manifest.name,
            if host_arm {
                "an Intel or AMD"
            } else {
                "an ARM"
            },
            if host_arm {
                "an ARM one"
            } else {
                "an Intel or AMD one"
            },
        )));
    }
    let image = sdk
        .system_images()
        .into_iter()
        .find(|i| i.sysdir == manifest.sysdir)
        .ok_or_else(|| {
            Error::Message(format!(
                "{} needs {}, {}, which isn't installed here. Download it in Android Versions, then import again.",
                manifest.name,
                manifest
                    .release
                    .clone()
                    .unwrap_or_else(|| crate::sdk::android_name(manifest.api)),
                crate::sdk::image_kind(&manifest.tag)
            ))
        })?;
    let name = match name {
        Some(n) => n.trim().to_string(),
        None => store.free_name(&manifest.name)?,
    };
    let mut device = store.create(&name, &image, Default::default())?;
    // The new device's folder is replaced by the exported one, keeping the
    // pointer the store just wrote.
    let failed = |e: &dyn std::fmt::Display| Error::Message(format!("Importing failed: {e}"));
    let file = std::fs::File::open(path).map_err(|e| failed(&e))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| failed(&e))?;
    let total: u64 = (0..zip.len())
        .filter_map(|i| zip.by_index_raw(i).ok().map(|e| e.size()))
        .sum();
    let result = (|| -> Result<()> {
        std::fs::remove_dir_all(&device.dir).map_err(|e| failed(&e))?;
        std::fs::create_dir_all(&device.dir).map_err(|e| failed(&e))?;
        let mut done = 0u64;
        let mut buffer = vec![0u8; 1 << 20];
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i).map_err(|e| failed(&e))?;
            let Some(inside) = entry.enclosed_name() else {
                continue;
            };
            let Ok(relative) = inside.strip_prefix(FOLDER.trim_end_matches('/')) else {
                continue;
            };
            let target = device.dir.join(relative);
            if entry.is_dir() {
                std::fs::create_dir_all(&target).map_err(|e| failed(&e))?;
                continue;
            }
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|e| failed(&e))?;
            }
            let mut out = std::fs::File::create(&target).map_err(|e| failed(&e))?;
            let mut length = 0u64;
            loop {
                let read = entry.read(&mut buffer).map_err(|e| failed(&e))?;
                if read == 0 {
                    break;
                }
                // Runs of zeros, most of a disk image, stay gaps, so the
                // file takes no more room than it did.
                if buffer[..read].iter().all(|b| *b == 0) {
                    out.seek(SeekFrom::Current(read as i64))
                        .map_err(|e| failed(&e))?;
                } else {
                    out.write_all(&buffer[..read]).map_err(|e| failed(&e))?;
                }
                length += read as u64;
                done += read as u64;
                progress(done, total);
            }
            out.set_len(length).map_err(|e| failed(&e))?;
        }
        progress(total, total);
        Ok(())
    })();
    if let Err(e) = result {
        let _ = store.delete(&device);
        return Err(e);
    }
    // Its own record, under its new name.
    let exported = crate::device::load_device(&device.dir)
        .ok_or_else(|| failed(&"the device's own record is missing"))?;
    let mut meta = exported.meta;
    meta.name = name;
    meta.created = device.meta.created;
    device.meta = meta;
    device.save_meta()?;
    Ok(device)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_files_and_the_quick_start_stay_behind() {
        assert!(left_out("hardware-qemu.ini"));
        assert!(left_out("multiinstance.lock"));
        assert!(left_out("snapshots/default_boot/ram.bin"));
        assert!(!left_out("snapshots/before-login/ram.bin"));
        assert!(!left_out("userdata-qemu.img.qcow2"));
        assert!(!left_out("aae.toml"));
    }

    #[test]
    fn exports_end_in_aaedevice() {
        assert_eq!(
            file_for(Path::new("/a/Pixel")),
            PathBuf::from("/a/Pixel.aaedevice")
        );
        assert_eq!(
            file_for(Path::new("/a/Pixel 9.aaedevice")),
            PathBuf::from("/a/Pixel 9.aaedevice")
        );
    }
}
