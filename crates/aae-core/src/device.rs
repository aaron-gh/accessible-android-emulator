//! The device store: AAE's named virtual devices.
//!
//! Each device is an ordinary emulator AVD folder (`<id>.avd`) inside AAE's
//! devices folder, plus an `aae.toml` file holding what AAE knows about it: its
//! display name, the screen reader it uses, and the accessibility services to
//! keep on. Because they are ordinary AVDs, the emulator runs them as they are,
//! and Android Studio can use them by pointing `ANDROID_AVD_HOME` at the folder.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::{Error, IoContext, Result};
use crate::paths;
use crate::sdk::{SystemImage, android_name, image_kind};

const META_FILE: &str = "aae.toml";
const RUNTIME_FILE: &str = "running.toml";

/// Files in an AVD folder that belong to one run of the emulator and must not
/// be copied into a clone.
const RUN_ONLY_FILES: &[&str] = &[
    RUNTIME_FILE,
    "emulator.log",
    "multiinstance.lock",
    "hardware-qemu.ini.lock",
    "read-snapshot.txt",
    "tmpAdbCmds",
];

/// Screen size and memory, in plain words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Profile {
    SmallPhone,
    #[default]
    Phone,
    Tablet,
}

impl Profile {
    pub fn describe(self) -> &'static str {
        match self {
            Profile::SmallPhone => "Small phone",
            Profile::Phone => "Phone",
            Profile::Tablet => "Tablet",
        }
    }

    /// Width, height, density and memory in megabytes.
    fn hardware(self) -> (u32, u32, u32, u32) {
        match self {
            Profile::SmallPhone => (720, 1280, 320, 2048),
            Profile::Phone => (1080, 2400, 420, 2048),
            Profile::Tablet => (2560, 1600, 320, 3072),
        }
    }
}

/// What AAE stores about a device, in `aae.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceMeta {
    /// The name the user chose, such as "Pixel 15 clean".
    pub name: String,
    pub api: u32,
    pub tag: String,
    pub abi: String,
    pub sysdir: String,
    #[serde(default)]
    pub profile: Profile,
    /// Seconds since 1970.
    pub created: u64,
    #[serde(default)]
    pub notes: String,
    /// True once first-boot setup has finished.
    #[serde(default)]
    pub provisioned: bool,
    /// The screen reader's accessibility service, as `package/class`.
    #[serde(default)]
    pub screen_reader: Option<String>,
    /// The user chose to go without a screen reader on this device, so AAE
    /// doesn't offer one again.
    #[serde(default)]
    pub screen_reader_declined: bool,
    /// A screen reader build waiting to be installed when the device next
    /// starts, queued while it was stopped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_screen_reader: Option<PathBuf>,
    /// How fast the device really plays audio, as measured: 1.0 when right.
    /// None until measured. See [`crate::audio::measure_speed`].
    #[serde(default)]
    pub audio_speed: Option<f64>,
    /// While the speech log is on: the real speech engine AAE's relay passes
    /// requests to, to restore when it's turned off.
    #[serde(default)]
    pub speech_log_engine: Option<String>,
    /// Accessibility services AAE turns back on after every boot and install,
    /// as `package/class`. Includes the screen reader.
    #[serde(default)]
    pub keep_enabled: Vec<String>,
    /// What the user chose for each special part of an installed app, by
    /// component: accessibility services, keyboards, notification listeners
    /// and device administrators. Applied again when the app is reinstalled.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub app_choices: std::collections::BTreeMap<String, bool>,
}

/// One device in the store.
#[derive(Debug, Clone)]
pub struct Device {
    /// The emulator's name for the device (the AVD name). Letters, digits and dashes.
    pub id: String,
    pub dir: PathBuf,
    pub meta: DeviceMeta,
}

/// The ports and process of a running device, in `running.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeInfo {
    pub pid: u32,
    pub console_port: u16,
    pub adb_port: u16,
    pub grpc_port: u16,
    pub log: PathBuf,
}

impl RuntimeInfo {
    /// The adb serial number, such as `emulator-5554`.
    pub fn serial(&self) -> String {
        format!("emulator-{}", self.console_port)
    }
}

impl Device {
    pub fn describe(&self) -> String {
        format!(
            "{}: {}, {}, {}",
            self.meta.name,
            android_name(self.meta.api),
            image_kind(&self.meta.tag),
            self.meta.profile.describe()
        )
    }

    pub fn save_meta(&self) -> Result<()> {
        write_toml(&self.dir.join(META_FILE), &self.meta)
    }

    pub fn runtime(&self) -> Option<RuntimeInfo> {
        let path = self.dir.join(RUNTIME_FILE);
        let text = std::fs::read_to_string(path).ok()?;
        toml::from_str(&text).ok()
    }

    pub fn set_runtime(&self, info: Option<&RuntimeInfo>) -> Result<()> {
        let path = self.dir.join(RUNTIME_FILE);
        match info {
            Some(info) => write_toml(&path, info),
            None => match std::fs::remove_file(&path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                    Err(e).context(|| format!("Removing {}", path.display()))
                }
                _ => Ok(()),
            },
        }
    }

    /// Adds a service to the keep-enabled list. Returns false if it was already there.
    pub fn keep_service_enabled(&mut self, component: &str) -> bool {
        if self.meta.keep_enabled.iter().any(|c| c == component) {
            return false;
        }
        self.meta.keep_enabled.push(component.to_string());
        true
    }

    /// Disk space used by the device, in bytes.
    pub fn disk_usage(&self) -> u64 {
        dir_size(&self.dir)
    }

    fn ini_path(&self) -> PathBuf {
        self.dir.with_extension("ini")
    }
}

/// AAE's collection of devices.
#[derive(Debug, Clone)]
pub struct DeviceStore {
    pub root: PathBuf,
}

impl DeviceStore {
    pub fn open_default() -> Result<Self> {
        Self::open(paths::devices_dir())
    }

    pub fn open(root: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&root).context(|| format!("Creating {}", root.display()))?;
        Ok(DeviceStore { root })
    }

    /// All devices, by name.
    pub fn list(&self) -> Result<Vec<Device>> {
        let mut devices = Vec::new();
        let entries =
            std::fs::read_dir(&self.root).context(|| format!("Reading {}", self.root.display()))?;
        for entry in entries.flatten() {
            let dir = entry.path();
            if dir.extension().is_some_and(|ext| ext == "avd") {
                if let Some(device) = load_device(&dir) {
                    devices.push(device);
                }
            }
        }
        devices.sort_by_key(|d| d.meta.name.to_lowercase());
        Ok(devices)
    }

    /// Finds a device by its name or its id, ignoring case.
    pub fn get(&self, name: &str) -> Result<Device> {
        let wanted = name.trim().to_lowercase();
        self.list()?
            .into_iter()
            .find(|d| d.meta.name.to_lowercase() == wanted || d.id == wanted)
            .ok_or_else(|| Error::DeviceNotFound(name.to_string()))
    }

    /// Creates a new device from an installed system image.
    pub fn create(&self, name: &str, image: &SystemImage, profile: Profile) -> Result<Device> {
        let (id, dir) = self.reserve(name)?;
        std::fs::create_dir_all(&dir).context(|| format!("Creating {}", dir.display()))?;
        let meta = DeviceMeta {
            name: name.trim().to_string(),
            api: image.api,
            tag: image.tag.clone(),
            abi: image.abi.clone(),
            sysdir: image.sysdir.clone(),
            profile,
            created: now(),
            notes: String::new(),
            provisioned: false,
            screen_reader: None,
            screen_reader_declined: false,
            pending_screen_reader: None,
            audio_speed: None,
            speech_log_engine: None,
            keep_enabled: Vec::new(),
            app_choices: Default::default(),
        };
        let device = Device { id, dir, meta };
        write_config_ini(&device)?;
        device.save_meta()?;
        write_avd_ini(&device)?;
        Ok(device)
    }

    /// Copies a device, with all its apps and data, under a new name. On file
    /// systems that support it (APFS, Btrfs, ReFS) the copy shares unchanged
    /// blocks with the original, so it is quick and starts small.
    pub fn clone_device(&self, source: &Device, name: &str) -> Result<Device> {
        if source.runtime().is_some() {
            return Err(Error::MustStop(source.meta.name.clone(), "cloned"));
        }
        let (id, dir) = self.reserve(name)?;
        copy_dir(&source.dir, &dir)?;
        let mut meta = source.meta.clone();
        meta.name = name.trim().to_string();
        meta.created = now();
        let device = Device { id, dir, meta };
        device.save_meta()?;
        write_avd_ini(&device)?;
        Ok(device)
    }

    /// Deletes a device and its files. Returns the bytes freed.
    pub fn delete(&self, device: &Device) -> Result<u64> {
        if device.runtime().is_some() {
            return Err(Error::MustStop(device.meta.name.clone(), "deleted"));
        }
        let size = device.disk_usage();
        std::fs::remove_dir_all(&device.dir)
            .context(|| format!("Deleting {}", device.dir.display()))?;
        let _ = std::fs::remove_file(device.ini_path());
        Ok(size)
    }

    /// Renames a device. Its id, and so its folder, stays the same.
    pub fn rename(&self, device: &mut Device, name: &str) -> Result<()> {
        let wanted = name.trim();
        if wanted.is_empty() {
            return Err(Error::InvalidName(name.to_string()));
        }
        if self
            .list()?
            .iter()
            .any(|d| d.id != device.id && d.meta.name.eq_ignore_ascii_case(wanted))
        {
            return Err(Error::DeviceExists(wanted.to_string()));
        }
        device.meta.name = wanted.to_string();
        device.save_meta()
    }

    /// Checks a new name is free and picks an unused id for it.
    fn reserve(&self, name: &str) -> Result<(String, PathBuf)> {
        let base = slug(name).ok_or_else(|| Error::InvalidName(name.to_string()))?;
        let existing = self.list()?;
        if existing
            .iter()
            .any(|d| d.meta.name.eq_ignore_ascii_case(name.trim()))
        {
            return Err(Error::DeviceExists(name.trim().to_string()));
        }
        let mut id = base.clone();
        let mut n = 2;
        while self.root.join(format!("{id}.avd")).exists() {
            id = format!("{base}-{n}");
            n += 1;
        }
        let dir = self.root.join(format!("{id}.avd"));
        Ok((id, dir))
    }

    /// Rewrites each device's `<id>.ini` pointer, in case the store was moved.
    pub fn repair_pointers(&self) -> Result<()> {
        for device in self.list()? {
            write_avd_ini(&device)?;
        }
        Ok(())
    }
}

fn load_device(dir: &Path) -> Option<Device> {
    let text = std::fs::read_to_string(dir.join(META_FILE)).ok()?;
    let meta: DeviceMeta = toml::from_str(&text).ok()?;
    let id = dir.file_stem()?.to_str()?.to_string();
    Some(Device {
        id,
        dir: dir.to_path_buf(),
        meta,
    })
}

/// Turns a display name into an AVD name: lowercase letters, digits and dashes.
pub fn slug(name: &str) -> Option<String> {
    let mut out = String::new();
    for c in name.trim().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_string();
    (!out.is_empty()).then_some(out)
}

/// Writes the emulator's `config.ini` with AAE's defaults. The hardware keyboard
/// is on, audio in and out are on, and there is no device frame.
fn write_config_ini(device: &Device) -> Result<()> {
    let meta = &device.meta;
    let (width, height, density, ram) = meta.profile.hardware();
    let arch = if meta.abi.starts_with("arm64") {
        "arm64"
    } else {
        "x86_64"
    };
    let play = meta.tag.contains("playstore");
    let entries: Vec<(&str, String)> = vec![
        ("avd.ini.encoding", "UTF-8".into()),
        ("AvdId", device.id.clone()),
        ("avd.ini.displayname", meta.name.clone()),
        ("abi.type", meta.abi.clone()),
        ("hw.cpu.arch", arch.into()),
        ("hw.cpu.ncore", "4".into()),
        ("image.sysdir.1", meta.sysdir.clone()),
        ("tag.id", meta.tag.clone()),
        ("target", format!("android-{}", meta.api)),
        (
            "PlayStore.enabled",
            if play { "true" } else { "false" }.into(),
        ),
        ("hw.lcd.width", width.to_string()),
        ("hw.lcd.height", height.to_string()),
        ("hw.lcd.density", density.to_string()),
        ("hw.ramSize", ram.to_string()),
        ("vm.heapSize", "256".into()),
        ("disk.dataPartition.size", "6G".into()),
        ("hw.keyboard", "yes".into()),
        ("hw.keyboard.lid", "no".into()),
        ("hw.mainKeys", "no".into()),
        ("hw.dPad", "no".into()),
        ("hw.audioInput", "yes".into()),
        ("hw.audioOutput", "yes".into()),
        ("hw.gpu.enabled", "yes".into()),
        ("hw.gpu.mode", "auto".into()),
        ("hw.sdCard", "yes".into()),
        ("sdcard.size", "512M".into()),
        ("hw.battery", "yes".into()),
        ("hw.gps", "yes".into()),
        ("hw.accelerometer", "yes".into()),
        ("hw.sensors.orientation", "yes".into()),
        ("hw.gsmModem", "yes".into()),
        ("hw.initialOrientation", "portrait".into()),
        ("showDeviceFrame", "no".into()),
        ("fastboot.forceColdBoot", "no".into()),
        ("fastboot.forceFastBoot", "yes".into()),
    ];
    let mut text = String::new();
    for (key, value) in entries {
        text.push_str(&format!("{key}={value}\n"));
    }
    let path = device.dir.join("config.ini");
    std::fs::write(&path, text).context(|| format!("Writing {}", path.display()))
}

/// Writes the `<id>.ini` pointer file the emulator uses to find the AVD folder.
fn write_avd_ini(device: &Device) -> Result<()> {
    let text = format!(
        "avd.ini.encoding=UTF-8\npath={}\ntarget=android-{}\n",
        device.dir.display(),
        device.meta.api
    );
    let path = device.ini_path();
    std::fs::write(&path, text).context(|| format!("Writing {}", path.display()))
}

fn write_toml<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let text = toml::to_string_pretty(value).map_err(|e| Error::Config {
        path: path.to_path_buf(),
        reason: e.to_string(),
    })?;
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, text).context(|| format!("Writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).context(|| format!("Writing {}", path.display()))
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to).context(|| format!("Creating {}", to.display()))?;
    let entries = std::fs::read_dir(from).context(|| format!("Reading {}", from.display()))?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        if RUN_ONLY_FILES.iter().any(|skip| name == *skip)
            || name.to_string_lossy().ends_with(".lock")
        {
            continue;
        }
        let src = entry.path();
        let dst = to.join(&name);
        if src.is_dir() {
            copy_dir(&src, &dst)?;
        } else {
            // std::fs::copy clones the file on APFS, so unchanged blocks are shared.
            std::fs::copy(&src, &dst).context(|| format!("Copying {}", src.display()))?;
        }
    }
    Ok(())
}

/// The disk space a folder really uses. Device disks are sparse files, which
/// reserve their full size but take up only what has been written, so this
/// counts allocated blocks where the platform reports them.
pub(crate) fn dir_size(dir: &Path) -> u64 {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| match entry.metadata() {
            Ok(m) if m.is_dir() => dir_size(&entry.path()),
            Ok(m) => allocated(&m),
            Err(_) => 0,
        })
        .sum()
}

#[cfg(unix)]
fn allocated(metadata: &std::fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    metadata.blocks() * 512
}

#[cfg(not(unix))]
fn allocated(metadata: &std::fs::Metadata) -> u64 {
    metadata.len()
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Formats a byte count for speech, such as "2.4 gigabytes".
pub fn human_size(bytes: u64) -> String {
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.1} gigabytes", b / GB)
    } else {
        format!("{:.0} megabytes", b / MB)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image() -> SystemImage {
        SystemImage {
            api: 35,
            tag: "google_apis".into(),
            abi: "arm64-v8a".into(),
            sysdir: "system-images/android-35/google_apis/arm64-v8a/".into(),
            path: PathBuf::from("/nowhere"),
        }
    }

    fn temp_store(test: &str) -> DeviceStore {
        let dir = std::env::temp_dir().join(format!("aae-test-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        DeviceStore::open(dir).unwrap()
    }

    #[test]
    fn slugs_names() {
        assert_eq!(slug("Pixel 15 clean").as_deref(), Some("pixel-15-clean"));
        assert_eq!(slug("  Banking app!! ").as_deref(), Some("banking-app"));
        assert_eq!(slug("!!!"), None);
    }

    #[test]
    fn creates_clones_and_deletes() {
        let store = temp_store("crud");
        let device = store
            .create("Pixel 15 clean", &image(), Profile::Phone)
            .unwrap();
        assert_eq!(device.id, "pixel-15-clean");
        let config = std::fs::read_to_string(device.dir.join("config.ini")).unwrap();
        assert!(config.contains("hw.keyboard=yes"));

        assert!(matches!(
            store.create("pixel 15 CLEAN", &image(), Profile::Phone),
            Err(Error::DeviceExists(_))
        ));

        let clone = store.clone_device(&device, "Pixel 15 banking").unwrap();
        assert_eq!(clone.meta.api, 35);
        assert_eq!(store.list().unwrap().len(), 2);
        assert_eq!(
            store.get("PIXEL 15 BANKING").unwrap().id,
            "pixel-15-banking"
        );

        store.delete(&clone).unwrap();
        assert_eq!(store.list().unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(&store.root);
    }

    #[test]
    fn same_slug_gets_a_new_id() {
        let store = temp_store("ids");
        let a = store.create("Test one", &image(), Profile::Phone).unwrap();
        let b = store.create("Test-one", &image(), Profile::Phone).unwrap();
        assert_ne!(a.id, b.id);
        let _ = std::fs::remove_dir_all(&store.root);
    }
}
