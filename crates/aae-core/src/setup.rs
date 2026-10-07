//! Setting up the Android SDK parts AAE needs, without Android Studio: the
//! emulator, the platform tools (adb), and the build tools (aapt2 and
//! apksigner, for reading and checking apps).
//!
//! They come from Google's SDK repository, are checked against its
//! checksums, and are installed as sdkmanager would install them, with the
//! same folders, `package.xml` and licence records, so Android Studio sees
//! them too.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::catalog::{self, Archive, InstallProgress, child, child_text, revision, version_key};
use crate::error::{Error, IoContext, Result};
use crate::paths;
use crate::sdk::{Sdk, read_properties};

const REPOSITORY: &str = "https://dl.google.com/android/repository";
const LIST: &str = "repository2-3.xml";
/// How long a downloaded list is used before fetching it again.
const LIST_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);
/// The file AAE leaves in each tool it installs. AAE updates only these:
/// tools installed another way, such as by Android Studio, are left to it.
/// If Android Studio replaces one, the marker goes with the old folder.
const MARKER: &str = ".installed-by-aae";
/// The namespace the packages' type details use.
const GENERIC_NAMESPACE: &str = "http://schemas.android.com/repository/android/generic/02";

/// One of the SDK's tool packages, as Google offers it for this computer.
#[derive(Debug, Clone)]
pub struct Tool {
    /// The SDK's name for it, such as "emulator" or "build-tools;37.0.0".
    pub path: String,
    /// Such as "Android Emulator".
    pub name: String,
    pub revision: String,
    pub url: String,
    /// Download size in bytes.
    pub size: u64,
    pub sha1: String,
    pub licence_id: String,
    package_xml: String,
}

impl Tool {
    /// Where it goes in the SDK, such as `emulator` or `build-tools/37.0.0`.
    pub fn install_dir(&self, sdk: &Sdk) -> PathBuf {
        self.path
            .split(';')
            .fold(sdk.root.clone(), |dir, part| dir.join(part))
    }

    /// The revision installed in the SDK, if any.
    pub fn installed_revision(&self, sdk: &Sdk) -> Option<String> {
        read_properties(&self.install_dir(sdk).join("source.properties"))
            .ok()?
            .remove("Pkg.Revision")
    }

    /// True if AAE installed the revision that's there now.
    pub fn installed_by_aae(&self, sdk: &Sdk) -> bool {
        self.install_dir(sdk).join(MARKER).is_file()
    }

    fn is_build_tools(&self) -> bool {
        self.path.starts_with("build-tools;")
    }
}

/// The tool packages AAE needs, newest stable revision of each, and the
/// licences they are under.
#[derive(Debug, Clone, Default)]
pub struct Tools {
    pub tools: Vec<Tool>,
    /// Licence text by licence id.
    pub licences: HashMap<String, String>,
}

impl Tools {
    /// Reads Google's list, from AAE's copy if it is less than a day old.
    /// Falls back to an older copy when offline.
    pub fn load(refresh: bool) -> Result<Tools> {
        parse(&list_xml(refresh)?)
    }

    /// The tools this SDK doesn't have yet. Build tools count as present
    /// when any installed version has what AAE uses.
    pub fn missing(&self, sdk: &Sdk) -> Vec<&Tool> {
        self.tools
            .iter()
            .filter(|t| {
                if t.is_build_tools() {
                    sdk.aapt2_bin().is_none() || sdk.apksigner_bin().is_none()
                } else {
                    t.installed_revision(sdk).is_none()
                }
            })
            .collect()
    }

    /// Tools AAE installed that have a newer stable revision available.
    /// Build tools install side by side, so a newer version is never an
    /// update; tools installed another way are left to whatever installed them.
    pub fn updates(&self, sdk: &Sdk) -> Vec<&Tool> {
        self.tools
            .iter()
            .filter(|t| !t.is_build_tools() && t.installed_by_aae(sdk))
            .filter(|t| {
                t.installed_revision(sdk)
                    .is_some_and(|installed| version_key(&installed) < version_key(&t.revision))
            })
            .collect()
    }

    /// The installed emulator and platform tools that something else, such as
    /// Android Studio, installed and keeps up to date.
    pub fn managed_elsewhere(&self, sdk: &Sdk) -> Vec<&Tool> {
        self.tools
            .iter()
            .filter(|t| !t.is_build_tools())
            .filter(|t| t.installed_revision(sdk).is_some() && !t.installed_by_aae(sdk))
            .collect()
    }

    /// The licences of these tools that haven't been accepted in this SDK,
    /// as (id, text).
    pub fn licences_to_accept(&self, sdk: &Sdk, tools: &[&Tool]) -> Vec<(String, String)> {
        let mut ids: Vec<&str> = tools.iter().map(|t| t.licence_id.as_str()).collect();
        ids.sort();
        ids.dedup();
        ids.into_iter()
            .map(|id| {
                let text = self.licences.get(id).cloned().unwrap_or_default();
                (id.to_string(), text)
            })
            .filter(|(id, text)| !catalog::licence_accepted(sdk, id, text))
            .collect()
    }
}

/// Reads Google's SDK repository list: the newest stable emulator, platform
/// tools and build tools for this computer.
fn parse(xml: &str) -> Result<Tools> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| Error::Config {
        path: PathBuf::from(format!("{REPOSITORY}/{LIST}")),
        reason: e.to_string(),
    })?;
    let root = doc.root_element();
    let stable: Vec<String> = root
        .children()
        .filter(|n| n.has_tag_name("channel") && n.text().map(str::trim) == Some("stable"))
        .filter_map(|n| n.attribute("id").map(String::from))
        .collect();
    let licences = root
        .children()
        .filter(|n| n.has_tag_name("license"))
        .filter_map(|n| Some((n.attribute("id")?.to_string(), n.text()?.to_string())))
        .collect();
    let mut tools: Vec<Tool> = root
        .children()
        .filter(|n| n.has_tag_name("remotePackage"))
        .filter_map(|n| read_tool(n, xml, &stable))
        .collect();
    tools.sort_by_key(|t| std::cmp::Reverse(version_key(&t.revision)));
    // The newest of each: emulator, platform-tools, and one build-tools.
    let mut chosen: Vec<Tool> = Vec::new();
    for tool in tools {
        let kind = |t: &Tool| t.path.split(';').next().unwrap_or("").to_string();
        if !chosen.iter().any(|c| kind(c) == kind(&tool)) {
            chosen.push(tool);
        }
    }
    chosen.sort_by_key(|t| match t.path.split(';').next() {
        Some("emulator") => 0,
        Some("platform-tools") => 1,
        _ => 2,
    });
    Ok(Tools {
        tools: chosen,
        licences,
    })
}

fn read_tool(package: roxmltree::Node, xml: &str, stable: &[String]) -> Option<Tool> {
    let path = package.attribute("path")?;
    let wanted = path == "emulator"
        || path == "platform-tools"
        || (path.starts_with("build-tools;") && !path.contains("-rc"));
    if !wanted {
        return None;
    }
    let channel = child(package, "channelRef")
        .and_then(|c| c.attribute("ref"))
        .unwrap_or("channel-0");
    if !stable.iter().any(|s| s == channel) {
        return None;
    }
    let archive = child(package, "archives")?
        .children()
        .filter(|a| a.has_tag_name("archive"))
        .find(|a| {
            child_text(*a, "host-os") == Some(host_os())
                && child_text(*a, "host-arch").is_none_or(|arch| arch == host_arch())
        })?;
    let complete = child(archive, "complete")?;
    let range = package.range();
    Some(Tool {
        path: path.to_string(),
        name: child_text(package, "display-name")?.to_string(),
        revision: revision(child(package, "revision")?)?,
        url: format!("{REPOSITORY}/{}", child_text(complete, "url")?),
        size: child_text(complete, "size")?.parse().ok()?,
        sha1: child_text(complete, "checksum")?.to_lowercase(),
        licence_id: child(package, "uses-license")?
            .attribute("ref")?
            .to_string(),
        package_xml: xml[range].to_string(),
    })
}

/// This computer's operating system, as Google's list names it.
fn host_os() -> &'static str {
    if cfg!(target_os = "macos") {
        "macosx"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    }
}

/// This computer's processor, as Google's list names it.
fn host_arch() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        "x64"
    }
}

fn list_xml(refresh: bool) -> Result<String> {
    let cache = paths::data_dir().join("cache").join(LIST);
    let fresh = std::fs::metadata(&cache)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_some_and(|age| age < LIST_MAX_AGE);
    if fresh && !refresh {
        if let Ok(xml) = std::fs::read_to_string(&cache) {
            return Ok(xml);
        }
    }
    let fetched = ureq::get(&format!("{REPOSITORY}/{LIST}"))
        .call()
        .map_err(|e| Error::Download(format!("Could not reach Google's list of SDK tools: {e}")))
        .and_then(|mut r| {
            r.body_mut()
                .with_config()
                .limit(20 * 1024 * 1024)
                .read_to_string()
                .map_err(|e| {
                    Error::Download(format!("Google's list of SDK tools didn't download: {e}"))
                })
        });
    match fetched {
        Ok(xml) => {
            if let Some(dir) = cache.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(&cache, &xml);
            Ok(xml)
        }
        // Offline: an old list is better than none.
        Err(e) => std::fs::read_to_string(&cache).map_err(|_| e),
    }
}

/// Downloads and installs a tool, replacing an older revision. Blocks until
/// done, so call it off the async runtime. Its licence must already be
/// accepted, and no device may be running, as the emulator and adb are
/// replaced.
pub fn install(
    sdk: &Sdk,
    tools: &Tools,
    tool: &Tool,
    mut progress: impl FnMut(InstallProgress),
) -> Result<()> {
    let licence = tools
        .licences
        .get(&tool.licence_id)
        .map(String::as_str)
        .unwrap_or("");
    if !catalog::licence_accepted(sdk, &tool.licence_id, licence) {
        return Err(Error::LicenceNotAccepted(tool.licence_id.clone()));
    }
    let temp = sdk.root.join(".temp");
    std::fs::create_dir_all(&temp).context(|| format!("Creating {}", temp.display()))?;
    if let Some(free) = crate::platform::free_space(&temp) {
        // The zip, and the tool unpacked, which is about twice its size.
        let needed = tool.size * 3;
        if free < needed {
            return Err(Error::Download(format!(
                "{} needs about {} of free disk space, and only {} is free.",
                tool.name,
                crate::device::human_size(needed),
                crate::device::human_size(free)
            )));
        }
    }
    let stem = tool.path.replace(';', "-");
    let zip_path = temp.join(format!("aae-{stem}.zip"));
    let unpack = temp.join(format!("aae-{stem}"));
    let old = temp.join(format!("aae-{stem}-old"));
    let dest = tool.install_dir(sdk);
    let result: Result<()> = (|| {
        let archive = Archive {
            url: &tool.url,
            size: tool.size,
            sha1: &tool.sha1,
            what: &tool.name,
        };
        catalog::download(&archive, &zip_path, &mut progress)?;
        progress(InstallProgress::Unpacking);
        let top = catalog::unpack_folder(&zip_path, &unpack)?;
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).context(|| format!("Creating {}", parent.display()))?;
        }
        // Move any older revision aside, so a failed move can put it back.
        let _ = std::fs::remove_dir_all(&old);
        let replacing = dest.exists();
        if replacing {
            std::fs::rename(&dest, &old)
                .context(|| format!("Moving the old {} aside", tool.name))?;
        }
        if let Err(e) = std::fs::rename(&top, &dest) {
            if replacing {
                let _ = std::fs::rename(&old, &dest);
            }
            return Err(e).context(|| format!("Moving {} to {}", tool.name, dest.display()));
        }
        catalog::write_local_package(
            &tool.package_xml,
            ("generic", GENERIC_NAMESPACE),
            &tool.licence_id,
            licence,
            &dest,
        )?;
        let marker = dest.join(MARKER);
        std::fs::write(&marker, format!("{}\n", tool.revision))
            .context(|| format!("Writing {}", marker.display()))
    })();
    let _ = std::fs::remove_file(&zip_path);
    let _ = std::fs::remove_dir_all(&unpack);
    let _ = std::fs::remove_dir_all(&old);
    result
}

/// Whether this computer can run the emulator at full speed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Virtualisation {
    Available,
    /// Not available, with how to make it available.
    Missing(String),
    /// AAE can't tell on this system; the emulator checks when it starts.
    Unknown,
}

/// Checks the computer's hardware virtualisation: Hypervisor.framework on
/// macOS, KVM on Linux, and on Windows, whatever the SDK's emulator finds:
/// Windows Hypervisor Platform or Google's hypervisor driver.
pub fn virtualisation(sdk: &Sdk) -> Virtualisation {
    let _ = sdk;
    #[cfg(target_os = "macos")]
    {
        let mut value: libc::c_int = 0;
        let mut size = std::mem::size_of::<libc::c_int>();
        let name = c"kern.hv_support";
        let found = unsafe {
            libc::sysctlbyname(
                name.as_ptr(),
                (&mut value as *mut libc::c_int).cast(),
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        } == 0;
        if found && value == 1 {
            Virtualisation::Available
        } else {
            Virtualisation::Missing(
                "This Mac doesn't offer hardware virtualisation, which the Android emulator needs. \
                 Macs with Apple silicon always have it; on an Intel Mac it needs a processor with VT-x, \
                 and it isn't available inside another virtual machine."
                    .into(),
            )
        }
    }
    #[cfg(target_os = "linux")]
    {
        let kvm = Path::new("/dev/kvm");
        if !kvm.exists() {
            Virtualisation::Missing(
                "KVM isn't available. Turn on virtualisation (VT-x or AMD-V) in the computer's firmware \
                 settings, and install your distribution's KVM package."
                    .into(),
            )
        } else if std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(kvm)
            .is_err()
        {
            Virtualisation::Missing(
                "You don't have permission to use KVM. Add yourself to the kvm group, then log out and back in."
                    .into(),
            )
        } else {
            Virtualisation::Available
        }
    }
    #[cfg(windows)]
    {
        windows_acceleration(sdk)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        Virtualisation::Unknown
    }
}

/// Asks the emulator whether it can use a hypervisor. It answers between
/// lines saying "accel:" and "accel", with a status, 0 when it can, then
/// what it found.
#[cfg(windows)]
fn windows_acceleration(sdk: &Sdk) -> Virtualisation {
    use crate::platform::NoConsole;
    if crate::platform::windows_on_arm() {
        return Virtualisation::Missing(
            "AAE needs a PC with an Intel or AMD processor: Google's Android emulator doesn't run on Windows on ARM yet."
                .into(),
        );
    }
    let Ok(emulator) = sdk.emulator_bin() else {
        // Not installed yet; it's checked once it is.
        return Virtualisation::Unknown;
    };
    let Ok(out) = std::process::Command::new(emulator)
        .no_console()
        .arg("-accel-check")
        .output()
    else {
        return Virtualisation::Unknown;
    };
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    match parse_accel_check(&text) {
        Some((0, _)) => Virtualisation::Available,
        Some((_, found)) => Virtualisation::Missing(format!(
            "The emulator can't use hardware acceleration: {found} Turn on Windows Hypervisor Platform: in Control Panel, choose Programs, then Turn Windows features on or off, check Windows Hypervisor Platform, and restart. Virtualisation must also be on in the computer's firmware settings."
        )),
        None => Virtualisation::Unknown,
    }
}

/// The status and message from the emulator's `-accel-check`.
#[cfg_attr(not(windows), allow(dead_code))]
fn parse_accel_check(text: &str) -> Option<(i32, String)> {
    let mut lines = text.lines().map(str::trim);
    lines.find(|l| *l == "accel:")?;
    let status = lines.next()?.parse().ok()?;
    let found: Vec<&str> = lines.take_while(|l| *l != "accel").collect();
    let mut found = found.join(" ");
    if !found.is_empty() && !found.ends_with('.') {
        found.push('.');
    }
    Some((status, found))
}

/// True if a path is AAE's own SDK folder, rather than one shared with
/// Android Studio.
pub fn is_own_sdk(path: &Path) -> bool {
    path == paths::sdk_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_emulators_acceleration_check() {
        let usable = "accel:\n0\nWHPX(10.0.26100) is installed and usable.\naccel\n";
        assert_eq!(
            parse_accel_check(usable),
            Some((0, "WHPX(10.0.26100) is installed and usable.".into()))
        );
        let missing = "accel:\n1\nAEHD is not installed\naccel\n";
        assert_eq!(
            parse_accel_check(missing),
            Some((1, "AEHD is not installed.".into()))
        );
        assert_eq!(parse_accel_check("nonsense"), None);
    }

    const XML: &str = r#"<?xml version='1.0' encoding='utf-8'?>
<sdk:sdk-repository xmlns:sdk="http://schemas.android.com/sdk/android/repo/repository2/03" xmlns:generic="http://schemas.android.com/repository/android/generic/02" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
  <license id="android-sdk-license" type="text">Terms</license>
  <channel id="channel-0">stable</channel>
  <channel id="channel-2">dev</channel>
  <remotePackage path="emulator">
    <type-details xsi:type="generic:genericDetailsType"/>
    <revision><major>37</major><minor>3</minor><micro>3</micro></revision>
    <display-name>Android Emulator</display-name>
    <uses-license ref="android-sdk-license"/>
    <channelRef ref="channel-2"/>
    <archives><archive><complete><size>1</size><checksum type="sha1">AA</checksum><url>dev.zip</url></complete><host-os>macosx</host-os><host-arch>aarch64</host-arch></archive></archives>
  </remotePackage>
  <remotePackage path="emulator">
    <type-details xsi:type="generic:genericDetailsType"/>
    <revision><major>37</major><minor>2</minor><micro>12</micro></revision>
    <display-name>Android Emulator</display-name>
    <uses-license ref="android-sdk-license"/>
    <channelRef ref="channel-0"/>
    <archives>
      <archive><complete><size>2</size><checksum type="sha1">BB</checksum><url>emu-x64.zip</url></complete><host-os>macosx</host-os><host-arch>x64</host-arch></archive>
      <archive><complete><size>3</size><checksum type="sha1">CC</checksum><url>emu-arm.zip</url></complete><host-os>macosx</host-os><host-arch>aarch64</host-arch></archive>
      <archive><complete><size>4</size><checksum type="sha1">DD</checksum><url>emu-linux.zip</url></complete><host-os>linux</host-os><host-arch>x64</host-arch></archive>
    </archives>
  </remotePackage>
  <remotePackage path="build-tools;37.0.0-rc1">
    <type-details xsi:type="generic:genericDetailsType"/>
    <revision><major>37</major><minor>0</minor><micro>0</micro><preview>1</preview></revision>
    <display-name>Build-Tools 37 rc1</display-name>
    <uses-license ref="android-sdk-license"/>
    <channelRef ref="channel-0"/>
    <archives><archive><complete><size>5</size><checksum type="sha1">EE</checksum><url>bt-rc.zip</url></complete><host-os>macosx</host-os></archive></archives>
  </remotePackage>
  <remotePackage path="build-tools;36.1.0">
    <type-details xsi:type="generic:genericDetailsType"/>
    <revision><major>36</major><minor>1</minor><micro>0</micro></revision>
    <display-name>Build-Tools 36.1</display-name>
    <uses-license ref="android-sdk-license"/>
    <channelRef ref="channel-0"/>
    <archives><archive><complete><size>6</size><checksum type="sha1">FF</checksum><url>bt.zip</url></complete><host-os>macosx</host-os></archive></archives>
  </remotePackage>
</sdk:sdk-repository>"#;

    #[test]
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    fn updates_only_what_aae_installed() {
        let tools = parse(XML).unwrap();
        let root = std::env::temp_dir().join(format!("aae-test-setup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let sdk = Sdk { root: root.clone() };
        let emulator = root.join("emulator");
        std::fs::create_dir_all(&emulator).unwrap();
        std::fs::write(emulator.join("source.properties"), "Pkg.Revision=36.1.0\n").unwrap();
        // Installed by something else, such as Android Studio: left alone.
        assert!(tools.updates(&sdk).is_empty());
        assert_eq!(tools.managed_elsewhere(&sdk).len(), 1);
        // Installed by AAE: offered the newer revision.
        std::fs::write(emulator.join(MARKER), "36.1.0\n").unwrap();
        let updates: Vec<&str> = tools
            .updates(&sdk)
            .iter()
            .map(|t| t.revision.as_str())
            .collect();
        assert_eq!(updates, vec!["37.2.12"]);
        assert!(tools.managed_elsewhere(&sdk).is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    fn picks_stable_tools_for_this_computer() {
        let tools = parse(XML).unwrap();
        let paths: Vec<&str> = tools.tools.iter().map(|t| t.path.as_str()).collect();
        assert_eq!(paths, vec!["emulator", "build-tools;36.1.0"]);
        let emulator = &tools.tools[0];
        assert_eq!(emulator.revision, "37.2.12");
        assert_eq!(emulator.url, format!("{REPOSITORY}/emu-arm.zip"));
        assert_eq!(emulator.sha1, "cc");
        assert!(
            emulator
                .package_xml
                .starts_with("<remotePackage path=\"emulator\">")
        );
        assert_eq!(tools.licences.get("android-sdk-license").unwrap(), "Terms");
        let sdk = Sdk {
            root: PathBuf::from("/nowhere"),
        };
        assert_eq!(
            tools.tools[1].install_dir(&sdk),
            PathBuf::from("/nowhere/build-tools/36.1.0")
        );
    }
}
