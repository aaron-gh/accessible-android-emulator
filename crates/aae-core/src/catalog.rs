//! Downloading Android versions.
//!
//! Google publishes a catalogue of system images for each image type. AAE
//! reads it, lists the stable versions that run at full speed on this
//! computer, and installs one into the SDK just as Google's own sdkmanager
//! would: the same folders, the same `package.xml`, and the same record of
//! accepted licences, so Android Studio sees what AAE installs and the other
//! way round.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use sha1::{Digest, Sha1};

use crate::error::{Error, IoContext, Result};
use crate::paths;
use crate::sdk::{Sdk, SystemImage, android_name, host_abi, image_kind};

const REPOSITORY: &str = "https://dl.google.com/android/repository/sys-img";
/// The image types AAE offers: plain Android, with Google services, and with Google Play.
const TAGS: &[&str] = &["default", "google_apis", "google_apis_playstore"];
/// The catalogue file's folder for each tag, when it differs from the tag.
fn catalogue_folder(tag: &str) -> &str {
    if tag == "default" { "android" } else { tag }
}
/// The oldest Android version AAE supports.
const MIN_API: u32 = 21;
/// How long a downloaded catalogue is used before fetching it again.
const CATALOGUE_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);

/// A system image that can be downloaded.
#[derive(Debug, Clone)]
pub struct RemoteImage {
    pub api: u32,
    pub tag: String,
    pub abi: String,
    /// The SDK's name for it, such as `system-images;android-35;google_apis;arm64-v8a`.
    pub path: String,
    pub revision: String,
    pub url: String,
    /// Download size in bytes.
    pub size: u64,
    pub sha1: String,
    pub licence_id: String,
    /// The lowest emulator version it needs, such as "34.2.16".
    pub min_emulator: Option<String>,
    /// The package entry as the catalogue has it, for writing `package.xml`.
    package_xml: String,
    /// The namespace of the catalogue's `sys-img` prefix.
    sys_img_namespace: String,
}

impl RemoteImage {
    /// Such as "Android 15 (API 35), With Google Play".
    pub fn describe(&self) -> String {
        format!("{}, {}", android_name(self.api), image_kind(&self.tag))
    }

    /// Where it goes in the SDK.
    pub fn install_dir(&self, sdk: &Sdk) -> PathBuf {
        sdk.root
            .join("system-images")
            .join(format!("android-{}", self.api))
            .join(&self.tag)
            .join(&self.abi)
    }

    pub fn is_installed(&self, sdk: &Sdk) -> bool {
        self.install_dir(sdk).join("source.properties").is_file()
    }
}

/// The downloadable images, and the licences they are under.
#[derive(Debug, Clone, Default)]
pub struct Catalogue {
    pub images: Vec<RemoteImage>,
    /// Licence text by licence id.
    pub licences: HashMap<String, String>,
}

impl Catalogue {
    /// Reads the catalogue, from AAE's copy if it is less than a day old, or
    /// from Google. Falls back to an older copy when offline.
    pub fn load(refresh: bool) -> Result<Self> {
        let mut catalogue = Catalogue::default();
        for tag in TAGS {
            let xml = catalogue_xml(tag, refresh)?;
            catalogue.add(tag, &xml)?;
        }
        catalogue.images.sort_by(|a, b| {
            b.api
                .cmp(&a.api)
                .then(a.tag.cmp(&b.tag))
                .then(version_key(&b.revision).cmp(&version_key(&a.revision)))
        });
        // Keep only the newest revision of each version and kind.
        catalogue
            .images
            .dedup_by(|later, kept| later.api == kept.api && later.tag == kept.tag);
        Ok(catalogue)
    }

    /// The image for an API level and image type, if Google offers one for this computer.
    pub fn find(&self, api: u32, tag: &str) -> Option<&RemoteImage> {
        self.images.iter().find(|i| i.api == api && i.tag == tag)
    }

    fn add(&mut self, tag: &str, xml: &str) -> Result<()> {
        let parse_error = |reason: String| Error::Config {
            path: PathBuf::from(format!(
                "{REPOSITORY}/{}/sys-img2-4.xml",
                catalogue_folder(tag)
            )),
            reason,
        };
        let doc = roxmltree::Document::parse(xml).map_err(|e| parse_error(e.to_string()))?;
        let root = doc.root_element();
        let sys_img_namespace = root
            .lookup_namespace_uri(Some("sys-img"))
            .unwrap_or_default()
            .to_string();
        let stable: Vec<String> = root
            .children()
            .filter(|n| n.has_tag_name("channel") && n.text() == Some("stable"))
            .filter_map(|n| n.attribute("id").map(String::from))
            .collect();
        for licence in root.children().filter(|n| n.has_tag_name("license")) {
            if let (Some(id), Some(text)) = (licence.attribute("id"), licence.text()) {
                self.licences.insert(id.to_string(), text.to_string());
            }
        }
        for package in root.children().filter(|n| n.has_tag_name("remotePackage")) {
            if let Some(image) = read_package(
                package,
                &xml[package.range()],
                &stable,
                tag,
                &sys_img_namespace,
            ) {
                self.images.push(image);
            }
        }
        Ok(())
    }
}

fn version_key(version: &str) -> Vec<u32> {
    version.split('.').map(|p| p.parse().unwrap_or(0)).collect()
}

fn child<'a, 'i>(node: roxmltree::Node<'a, 'i>, name: &str) -> Option<roxmltree::Node<'a, 'i>> {
    node.children().find(|n| n.has_tag_name(name))
}

fn child_text<'a>(node: roxmltree::Node<'a, '_>, name: &str) -> Option<&'a str> {
    child(node, name).and_then(|n| n.text()).map(str::trim)
}

fn revision(node: roxmltree::Node) -> Option<String> {
    let parts: Vec<&str> = ["major", "minor", "micro"]
        .iter()
        .filter_map(|p| child_text(node, p))
        .collect();
    (!parts.is_empty()).then(|| parts.join("."))
}

fn read_package(
    package: roxmltree::Node,
    xml: &str,
    stable: &[String],
    tag: &str,
    sys_img_namespace: &str,
) -> Option<RemoteImage> {
    let path = package.attribute("path")?;
    // Extension images (such as android-35-ext14) are variants of a version;
    // AAE offers the base one.
    if path.contains("-ext") {
        return None;
    }
    let channel = child(package, "channelRef")
        .and_then(|c| c.attribute("ref"))
        .unwrap_or("channel-0");
    if !stable.iter().any(|s| s == channel) {
        return None;
    }
    let details = child(package, "type-details")?;
    let api: u32 = child_text(details, "api-level")?.parse().ok()?;
    let abi = child_text(details, "abi")?;
    let image_tag = child(details, "tag").and_then(|t| child_text(t, "id"))?;
    if api < MIN_API || abi != host_abi() || image_tag != tag {
        return None;
    }
    let archive = child(child(package, "archives")?, "archive")?;
    let complete = child(archive, "complete")?;
    let min_emulator = child(package, "dependencies")
        .into_iter()
        .flat_map(|d| d.children())
        .find(|d| d.attribute("path") == Some("emulator"))
        .and_then(|d| child(d, "min-revision"))
        .and_then(revision);
    Some(RemoteImage {
        api,
        tag: tag.to_string(),
        abi: abi.to_string(),
        path: path.to_string(),
        revision: revision(child(package, "revision")?)?,
        url: format!(
            "{REPOSITORY}/{}/{}",
            catalogue_folder(tag),
            child_text(complete, "url")?
        ),
        size: child_text(complete, "size")?.parse().ok()?,
        sha1: child_text(complete, "checksum")?.to_lowercase(),
        licence_id: child(package, "uses-license")
            .and_then(|l| l.attribute("ref"))?
            .to_string(),
        min_emulator,
        package_xml: xml.to_string(),
        sys_img_namespace: sys_img_namespace.to_string(),
    })
}

fn catalogue_xml(tag: &str, refresh: bool) -> Result<String> {
    let cache = paths::data_dir()
        .join("cache")
        .join(format!("sys-img-{tag}.xml"));
    let fresh = std::fs::metadata(&cache)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_some_and(|age| age < CATALOGUE_MAX_AGE);
    if fresh && !refresh {
        if let Ok(xml) = std::fs::read_to_string(&cache) {
            return Ok(xml);
        }
    }
    let url = format!("{REPOSITORY}/{}/sys-img2-4.xml", catalogue_folder(tag));
    match fetch_text(&url) {
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

fn fetch_text(url: &str) -> Result<String> {
    ureq::get(url)
        .call()
        .map_err(|e| {
            Error::Download(format!(
                "Could not reach Google's list of Android versions: {e}"
            ))
        })?
        .body_mut()
        .read_to_string()
        .map_err(|e| {
            Error::Download(format!(
                "Google's list of Android versions didn't download: {e}"
            ))
        })
}

// Licences.

/// True if the licence's current text has been accepted in this SDK, by AAE,
/// Android Studio or sdkmanager. The SDK records each accepted text as the
/// SHA-1 of the text, one per line, in `licenses/<id>`.
pub fn licence_accepted(sdk: &Sdk, id: &str, text: &str) -> bool {
    let hash = sha1_hex(text.as_bytes());
    std::fs::read_to_string(sdk.root.join("licenses").join(id))
        .is_ok_and(|file| file.lines().any(|line| line.trim() == hash))
}

/// Records that the user accepted the licence's current text.
pub fn accept_licence(sdk: &Sdk, id: &str, text: &str) -> Result<()> {
    if licence_accepted(sdk, id, text) {
        return Ok(());
    }
    let dir = sdk.root.join("licenses");
    std::fs::create_dir_all(&dir).context(|| format!("Creating {}", dir.display()))?;
    let path = dir.join(id);
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .context(|| format!("Opening {}", path.display()))?;
    writeln!(file, "\n{}", sha1_hex(text.as_bytes()))
        .context(|| format!("Writing {}", path.display()))
}

fn sha1_hex(bytes: &[u8]) -> String {
    Sha1::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

// Installing.

/// Progress while installing an image.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InstallProgress {
    /// Bytes downloaded so far, and the total.
    Downloading {
        done: u64,
        total: u64,
    },
    Verifying,
    Unpacking,
}

impl InstallProgress {
    /// The percentage downloaded, while downloading.
    pub fn percent(&self) -> Option<u32> {
        match self {
            InstallProgress::Downloading { done, total } if *total > 0 => {
                Some((done * 100 / total) as u32)
            }
            _ => None,
        }
    }
}

/// Downloads and installs an image. Blocks until done, so call it off the
/// async runtime. The licence must already be accepted.
pub fn install(
    sdk: &Sdk,
    catalogue: &Catalogue,
    image: &RemoteImage,
    mut progress: impl FnMut(InstallProgress),
) -> Result<SystemImage> {
    let licence = catalogue
        .licences
        .get(&image.licence_id)
        .map(String::as_str)
        .unwrap_or("");
    if !licence_accepted(sdk, &image.licence_id, licence) {
        return Err(Error::LicenceNotAccepted(image.licence_id.clone()));
    }
    let dest = image.install_dir(sdk);
    if image.is_installed(sdk) {
        return SystemImage::read_dir(&sdk.root, &dest).ok_or_else(|| {
            Error::Download(format!(
                "{} is installed but can't be read.",
                image.describe()
            ))
        });
    }
    let temp = sdk.root.join(".temp");
    std::fs::create_dir_all(&temp).context(|| format!("Creating {}", temp.display()))?;
    if let Some(free) = free_space(&temp) {
        // The zip, and the image unpacked, which is about twice its size.
        let needed = image.size * 3;
        if free < needed {
            return Err(Error::Download(format!(
                "{} needs about {} of free disk space, and only {} is free.",
                image.describe(),
                crate::device::human_size(needed),
                crate::device::human_size(free)
            )));
        }
    }

    let zip_path = temp.join(format!("aae-{}-{}-{}.zip", image.api, image.tag, image.abi));
    let unpack = temp.join(format!("aae-{}-{}-{}", image.api, image.tag, image.abi));
    let result: Result<()> = (|| {
        download(image, &zip_path, &mut progress)?;
        progress(InstallProgress::Unpacking);
        let _ = std::fs::remove_dir_all(&unpack);
        let file =
            std::fs::File::open(&zip_path).context(|| format!("Opening {}", zip_path.display()))?;
        let mut archive = zip::ZipArchive::new(file)
            .map_err(|e| Error::Download(format!("The download is not a valid archive: {e}")))?;
        archive
            .extract(&unpack)
            .map_err(|e| Error::Download(format!("The download couldn't be unpacked: {e}")))?;
        // The archive holds one folder, named after the processor type.
        let top = std::fs::read_dir(&unpack)
            .context(|| format!("Reading {}", unpack.display()))?
            .flatten()
            .map(|e| e.path())
            .find(|p| p.is_dir())
            .ok_or_else(|| Error::Download("The download was empty.".into()))?;
        let _ = std::fs::remove_dir_all(&dest);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).context(|| format!("Creating {}", parent.display()))?;
        }
        std::fs::rename(&top, &dest)
            .context(|| format!("Moving the image to {}", dest.display()))?;
        write_package_xml(image, licence, &dest)?;
        Ok(())
    })();
    let _ = std::fs::remove_file(&zip_path);
    let _ = std::fs::remove_dir_all(&unpack);
    result?;
    SystemImage::read_dir(&sdk.root, &dest).ok_or_else(|| {
        Error::Download(format!(
            "{} was installed but can't be read.",
            image.describe()
        ))
    })
}

fn download(
    image: &RemoteImage,
    path: &Path,
    progress: &mut impl FnMut(InstallProgress),
) -> Result<()> {
    let mut response = ureq::get(&image.url)
        .call()
        .map_err(|e| Error::Download(format!("Could not download {}: {e}", image.describe())))?;
    let mut reader = response
        .body_mut()
        .with_config()
        .limit(image.size + 1024 * 1024)
        .reader();
    let mut file =
        std::fs::File::create(path).context(|| format!("Creating {}", path.display()))?;
    let mut hasher = Sha1::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    let mut done = 0u64;
    progress(InstallProgress::Downloading {
        done,
        total: image.size,
    });
    loop {
        let read = reader.read(&mut buffer).map_err(|e| {
            Error::Download(format!("The download of {} stopped: {e}", image.describe()))
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        file.write_all(&buffer[..read])
            .context(|| format!("Writing {}", path.display()))?;
        done += read as u64;
        progress(InstallProgress::Downloading {
            done,
            total: image.size,
        });
    }
    file.flush()
        .context(|| format!("Writing {}", path.display()))?;
    progress(InstallProgress::Verifying);
    let actual: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if actual != image.sha1 {
        return Err(Error::Download(format!(
            "The download of {} was damaged on the way, so it wasn't installed. Try again.",
            image.describe()
        )));
    }
    Ok(())
}

/// Writes the `package.xml` Android Studio and sdkmanager expect in an
/// installed package: the catalogue's entry, as a local package, with its licence.
fn write_package_xml(image: &RemoteImage, licence: &str, dir: &Path) -> Result<()> {
    let mut entry = image
        .package_xml
        .replacen("<remotePackage", "<localPackage", 1);
    entry = entry.replace("</remotePackage>", "</localPackage>");
    // The download details belong to the catalogue, not an installed package.
    if let (Some(start), Some(end)) = (entry.find("<archives>"), entry.find("</archives>")) {
        entry.replace_range(start..end + "</archives>".len(), "");
    }
    if let Some(start) = entry.find("<channelRef") {
        if let Some(len) = entry[start..].find("/>") {
            entry.replace_range(start..start + len + 2, "");
        }
    }
    let licence = licence
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <ns2:repository xmlns:ns2=\"http://schemas.android.com/repository/android/common/02\" \
         xmlns:sys-img=\"{}\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\n\
         <license id=\"{}\" type=\"text\">{licence}</license>\n{entry}\n</ns2:repository>\n",
        image.sys_img_namespace, image.licence_id
    );
    let path = dir.join("package.xml");
    std::fs::write(&path, xml).context(|| format!("Writing {}", path.display()))
}

/// Free space on the disk holding `path`, in bytes.
fn free_space(path: &Path) -> Option<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
        let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) } != 0 {
            return None;
        }
        Some(stat.f_bavail as u64 * stat.f_frsize as u64)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const XML: &str = r#"<?xml version='1.0' encoding='utf-8'?>
<sys-img:sdk-sys-img xmlns:sys-img="http://schemas.android.com/sdk/android/repo/sys-img2/04" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
  <license id="android-sdk-arm-dbt-license" type="text">Terms</license>
  <channel id="channel-0">stable</channel>
  <channel id="channel-3">canary</channel>
  <remotePackage path="system-images;android-34;google_apis;arm64-v8a">
    <type-details xsi:type="sys-img:sysImgDetailsType">
      <api-level>34</api-level>
      <tag><id>google_apis</id><display>Google APIs</display></tag>
      <abi>arm64-v8a</abi>
    </type-details>
    <revision><major>14</major></revision>
    <uses-license ref="android-sdk-arm-dbt-license"/>
    <dependencies><dependency path="emulator"><min-revision><major>34</major><minor>2</minor></min-revision></dependency></dependencies>
    <channelRef ref="channel-0"/>
    <archives><archive><complete><size>1610393229</size><checksum type="sha1">2FE8B46D</checksum><url>arm64-v8a-34_r14.zip</url></complete></archive></archives>
  </remotePackage>
  <remotePackage path="system-images;android-35-ext14;google_apis;arm64-v8a">
    <type-details xsi:type="sys-img:sysImgDetailsType"><api-level>35</api-level><tag><id>google_apis</id></tag><abi>arm64-v8a</abi></type-details>
    <revision><major>1</major></revision><uses-license ref="android-sdk-arm-dbt-license"/><channelRef ref="channel-0"/>
    <archives><archive><complete><size>1</size><checksum type="sha1">aa</checksum><url>x.zip</url></complete></archive></archives>
  </remotePackage>
  <remotePackage path="system-images;android-36;google_apis;arm64-v8a">
    <type-details xsi:type="sys-img:sysImgDetailsType"><api-level>36</api-level><tag><id>google_apis</id></tag><abi>arm64-v8a</abi></type-details>
    <revision><major>1</major></revision><uses-license ref="android-sdk-arm-dbt-license"/><channelRef ref="channel-3"/>
    <archives><archive><complete><size>1</size><checksum type="sha1">aa</checksum><url>x.zip</url></complete></archive></archives>
  </remotePackage>
</sys-img:sdk-sys-img>"#;

    #[test]
    fn reads_stable_base_images() {
        let mut catalogue = Catalogue::default();
        catalogue.add("google_apis", XML).unwrap();
        if host_abi() != "arm64-v8a" {
            assert!(catalogue.images.is_empty());
            return;
        }
        assert_eq!(
            catalogue.images.len(),
            1,
            "extension and canary images are left out"
        );
        let image = &catalogue.images[0];
        assert_eq!(image.api, 34);
        assert_eq!(image.revision, "14");
        assert_eq!(image.sha1, "2fe8b46d");
        assert_eq!(image.min_emulator.as_deref(), Some("34.2"));
        assert_eq!(
            image.url,
            "https://dl.google.com/android/repository/sys-img/google_apis/arm64-v8a-34_r14.zip"
        );
        assert_eq!(catalogue.licences["android-sdk-arm-dbt-license"], "Terms");
    }

    #[test]
    fn records_and_checks_licences() {
        let root = std::env::temp_dir().join(format!("aae-licences-{}", std::process::id()));
        let sdk = Sdk { root: root.clone() };
        assert!(!licence_accepted(&sdk, "x", "Terms"));
        accept_licence(&sdk, "x", "Terms").unwrap();
        assert!(licence_accepted(&sdk, "x", "Terms"));
        assert!(!licence_accepted(&sdk, "x", "New terms"));
        let _ = std::fs::remove_dir_all(root);
    }
}
