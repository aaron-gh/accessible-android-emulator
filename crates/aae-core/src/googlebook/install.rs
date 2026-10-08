//! Installing what Googlebook devices need, in AAE's data folder:
//!
//! - `components/`: AAE's component bundle (see googlebook/build.sh)
//! - `utm/`: the frameworks and data of UTM 5.0.6, whose QEMU runs the VM
//! - `cuttlefish/`: parts of Google's Cuttlefish image (see [`super::cuttlefish`])
//! - `image/`: the disk, kernel and ramdisk assembled from Google's recovery
//!   image for the Dell Googlebook, which is kept as `image/recovery.raw`
//!
//! Every download is checked against a pinned SHA-256 and resumes where it
//! stopped. `AAE_GOOGLEBOOK_DOWNLOADS` names a folder to take downloads from
//! first, and `AAE_GOOGLEBOOK_COMPONENTS` a component bundle (.tar.gz) to
//! use instead of the published one.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

use crate::error::{Error, IoContext, Result};

/// The component bundle this version of AAE uses, published as release
/// googlebook-components-<version>. googlebook/release.sh sets these.
pub const COMPONENTS_VERSION: u32 = 1;
const COMPONENTS_SHA256: &str = "3d1d40b1703157f17f3b2d85f7134de3361da638f45ed04b27315d8e6b7cfc5e";
const COMPONENTS_SIZE: u64 = 7_668_079;

fn components_file() -> String {
    format!("aae-googlebook-{COMPONENTS_VERSION}.tar.gz")
}

fn components_url() -> String {
    format!(
        "https://github.com/aaron-gh/accessible-android-emulator/releases/download/googlebook-components-{COMPONENTS_VERSION}/{}",
        components_file()
    )
}

const UTM_URL: &str = "https://github.com/utmapp/UTM/releases/download/v5.0.6/UTM.dmg";
const UTM_SHA256: &str = "6a722486a660e0ab2cf5826bbeaee0f5963999029366709f5a3048d73b1d7cb1";
const UTM_SIZE: u64 = 303_864_562;
const UTM_VERSION: &str = "5.0.6";

/// Google's recovery image for the Dell Googlebook, build 16471258.
const RECOVERY_URL: &str = "https://dl.google.com/device/recovery/mica-user/16471258/recovery.zip";
const RECOVERY_SHA256: &str = "cb68dd6dbd73e568cfeecca0cbd4ceb23338640814dc3ee52aba5507e5a645d7";
const RECOVERY_SIZE: u64 = 7_601_764_157;
const RECOVERY_ENTRY: &str = "android-desktop_signed_recovery_image.bin";
const RECOVERY_RAW_SHA256: &str =
    "b0fd614ffe1a088fa3451a4b81c55a73f83e246286184a46ca306250db9af1f1";

/// Roughly what installing downloads, for telling people before they start.
pub const DOWNLOAD_BYTES: u64 =
    COMPONENTS_SIZE + UTM_SIZE + super::cuttlefish::ZIP_SIZE + RECOVERY_SIZE;
/// Roughly the disk space installing takes.
pub const DISK_BYTES: u64 = 22_000_000_000;

/// Progress while installing.
#[derive(Debug, Clone)]
pub enum Progress {
    /// A step starts, such as "Assembling the Googlebook disk".
    Step(String),
    Downloading {
        what: &'static str,
        done: u64,
        total: u64,
    },
}

pub fn root() -> PathBuf {
    crate::paths::data_dir().join("googlebook")
}
pub fn components() -> PathBuf {
    root().join("components")
}
pub fn utm() -> PathBuf {
    root().join("utm")
}
pub fn cuttlefish() -> PathBuf {
    root().join("cuttlefish")
}
pub fn image() -> PathBuf {
    root().join("image")
}

fn stamp(dir: &Path) -> Option<String> {
    std::fs::read_to_string(dir.join("installed.txt"))
        .ok()
        .map(|s| s.trim().to_string())
}

fn set_stamp(dir: &Path, value: &str) -> Result<()> {
    let path = dir.join("installed.txt");
    std::fs::write(&path, value).context(|| format!("Writing {}", path.display()))
}

fn components_stamp() -> String {
    format!("components {COMPONENTS_VERSION}")
}

fn cuttlefish_stamp() -> String {
    format!("cuttlefish {}", super::cuttlefish::BUILD)
}

fn image_stamp() -> String {
    format!("{} {}", components_stamp(), cuttlefish_stamp())
}

/// Whether everything is installed for this version of AAE.
pub fn installed() -> bool {
    stamp(&components()).as_deref() == Some(&components_stamp())
        && stamp(&utm()).as_deref() == Some(UTM_VERSION)
        && stamp(&cuttlefish()).as_deref() == Some(&cuttlefish_stamp())
        && stamp(&image()).as_deref() == Some(&image_stamp())
}

/// Installs whatever is missing. Safe to run again after it stopped part way.
pub fn install(progress: &mut dyn FnMut(Progress)) -> Result<()> {
    if !super::supported() {
        return Err(Error::Message(
            "Googlebook devices need a Mac with Apple silicon.".into(),
        ));
    }
    let downloads = root().join("downloads");
    std::fs::create_dir_all(&downloads).context(|| format!("Creating {}", downloads.display()))?;

    if stamp(&components()).as_deref() != Some(&components_stamp()) {
        let bundle = match std::env::var_os("AAE_GOOGLEBOOK_COMPONENTS") {
            Some(path) => PathBuf::from(path),
            None => {
                let path = downloads.join(components_file());
                fetch(
                    || Ok(components_url()),
                    &path,
                    COMPONENTS_SHA256,
                    COMPONENTS_SIZE,
                    "AAE's Googlebook components",
                    progress,
                )?;
                path
            }
        };
        progress(Progress::Step(
            "Unpacking AAE's Googlebook components".into(),
        ));
        let dir = components();
        let _ = std::fs::remove_dir_all(&dir);
        let unpack = root().join("components.part");
        let _ = std::fs::remove_dir_all(&unpack);
        std::fs::create_dir_all(&unpack).context(|| format!("Creating {}", unpack.display()))?;
        run(
            Command::new("/usr/bin/tar")
                .arg("-xzf")
                .arg(&bundle)
                .arg("-C")
                .arg(&unpack)
                .arg("--strip-components=1"),
            "unpack AAE's Googlebook components",
        )?;
        std::fs::rename(&unpack, &dir).context(|| format!("Moving {}", dir.display()))?;
        set_stamp(&dir, &components_stamp())?;
        let _ = std::fs::remove_file(downloads.join(components_file()));
    }

    if stamp(&utm()).as_deref() != Some(UTM_VERSION) {
        let dmg = downloads.join("UTM-5.0.6.dmg");
        fetch(
            || Ok(UTM_URL.into()),
            &dmg,
            UTM_SHA256,
            UTM_SIZE,
            "UTM",
            progress,
        )?;
        progress(Progress::Step("Copying QEMU from UTM".into()));
        install_utm(&dmg)?;
        let _ = std::fs::remove_file(&dmg);
    }

    if stamp(&cuttlefish()).as_deref() != Some(&cuttlefish_stamp()) {
        use super::cuttlefish as cf;
        let zip = downloads.join(cf::ZIP);
        fetch(
            cf::download_url,
            &zip,
            cf::ZIP_SHA256,
            cf::ZIP_SIZE,
            "Google's Cuttlefish image",
            progress,
        )?;
        progress(Progress::Step(
            "Extracting parts of Google's Cuttlefish image".into(),
        ));
        let tools = super::image::Tools {
            dir: components().join("tools"),
        };
        cf::extract(&zip, &cuttlefish(), &tools)?;
        set_stamp(&cuttlefish(), &cuttlefish_stamp())?;
        let _ = std::fs::remove_file(&zip);
    }

    let image_dir = image();
    if stamp(&image_dir).as_deref() != Some(&image_stamp()) {
        std::fs::create_dir_all(&image_dir)
            .context(|| format!("Creating {}", image_dir.display()))?;
        let raw = image_dir.join("recovery.raw");
        if !raw.exists() {
            let zip = downloads.join("mica-recovery.zip");
            fetch(
                || Ok(RECOVERY_URL.into()),
                &zip,
                RECOVERY_SHA256,
                RECOVERY_SIZE,
                "Google's Googlebook recovery image",
                progress,
            )?;
            progress(Progress::Step(
                "Unpacking Google's Googlebook recovery image".into(),
            ));
            unzip_recovery(&zip, &raw)?;
            let _ = std::fs::remove_file(&zip);
        }
        progress(Progress::Step("Assembling the Googlebook disk".into()));
        let _ = std::fs::remove_file(image_dir.join("installed.txt"));
        super::image::assemble(
            &raw,
            &components(),
            &cuttlefish(),
            &image_dir,
            &mut |step| progress(Progress::Step(format!("{step}."))),
        )?;
        set_stamp(&image_dir, &image_stamp())?;
    }
    Ok(())
}

fn run(command: &mut Command, what: &str) -> Result<()> {
    let out = command.output().context(|| format!("Trying to {what}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(Error::Message(format!(
            "Couldn't {what}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )))
    }
}

/// Copies UTM's frameworks, QEMU's data and the Vulkan driver description
/// out of UTM's disk image.
fn install_utm(dmg: &Path) -> Result<()> {
    let mount = root().join("utm-mount");
    let _ = std::fs::remove_dir_all(&mount);
    std::fs::create_dir_all(&mount).context(|| format!("Creating {}", mount.display()))?;
    run(
        Command::new("/usr/bin/hdiutil")
            .args([
                "attach",
                "-quiet",
                "-nobrowse",
                "-readonly",
                "-noautoopen",
                "-mountpoint",
            ])
            .arg(&mount)
            .arg(dmg),
        "open UTM's disk image",
    )?;
    let copy = || -> Result<()> {
        let part = root().join("utm.part");
        let _ = std::fs::remove_dir_all(&part);
        let app = mount.join("UTM.app/Contents");
        for (from, to) in [
            ("Frameworks", "Frameworks"),
            ("Resources/qemu", "Resources/qemu"),
            ("Resources/vulkan", "Resources/vulkan"),
        ] {
            run(
                Command::new("/usr/bin/ditto")
                    .arg(app.join(from))
                    .arg(part.join(to)),
                "copy QEMU from UTM",
            )?;
        }
        let dir = utm();
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::rename(&part, &dir).context(|| format!("Moving {}", dir.display()))?;
        set_stamp(&dir, UTM_VERSION)
    };
    let result = copy();
    let _ = Command::new("/usr/bin/hdiutil")
        .args(["detach", "-quiet", "-force"])
        .arg(&mount)
        .status();
    let _ = std::fs::remove_dir(&mount);
    result
}

/// Unpacks the disk image from Google's recovery zip, checking it.
fn unzip_recovery(zip: &Path, raw: &Path) -> Result<()> {
    let file = std::fs::File::open(zip).context(|| format!("Opening {}", zip.display()))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| {
        Error::Download(format!(
            "Google's recovery image isn't a valid archive: {e}"
        ))
    })?;
    let mut entry = archive.by_name(RECOVERY_ENTRY).map_err(|e| {
        Error::Download(format!("Google's recovery image is missing its disk: {e}"))
    })?;
    let part = raw.with_extension("raw.part");
    let mut out =
        std::fs::File::create(&part).context(|| format!("Creating {}", part.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 8 << 20];
    loop {
        let n = entry.read(&mut buffer).map_err(|e| {
            Error::Download(format!("Google's recovery image couldn't be unpacked: {e}"))
        })?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
        out.write_all(&buffer[..n])
            .context(|| format!("Writing {}", part.display()))?;
    }
    out.sync_all()
        .context(|| format!("Writing {}", part.display()))?;
    if hex(&hasher.finalize()) != RECOVERY_RAW_SHA256 {
        let _ = std::fs::remove_file(&part);
        return Err(Error::Download(
            "Google's recovery image unpacked to something unexpected, so it wasn't used.".into(),
        ));
    }
    std::fs::rename(&part, raw).context(|| format!("Moving {}", raw.display()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path).context(|| format!("Opening {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 8 << 20];
    loop {
        let n = file
            .read(&mut buffer)
            .context(|| format!("Reading {}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

/// Downloads `url` to `path` unless a file there (or of the same name in
/// `AAE_GOOGLEBOOK_DOWNLOADS`) already matches `sha256`. Resumes a partial
/// download.
fn fetch(
    url: impl FnOnce() -> Result<String>,
    path: &Path,
    sha256: &str,
    size: u64,
    what: &'static str,
    progress: &mut dyn FnMut(Progress),
) -> Result<()> {
    if path.exists() && sha256_file(path)? == sha256 {
        return Ok(());
    }
    if let (Some(dir), Some(name)) = (
        std::env::var_os("AAE_GOOGLEBOOK_DOWNLOADS"),
        path.file_name(),
    ) {
        let seed = PathBuf::from(dir).join(name);
        if seed.exists() && sha256_file(&seed)? == sha256 {
            let _ = std::fs::remove_file(path);
            run(
                Command::new("/bin/cp").arg("-c").arg(&seed).arg(path),
                "copy a download",
            )?;
            return Ok(());
        }
    }
    let part = path.with_extension("part");
    let mut done = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    let url = url()?;
    let mut request = ureq::get(&url);
    if done > 0 {
        request = request.header("Range", &format!("bytes={done}-"));
    }
    let mut response = request
        .call()
        .map_err(|e| Error::Download(format!("Couldn't download {what}: {e}")))?;
    if response.status() != 206 {
        done = 0;
    }
    let mut out = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(done > 0)
        .truncate(done == 0)
        .open(&part)
        .context(|| format!("Creating {}", part.display()))?;
    let mut reader = response.body_mut().with_config().limit(size * 2).reader();
    let mut buffer = vec![0u8; 1 << 20];
    progress(Progress::Downloading {
        what,
        done,
        total: size,
    });
    loop {
        let n = reader.read(&mut buffer).map_err(|e| {
            Error::Download(format!(
                "The download of {what} stopped: {e}. Try again to continue it."
            ))
        })?;
        if n == 0 {
            break;
        }
        out.write_all(&buffer[..n])
            .context(|| format!("Writing {}", part.display()))?;
        done += n as u64;
        progress(Progress::Downloading {
            what,
            done,
            total: size,
        });
    }
    out.sync_all()
        .context(|| format!("Writing {}", part.display()))?;
    progress(Progress::Step(format!("Checking {what}")));
    if sha256_file(&part)? != sha256 {
        let _ = std::fs::remove_file(&part);
        return Err(Error::Download(format!(
            "The download of {what} was damaged on the way, so it wasn't used. Try again."
        )));
    }
    std::fs::rename(&part, path).context(|| format!("Moving {}", path.display()))
}
