//! The device store: AAE's named virtual devices.
//!
//! Each device is a standard AVD folder (`<id>.avd`) in AAE's devices folder,
//! plus `aae.toml` with AAE's metadata ([`DeviceMeta`]).

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
    /// Folds in, like a Galaxy Z Fold: a narrow screen folded, a nearly
    /// square one unfolded.
    Foldable,
}

impl Profile {
    pub fn describe(self) -> &'static str {
        match self {
            Profile::SmallPhone => "Small phone",
            Profile::Phone => "Phone",
            Profile::Tablet => "Tablet",
            Profile::Foldable => "Foldable",
        }
    }

    /// Width, height, density and memory in megabytes.
    fn hardware(self) -> (u32, u32, u32, u32) {
        match self {
            Profile::SmallPhone => (720, 1280, 320, 2048),
            Profile::Phone => (1080, 2400, 420, 2048),
            Profile::Tablet => (2560, 1600, 320, 3072),
            // Unfolded; folded, the left half (see FOLDABLE).
            Profile::Foldable => (1768, 2208, 420, 3072),
        }
    }
}

/// The emulator's settings for a foldable: a hinge down the middle, and
/// folded, only the left half of the screen, as Android Studio's 7.6-inch
/// foldable has.
const FOLDABLE: &[(&str, &str)] = &[
    ("hw.sensor.hinge", "yes"),
    ("hw.sensor.hinge.count", "1"),
    ("hw.sensor.hinge.type", "1"),
    ("hw.sensor.hinge.sub_type", "1"),
    ("hw.sensor.hinge.ranges", "0-180"),
    ("hw.sensor.hinge.defaults", "180"),
    ("hw.sensor.hinge.areas", "884-0-1-2208"),
    ("hw.sensor.posture_list", "1, 2, 3"),
    ("hw.sensor.hinge_angles_posture_definitions", "0-30, 30-150, 150-180"),
    ("hw.sensor.hinge.fold_to_displayRegion.0.1_at_posture", "1"),
    ("hw.displayRegion.0.1.xOffset", "0"),
    ("hw.displayRegion.0.1.yOffset", "0"),
    ("hw.displayRegion.0.1.width", "884"),
    ("hw.displayRegion.0.1.height", "2208"),
];

/// What runs a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum DeviceKind {
    /// Google's Android emulator, from an SDK system image.
    #[default]
    Emulator,
    /// Googlebook OS in a QEMU virtual machine, from a gbos-vm build (see
    /// [`crate::googlebook`]). Apple silicon Macs only.
    Googlebook,
}

impl DeviceKind {
    pub fn is_emulator(&self) -> bool {
        *self == DeviceKind::Emulator
    }
}

/// What AAE stores about a device, in `aae.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceMeta {
    /// The name the user chose, such as "Pixel 15 clean".
    pub name: String,
    #[serde(default, skip_serializing_if = "DeviceKind::is_emulator")]
    pub kind: DeviceKind,
    pub api: u32,
    /// The exact release, such as "Android 16 (API 36.1)" or "Android 17
    /// Beta 3 preview, 16 KB pages". Missing for devices made before AAE
    /// recorded it, which then go by `api`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release: Option<String>,
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
    /// While AAE's speech relay is the device's speech engine, for the speech
    /// log, the speech bridge or both: the real engine it passes requests
    /// to, to restore when neither needs it.
    #[serde(default)]
    pub speech_log_engine: Option<String>,
    /// Whether the relay records the speech log. None on devices from before
    /// the speech bridge, when the relay always meant the speech log.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speech_log: Option<bool>,
    /// The helper version and real engine speech last worked through the
    /// relay with, as "<version> <engine>", so switching to it again
    /// needn't check.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relay_verified: Option<String>,
    /// How loud AAE plays this device's audio on the computer, from 0 to 1.
    /// None is full volume.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playback_volume: Option<f32>,
    /// Speech bridge on: TTS requests go to the host while an AAE app is
    /// connected.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub speech_bridge: bool,
    /// Processor cores chosen for this device. None lets AAE choose for the
    /// computer, fewer on a small one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cores: Option<u32>,
    /// The computer's audio output this device plays through, by name. None
    /// is the default output, following the system's choice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_output: Option<String>,
    /// The keyboard layout chosen for the device, by name, such as
    /// "german". None follows the computer's keyboard in the apps, and is
    /// English (US) otherwise (see [`crate::keyboard_layouts`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyboard_layout: Option<String>,
    /// The emulator couldn't use the computer's graphics adapter for this
    /// device, so it draws the screen in software, which is slower.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub software_graphics: bool,
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
    /// For a Googlebook device, its virtual machine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vm: Option<VmRuntime>,
}

/// A running Googlebook device's virtual machine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VmRuntime {
    /// Where QEMU's sockets are: control (`qmp.sock`), sound (`spice.sock`)
    /// and serial console (`serial.sock`). Short, as socket paths must be.
    pub run_dir: PathBuf,
    /// The display's size in pixels.
    pub width: u32,
    pub height: u32,
}

impl RuntimeInfo {
    /// The adb serial number, such as `emulator-5554`, or for a Googlebook
    /// device, the address adb connects to, such as `127.0.0.1:6520`.
    pub fn serial(&self) -> String {
        if self.vm.is_some() {
            format!("127.0.0.1:{}", self.adb_port)
        } else {
            format!("emulator-{}", self.console_port)
        }
    }
}

impl DeviceMeta {
    /// Whether the speech log is on.
    pub fn speech_log_on(&self) -> bool {
        self.speech_log.unwrap_or(self.speech_log_engine.is_some())
    }
}

impl Device {
    /// Which Android it runs, such as "Android 16 (API 36.1)".
    pub fn android(&self) -> String {
        self.meta
            .release
            .clone()
            .unwrap_or_else(|| android_name(self.meta.api))
    }

    pub fn describe(&self) -> String {
        if self.meta.kind == DeviceKind::Googlebook {
            return format!("{}: {}, virtual machine", self.meta.name, self.android());
        }
        format!(
            "{}: {}, {}, {}",
            self.meta.name,
            self.android(),
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
            kind: DeviceKind::Emulator,
            api: image.api,
            release: Some(if image.release.page_16k {
                format!("{}, 16 KB pages", image.release.describe())
            } else {
                image.release.describe()
            }),
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
            playback_volume: None,
            software_graphics: false,
            audio_output: None,
            keyboard_layout: None,
            cores: None,
            speech_bridge: false,
            speech_log: None,
            relay_verified: None,
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
        if device.meta.kind.is_emulator() {
            write_avd_ini(&device)?;
        }
        Ok(device)
    }

    /// Creates a Googlebook device. Googlebook OS must be installed (see
    /// [`crate::googlebook::install`]). Its disk is a copy-on-write clone of
    /// the installed one.
    pub fn create_googlebook(&self, name: &str) -> Result<Device> {
        let (id, dir) = self.reserve(name)?;
        std::fs::create_dir_all(&dir).context(|| format!("Creating {}", dir.display()))?;
        if let Err(e) = crate::googlebook::copy_image(&dir) {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(e);
        }
        let meta = DeviceMeta {
            name: name.trim().to_string(),
            kind: DeviceKind::Googlebook,
            api: crate::googlebook::API,
            release: Some(crate::googlebook::RELEASE.to_string()),
            tag: crate::googlebook::TAG.to_string(),
            abi: "arm64-v8a".to_string(),
            sysdir: String::new(),
            profile: Profile::Tablet,
            created: now(),
            notes: String::new(),
            provisioned: false,
            screen_reader: None,
            screen_reader_declined: false,
            pending_screen_reader: None,
            audio_speed: Some(1.0),
            speech_log_engine: None,
            playback_volume: None,
            software_graphics: false,
            audio_output: None,
            keyboard_layout: None,
            cores: None,
            speech_bridge: false,
            speech_log: None,
            relay_verified: None,
            keep_enabled: Vec::new(),
            app_choices: Default::default(),
        };
        let device = Device { id, dir, meta };
        device.save_meta()?;
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

    /// `name` if no device has it, otherwise it with the first free number
    /// after it, such as "Pixel 2".
    pub fn free_name(&self, name: &str) -> Result<String> {
        let taken: Vec<String> = self
            .list()?
            .into_iter()
            .map(|d| d.meta.name.to_lowercase())
            .collect();
        let name = name.trim();
        if !taken.contains(&name.to_lowercase()) {
            return Ok(name.to_string());
        }
        Ok((2..)
            .map(|n| format!("{name} {n}"))
            .find(|n| !taken.contains(&n.to_lowercase()))
            .expect("some number is free"))
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
        for device in self.list()?.iter().filter(|d| d.meta.kind.is_emulator()) {
            write_avd_ini(device)?;
        }
        Ok(())
    }
}

pub(crate) fn load_device(dir: &Path) -> Option<Device> {
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
    if meta.profile == Profile::Foldable {
        for (key, value) in FOLDABLE {
            text.push_str(&format!("{key}={value}\n"));
        }
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
            release: crate::sdk::Release::read("35", "android-35", false, false).unwrap(),
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
    fn remembers_the_exact_release() {
        let store = temp_store("release");
        let mut image = image();
        image.api = 37;
        image.release = crate::sdk::Release::read("37.1", "android-37.1", false, true).unwrap();
        let device = store.create("Seventeen", &image, Profile::Phone).unwrap();
        assert_eq!(device.android(), "Android 17 (API 37.1), 16 KB pages");
        let reread = store.get("Seventeen").unwrap();
        assert_eq!(reread.android(), "Android 17 (API 37.1), 16 KB pages");
        // Devices made before AAE recorded it go by their API level.
        let mut older = reread.clone();
        older.meta.release = None;
        assert_eq!(older.android(), "Android 17 (API 37)");
        let _ = std::fs::remove_dir_all(&store.root);
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
