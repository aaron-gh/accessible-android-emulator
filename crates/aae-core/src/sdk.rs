//! Finding the Android SDK and the system images installed in it.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::paths;

const EXE: &str = std::env::consts::EXE_SUFFIX;

/// An Android SDK on this computer.
#[derive(Debug, Clone)]
pub struct Sdk {
    pub root: PathBuf,
}

impl Sdk {
    /// Finds the SDK in this order: `ANDROID_HOME`, `ANDROID_SDK_ROOT`, AAE's own
    /// SDK folder, then the place Android Studio installs it on this platform.
    pub fn locate() -> Result<Self> {
        let mut candidates: Vec<PathBuf> = ["ANDROID_HOME", "ANDROID_SDK_ROOT"]
            .iter()
            .filter_map(std::env::var_os)
            .map(PathBuf::from)
            .collect();
        candidates.push(paths::sdk_dir());
        if let Some(dirs) = directories::BaseDirs::new() {
            let home = dirs.home_dir();
            if cfg!(target_os = "macos") {
                candidates.push(home.join("Library/Android/sdk"));
            } else if cfg!(target_os = "windows") {
                candidates.push(dirs.data_local_dir().join("Android/Sdk"));
            } else {
                candidates.push(home.join("Android/Sdk"));
            }
        }
        candidates
            .into_iter()
            .find(|dir| dir.join("emulator").is_dir() || dir.join("platform-tools").is_dir())
            .map(|root| Sdk { root })
            .ok_or(Error::SdkNotFound)
    }

    /// The SDK AAE uses, or where AAE sets one up when there is none:
    /// `ANDROID_HOME` or `ANDROID_SDK_ROOT` if set, otherwise an SDK already
    /// on this computer, otherwise AAE's own SDK folder.
    pub fn locate_or_new() -> Self {
        for var in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
            if let Some(dir) = std::env::var_os(var).filter(|d| !d.is_empty()) {
                return Sdk { root: dir.into() };
            }
        }
        Self::locate().unwrap_or_else(|_| Sdk {
            root: paths::sdk_dir(),
        })
    }

    pub fn emulator_bin(&self) -> Result<PathBuf> {
        let bin = self.root.join("emulator").join(format!("emulator{EXE}"));
        bin.is_file()
            .then_some(bin)
            .ok_or_else(|| Error::EmulatorMissing(self.root.clone()))
    }

    pub fn adb_bin(&self) -> Result<PathBuf> {
        let bin = self.root.join("platform-tools").join(format!("adb{EXE}"));
        bin.is_file()
            .then_some(bin)
            .ok_or_else(|| Error::AdbMissing(self.root.clone()))
    }

    /// The newest `aapt2` in build-tools, used to read app manifests.
    pub fn aapt2_bin(&self) -> Option<PathBuf> {
        let mut versions: Vec<PathBuf> = std::fs::read_dir(self.root.join("build-tools"))
            .ok()?
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|dir| dir.join(format!("aapt2{EXE}")).is_file())
            .collect();
        versions
            .sort_by_key(|dir| version_key(dir.file_name().and_then(|n| n.to_str()).unwrap_or("")));
        versions.pop().map(|dir| dir.join(format!("aapt2{EXE}")))
    }

    /// The newest `apksigner` in build-tools. AAE doesn't run it, as it needs
    /// Java; it's one of the build tools AAE checks are installed.
    pub fn apksigner_bin(&self) -> Option<PathBuf> {
        let name = if cfg!(windows) {
            "apksigner.bat"
        } else {
            "apksigner"
        };
        let mut versions: Vec<PathBuf> = std::fs::read_dir(self.root.join("build-tools"))
            .ok()?
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|dir| dir.join(name).is_file())
            .collect();
        versions
            .sort_by_key(|dir| version_key(dir.file_name().and_then(|n| n.to_str()).unwrap_or("")));
        versions.pop().map(|dir| dir.join(name))
    }

    /// The emulator's version, such as "37.2.12".
    pub fn emulator_version(&self) -> Option<String> {
        read_properties(&self.root.join("emulator/source.properties"))
            .ok()?
            .remove("Pkg.Revision")
    }

    /// All system images installed in this SDK, newest Android version first.
    pub fn system_images(&self) -> Vec<SystemImage> {
        let mut images = Vec::new();
        let Ok(platforms) = std::fs::read_dir(self.root.join("system-images")) else {
            return images;
        };
        for platform in platforms.flatten() {
            for tag in std::fs::read_dir(platform.path())
                .into_iter()
                .flatten()
                .flatten()
            {
                for abi in std::fs::read_dir(tag.path())
                    .into_iter()
                    .flatten()
                    .flatten()
                {
                    if let Some(image) = SystemImage::read_dir(&self.root, &abi.path()) {
                        images.push(image);
                    }
                }
            }
        }
        images.sort_by(|a, b| {
            b.api
                .cmp(&a.api)
                .then(
                    a.release
                        .preview
                        .is_some()
                        .cmp(&b.release.preview.is_some()),
                )
                .then(b.release.minor.cmp(&a.release.minor))
                .then(a.tag.cmp(&b.tag))
        });
        images
    }
}

/// The processor architecture of system image that runs at full speed on this host.
pub fn host_abi() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "arm64-v8a"
    } else {
        "x86_64"
    }
}

/// One installed Android system image.
#[derive(Debug, Clone)]
pub struct SystemImage {
    /// API level, such as 35.
    pub api: u32,
    /// Which release of that version: an update such as API 36.1, or a preview.
    pub release: Release,
    /// Image type, such as `google_apis_playstore`.
    pub tag: String,
    /// Processor architecture, such as `arm64-v8a`.
    pub abi: String,
    /// The image folder relative to the SDK root, with a trailing slash, as the
    /// emulator expects in `image.sysdir.1`.
    pub sysdir: String,
    pub path: PathBuf,
}

impl SystemImage {
    /// Reads the image installed in `dir`, inside the SDK at `sdk_root`.
    pub fn read_dir(sdk_root: &Path, dir: &Path) -> Option<Self> {
        let props = read_properties(&dir.join("source.properties")).ok()?;
        // The tag can list more than one, as "google_apis,page_size_16kb".
        let tags = props.get("SystemImage.TagId")?.clone();
        let tag = tags.split(',').next()?.trim().to_string();
        let abi = props.get("SystemImage.Abi")?.clone();
        let tag_folder = dir.parent()?.file_name()?.to_string_lossy().to_string();
        let folder = dir
            .parent()?
            .parent()?
            .file_name()?
            .to_string_lossy()
            .to_string();
        // A later update's number is in the API level ("36.1") or a key of
        // its own, depending on the SDK; the folder name says it too.
        let mut level = props.get("AndroidVersion.ApiLevel")?.clone();
        if !level.contains('.')
            && let Some(minor) = props
                .iter()
                .find(|(k, _)| k.starts_with("AndroidVersion.") && k.contains("Minor"))
                .map(|(_, v)| v)
        {
            level = format!("{level}.{minor}");
        }
        let release = Release::read(
            &level,
            &folder,
            props.contains_key("AndroidVersion.CodeName"),
            tag_folder.ends_with("_ps16k") || tags.contains("page_size_16kb"),
        )?;
        let relative = dir.strip_prefix(sdk_root).ok()?;
        let sysdir = format!("{}/", relative.to_string_lossy().replace('\\', "/"));
        Some(SystemImage {
            api: release.api,
            release,
            tag,
            abi,
            sysdir,
            path: dir.to_path_buf(),
        })
    }

    pub fn has_play_store(&self) -> bool {
        self.tag.contains("playstore")
    }

    pub fn runs_natively(&self) -> bool {
        self.abi == host_abi()
    }

    /// The image type in plain words.
    pub fn kind(&self) -> &'static str {
        image_kind(&self.tag)
    }
}

impl fmt::Display for SystemImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}, {}", self.release.describe(), self.kind())?;
        if self.release.page_16k {
            write!(f, ", 16 KB pages")?;
        }
        if !self.runs_natively() {
            write!(f, " ({}, slow on this computer)", self.abi)?;
        }
        Ok(())
    }
}

/// The image type in plain words, from its tag.
pub fn image_kind(tag: &str) -> &'static str {
    match tag {
        "default" | "aosp_atd" => "Plain Android",
        "google_apis" | "google_atd" => "With Google services",
        t if t.contains("playstore") => "With Google Play",
        // Such as google_apis_ps16k, with 16 KB memory pages.
        t if t.starts_with("google_apis") => "With Google services",
        t if t.starts_with("default") => "Plain Android",
        _ => "Other",
    }
}

/// Which Android release an image holds: a version, such as Android 17 (API
/// 37), a later update to one, such as API 36.1, or a preview.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Release {
    /// The API level, such as 37.
    pub api: u32,
    /// A later update's number, such as 1 for API 36.1; 0 for the version itself.
    pub minor: u32,
    /// A preview's name, such as "Beta 3" or "Canary, 9 September 2026".
    pub preview: Option<String>,
    /// Built with 16 KB memory pages, for testing apps with them. Google
    /// publishes previews, and some updates, only this way.
    pub page_16k: bool,
    /// The name of its folder in the SDK without "android-", such as "37.0",
    /// "36.1" or "37.2-beta3", which picks it on the command line.
    pub id: String,
}

impl Release {
    /// From the API level as the SDK writes it, such as "36" or "37.1", and
    /// the image's folder name, such as "android-37.2-beta3".
    pub fn read(api_level: &str, folder: &str, preview: bool, page_16k: bool) -> Option<Release> {
        let (api, minor) = parse_api_level(api_level)?;
        let id = folder
            .strip_prefix("android-")
            .unwrap_or(folder)
            .to_string();
        Some(Release {
            api,
            minor,
            preview: preview.then(|| preview_name(&id)),
            page_16k,
            id,
        })
    }

    /// Such as "Android 17 (API 37)", "Android 16 (API 36.1)" or "Android 17
    /// Beta 3 preview".
    pub fn describe(&self) -> String {
        let version = android_version(self.api)
            .map(String::from)
            .unwrap_or_else(|| format!("API {}", self.api));
        match (&self.preview, self.minor) {
            (Some(preview), _) => match preview.split_once(", ") {
                // "Canary, 9 September 2026": the date after "preview".
                Some((name, detail)) => format!("Android {version} {name} preview, {detail}"),
                None => format!("Android {version} {preview} preview"),
            },
            (None, 0) => android_name(self.api),
            (None, minor) => format!("Android {version} (API {}.{minor})", self.api),
        }
    }

    /// True if `text` names this release, as "37", "36.1" or its folder's
    /// name, such as "37.2-beta3".
    pub fn matches(&self, text: &str) -> bool {
        let text = text.trim().trim_start_matches("android-");
        text.eq_ignore_ascii_case(&self.id)
            || (self.preview.is_none() && parse_api_level(text) == Some((self.api, self.minor)))
    }
}

/// An API level as the SDK writes it, such as "36" or "36.1", as (36, 1).
pub fn parse_api_level(text: &str) -> Option<(u32, u32)> {
    let (major, minor) = text.trim().split_once('.').unwrap_or((text.trim(), "0"));
    Some((major.parse().ok()?, minor.parse().ok()?))
}

/// A preview's name from its folder: "37.2-beta3" is "Beta 3", and
/// "canary-20260909" is "Canary, 9 September 2026".
fn preview_name(id: &str) -> String {
    let lower = id.to_ascii_lowercase();
    if let Some((_, beta)) = lower.split_once("-beta") {
        return format!("Beta {beta}");
    }
    if let Some(date) = lower.strip_prefix("canary-") {
        const MONTHS: [&str; 12] = [
            "January",
            "February",
            "March",
            "April",
            "May",
            "June",
            "July",
            "August",
            "September",
            "October",
            "November",
            "December",
        ];
        if date.len() == 8
            && let (Ok(year), Ok(month), Ok(day)) = (
                date[..4].parse::<u32>(),
                date[4..6].parse::<usize>(),
                date[6..].parse::<u32>(),
            )
            && (1..=12).contains(&month)
        {
            return format!("Canary, {day} {} {year}", MONTHS[month - 1]);
        }
    }
    if lower.starts_with("canary") {
        return "Canary".into();
    }
    id.to_string()
}

/// The marketing name and API level of an Android version, such as
/// "Android 15 (API 35)".
pub fn android_name(api: u32) -> String {
    match android_version(api) {
        Some(version) => format!("Android {version} (API {api})"),
        None => format!("Android API {api}"),
    }
}

/// The marketing version of an API level, such as "15" for 35.
fn android_version(api: u32) -> Option<&'static str> {
    Some(match api {
        21 => "5.0",
        22 => "5.1",
        23 => "6",
        24 => "7.0",
        25 => "7.1",
        26 => "8.0",
        27 => "8.1",
        28 => "9",
        29 => "10",
        30 => "11",
        31 => "12",
        32 => "12L",
        33 => "13",
        34 => "14",
        35 => "15",
        36 => "16",
        37 => "17",
        _ => return None,
    })
}

/// Reads a Java-style `key=value` properties file, as the SDK uses.
pub fn read_properties(path: &Path) -> Result<HashMap<String, String>> {
    let text = std::fs::read_to_string(path).map_err(|e| Error::Config {
        path: path.to_path_buf(),
        reason: e.to_string(),
    })?;
    Ok(parse_properties(&text))
}

pub fn parse_properties(text: &str) -> HashMap<String, String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| line.split_once('='))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect()
}

fn version_key(version: &str) -> Vec<u32> {
    version
        .split(['.', '-'])
        .map(|part| part.parse().unwrap_or(0))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_android_versions() {
        assert_eq!(android_name(35), "Android 15 (API 35)");
        assert_eq!(android_name(99), "Android API 99");
    }

    #[test]
    fn parses_properties() {
        let props = parse_properties("# comment\nA=1\n B = two \n\nbad line\n");
        assert_eq!(props["A"], "1");
        assert_eq!(props["B"], "two");
        assert_eq!(props.len(), 2);
    }

    #[test]
    fn sorts_build_tools_versions() {
        assert!(version_key("36.0.0") > version_key("35.0.1"));
        assert!(version_key("36.1.0-rc1") > version_key("36.0.0"));
    }
}
