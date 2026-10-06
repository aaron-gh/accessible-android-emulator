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
                    if let Some(image) = SystemImage::read(&self.root, &abi.path()) {
                        images.push(image);
                    }
                }
            }
        }
        images.sort_by(|a, b| b.api.cmp(&a.api).then(a.tag.cmp(&b.tag)));
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
    fn read(sdk_root: &Path, dir: &Path) -> Option<Self> {
        let props = read_properties(&dir.join("source.properties")).ok()?;
        let api = props.get("AndroidVersion.ApiLevel")?.parse().ok()?;
        let tag = props.get("SystemImage.TagId")?.clone();
        let abi = props.get("SystemImage.Abi")?.clone();
        let relative = dir.strip_prefix(sdk_root).ok()?;
        let sysdir = format!("{}/", relative.to_string_lossy().replace('\\', "/"));
        Some(SystemImage {
            api,
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
        write!(f, "{}, {}", android_name(self.api), self.kind())?;
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
        _ => "Other",
    }
}

/// The marketing name and API level of an Android version, such as
/// "Android 15 (API 35)".
pub fn android_name(api: u32) -> String {
    let version = match api {
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
        _ => return format!("Android API {api}"),
    };
    format!("Android {version} (API {api})")
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
