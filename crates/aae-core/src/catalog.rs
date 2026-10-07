//! Downloading Android versions.
//!
//! Google publishes a catalogue of system images for each image type. AAE
//! reads it, lists the versions that run at full speed on this computer, and
//! their updates (such as API 36.1) and previews, and installs one into the
//! SDK just as Google's own sdkmanager
//! would: the same folders, the same `package.xml`, and the same record of
//! accepted licences, so Android Studio sees what AAE installs and the other
//! way round.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use sha1::{Digest, Sha1};

use crate::device::{DeviceStore, dir_size};
use crate::error::{Error, IoContext, Result};
use crate::paths;
use crate::sdk::{
    Release, Sdk, SystemImage, host_abi, image_kind, parse_api_level, parse_properties,
    read_properties,
};

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
    /// Which release of that version: an update such as API 36.1, or a preview.
    pub release: Release,
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
        let mut text = format!("{}, {}", self.release.describe(), image_kind(&self.tag));
        if self.release.page_16k {
            text.push_str(", 16 KB pages");
        }
        text
    }

    /// Where it goes in the SDK: the folders its SDK name gives, such as
    /// system-images/android-36.1/google_apis/arm64-v8a.
    pub fn install_dir(&self, sdk: &Sdk) -> PathBuf {
        self.path
            .split(';')
            .fold(sdk.root.clone(), |dir, part| dir.join(part))
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
        catalogue.tidy();
        Ok(catalogue)
    }

    /// Keeps what's worth offering, newest first: the newest revision of each
    /// image; a release's usual image rather than its 16 KB page one, where
    /// it comes both ways; and of the previews, the latest beta and the
    /// latest canary build of each kind.
    fn tidy(&mut self) {
        let images = &mut self.images;
        images.sort_by(|a, b| {
            a.path
                .cmp(&b.path)
                .then(version_key(&b.revision).cmp(&version_key(&a.revision)))
        });
        images.dedup_by(|later, kept| later.path == kept.path);

        let usual: std::collections::HashSet<(String, String)> = images
            .iter()
            .filter(|i| !i.release.page_16k)
            .map(|i| (i.release.id.clone(), i.tag.clone()))
            .collect();
        images.retain(|i| {
            !i.release.page_16k || !usual.contains(&(i.release.id.clone(), i.tag.clone()))
        });

        let track = |i: &RemoteImage| {
            let preview = i.release.preview.as_deref().unwrap_or_default();
            let kind = if preview.starts_with("Beta") {
                "beta"
            } else {
                "canary"
            };
            (i.tag.clone(), kind)
        };
        // Version, update, revision, and the release's name, for comparing.
        type Rank = (u32, u32, Vec<u32>, String);
        let mut newest: HashMap<(String, &str), Rank> = HashMap::new();
        for image in images.iter().filter(|i| i.release.preview.is_some()) {
            let rank = (
                image.api,
                image.release.minor,
                version_key(&image.revision),
                image.release.id.clone(),
            );
            let best = newest.entry(track(image)).or_insert_with(|| rank.clone());
            if rank > *best {
                *best = rank;
            }
        }
        images.retain(|i| {
            i.release.preview.is_none()
                || newest
                    .get(&track(i))
                    .is_some_and(|best| best.3 == i.release.id)
        });

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
                .then(a.release.page_16k.cmp(&b.release.page_16k))
                .then(a.tag.cmp(&b.tag))
        });
    }

    /// The image with this SDK name, such as
    /// `system-images;android-36.1;google_apis;arm64-v8a`.
    pub fn find(&self, path: &str) -> Option<&RemoteImage> {
        self.images.iter().find(|i| i.path == path)
    }

    /// The image of this kind for a release named as "37", "36.1" or a
    /// preview's folder name, such as "37.2-beta3".
    pub fn find_release(&self, text: &str, tag: &str) -> Option<&RemoteImage> {
        self.images
            .iter()
            .find(|i| i.tag == tag && i.release.matches(text))
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

pub(crate) fn version_key(version: &str) -> Vec<u32> {
    version.split('.').map(|p| p.parse().unwrap_or(0)).collect()
}

pub(crate) fn child<'a, 'i>(
    node: roxmltree::Node<'a, 'i>,
    name: &str,
) -> Option<roxmltree::Node<'a, 'i>> {
    node.children().find(|n| n.has_tag_name(name))
}

pub(crate) fn child_text<'a>(node: roxmltree::Node<'a, '_>, name: &str) -> Option<&'a str> {
    child(node, name).and_then(|n| n.text()).map(str::trim)
}

pub(crate) fn revision(node: roxmltree::Node) -> Option<String> {
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
    // "36" for a version, "36.1" for a later update to it.
    let level = child_text(details, "api-level")?;
    let (api, _) = parse_api_level(level)?;
    let abi = child_text(details, "abi")?;
    let tags: Vec<&str> = details
        .children()
        .filter(|n| n.has_tag_name("tag"))
        .filter_map(|t| child_text(t, "id"))
        .collect();
    let image_tag = *tags.first()?;
    if api < MIN_API || abi != host_abi() || image_tag != tag {
        return None;
    }
    let folder = path.split(';').nth(1)?;
    let page_16k = tags.contains(&"page_size_16kb")
        || path
            .split(';')
            .nth(2)
            .is_some_and(|t| t.ends_with("_ps16k"));
    let release = Release::read(
        level,
        folder,
        child(details, "codename").is_some(),
        page_16k,
    )?;
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
        release,
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
    if let Some(free) = crate::platform::free_space(&temp) {
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
        let what = image.describe();
        let archive = Archive {
            url: &image.url,
            size: image.size,
            sha1: &image.sha1,
            what: &what,
        };
        download(&archive, &zip_path, &mut progress)?;
        progress(InstallProgress::Unpacking);
        // The archive holds one folder, named after the processor type.
        let top = unpack_folder(&zip_path, &unpack)?;
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

/// A file to download: where from, how big, and its SHA-1 checksum.
pub(crate) struct Archive<'a> {
    pub url: &'a str,
    pub size: u64,
    pub sha1: &'a str,
    /// What it is, for messages, such as "Android 15 (API 35)".
    pub what: &'a str,
}

/// Downloads a file, checking it against its checksum.
pub(crate) fn download(
    archive: &Archive,
    path: &Path,
    progress: &mut impl FnMut(InstallProgress),
) -> Result<()> {
    let what = archive.what;
    let mut response = ureq::get(archive.url)
        .call()
        .map_err(|e| Error::Download(format!("Could not download {what}: {e}")))?;
    let mut reader = response
        .body_mut()
        .with_config()
        .limit(archive.size + 1024 * 1024)
        .reader();
    let mut file =
        std::fs::File::create(path).context(|| format!("Creating {}", path.display()))?;
    let mut hasher = Sha1::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    let mut done = 0u64;
    progress(InstallProgress::Downloading {
        done,
        total: archive.size,
    });
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|e| Error::Download(format!("The download of {what} stopped: {e}")))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        file.write_all(&buffer[..read])
            .context(|| format!("Writing {}", path.display()))?;
        done += read as u64;
        progress(InstallProgress::Downloading {
            done,
            total: archive.size,
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
    if actual != archive.sha1 {
        return Err(Error::Download(format!(
            "The download of {what} was damaged on the way, so it wasn't installed. Try again."
        )));
    }
    Ok(())
}

/// Unpacks a downloaded archive that holds one folder, and returns that folder.
pub(crate) fn unpack_folder(zip_path: &Path, unpack: &Path) -> Result<PathBuf> {
    let _ = std::fs::remove_dir_all(unpack);
    let file =
        std::fs::File::open(zip_path).context(|| format!("Opening {}", zip_path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| Error::Download(format!("The download is not a valid archive: {e}")))?;
    archive
        .extract(unpack)
        .map_err(|e| Error::Download(format!("The download couldn't be unpacked: {e}")))?;
    std::fs::read_dir(unpack)
        .context(|| format!("Reading {}", unpack.display()))?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.is_dir())
        .ok_or_else(|| Error::Download("The download was empty.".into()))
}

/// Writes the `package.xml` Android Studio and sdkmanager expect in an
/// installed package: the catalogue's entry, as a local package, with its licence.
fn write_package_xml(image: &RemoteImage, licence: &str, dir: &Path) -> Result<()> {
    write_local_package(
        &image.package_xml,
        ("sys-img", &image.sys_img_namespace),
        &image.licence_id,
        licence,
        dir,
    )
}

/// Writes `package.xml` for an installed package from its catalogue entry
/// (`<remotePackage>…</remotePackage>`), declaring the namespace its type
/// details use, such as ("generic", "http://…/generic/02").
pub(crate) fn write_local_package(
    remote_entry: &str,
    namespace: (&str, &str),
    licence_id: &str,
    licence: &str,
    dir: &Path,
) -> Result<()> {
    let mut entry = remote_entry.replacen("<remotePackage", "<localPackage", 1);
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
    let (prefix, uri) = namespace;
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <ns2:repository xmlns:ns2=\"http://schemas.android.com/repository/android/common/02\" \
         xmlns:{prefix}=\"{uri}\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\n\
         <license id=\"{licence_id}\" type=\"text\">{licence}</license>\n{entry}\n</ns2:repository>\n"
    );
    let path = dir.join("package.xml");
    std::fs::write(&path, xml).context(|| format!("Writing {}", path.display()))
}

/// The devices that use an installed Android version.
#[derive(Debug, Clone, Default)]
pub struct ImageUsers {
    /// AAE's devices made from it. It can't be deleted while there are any.
    pub devices: Vec<String>,
    /// Other emulator devices, such as Android Studio's, made from it. They
    /// won't start once it's deleted.
    pub others: Vec<String>,
}

/// Which devices use an installed Android version.
pub fn image_users(sdk: &Sdk, store: &DeviceStore, image: &SystemImage) -> Result<ImageUsers> {
    let devices = store
        .list()?
        .into_iter()
        .filter(|d| same_sysdir(&d.meta.sysdir, &image.sysdir))
        .map(|d| d.meta.name)
        .collect();
    // Android Studio's devices name images relative to its own SDK. When AAE
    // has its own SDK, theirs are other copies.
    let others = if sdk.root == paths::sdk_dir() {
        Vec::new()
    } else {
        other_devices()
            .into_iter()
            .filter(|(_, sysdir)| same_sysdir(sysdir, &image.sysdir))
            .map(|(name, _)| name)
            .collect()
    };
    Ok(ImageUsers { devices, others })
}

/// Deletes an installed Android version from the SDK, refusing while any of
/// AAE's devices use it. Returns the bytes freed.
pub fn remove(sdk: &Sdk, store: &DeviceStore, image: &SystemImage) -> Result<u64> {
    let users = image_users(sdk, store, image)?;
    if !users.devices.is_empty() {
        return Err(Error::ImageInUse(
            image.to_string(),
            users.devices.join(", "),
        ));
    }
    let size = dir_size(&image.path);
    std::fs::remove_dir_all(&image.path)
        .context(|| format!("Deleting {}", image.path.display()))?;
    // Tidy the folders that held only this image, as sdkmanager does.
    let top = sdk.root.join("system-images");
    let mut dir = image.path.parent();
    while let Some(parent) = dir {
        if parent == top || !parent.starts_with(&top) || std::fs::remove_dir(parent).is_err() {
            break;
        }
        dir = parent.parent();
    }
    Ok(size)
}

/// The disk space an installed Android version uses.
pub fn image_size(image: &SystemImage) -> u64 {
    dir_size(&image.path)
}

fn same_sysdir(a: &str, b: &str) -> bool {
    let tidy = |s: &str| s.replace('\\', "/").trim_end_matches('/').to_string();
    tidy(a) == tidy(b)
}

/// Other emulator devices on this computer, such as Android Studio's, as
/// (name, image folder) pairs.
fn other_devices() -> Vec<(String, String)> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(dir) = std::env::var_os("ANDROID_AVD_HOME") {
        dirs.push(dir.into());
    }
    if let Some(dir) = std::env::var_os("ANDROID_USER_HOME") {
        dirs.push(PathBuf::from(dir).join("avd"));
    }
    for var in ["ANDROID_PREFS_ROOT", "ANDROID_SDK_HOME"] {
        if let Some(dir) = std::env::var_os(var) {
            dirs.push(PathBuf::from(dir).join(".android/avd"));
        }
    }
    if let Some(home) = directories::BaseDirs::new() {
        dirs.push(home.home_dir().join(".android/avd"));
    }
    let ours = paths::devices_dir();
    dirs.retain(|d| *d != ours);
    dirs.dedup();
    let mut found = Vec::new();
    for dir in dirs {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let ini = entry.path();
            if ini.extension().is_none_or(|e| e != "ini") {
                continue;
            }
            let Ok(props) = read_properties(&ini) else {
                continue;
            };
            let avd = props
                .get("path")
                .map(PathBuf::from)
                .filter(|p| p.is_dir())
                .unwrap_or_else(|| ini.with_extension("avd"));
            let Ok(text) = std::fs::read_to_string(avd.join("config.ini")) else {
                continue;
            };
            let config = parse_properties(&text);
            let Some(sysdir) = config.get("image.sysdir.1") else {
                continue;
            };
            let name = config
                .get("avd.ini.displayname")
                .cloned()
                .unwrap_or_else(|| {
                    ini.file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default()
                });
            if !found.iter().any(|(n, _): &(String, String)| *n == name) {
                found.push((name, sysdir.clone()));
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_images_no_device_uses() {
        let root = std::env::temp_dir().join(format!("aae-test-remove-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("sdk/system-images/android-35/google_apis/arm64-v8a");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("source.properties"),
            "AndroidVersion.ApiLevel=35\nSystemImage.TagId=google_apis\nSystemImage.Abi=arm64-v8a\n",
        )
        .unwrap();
        std::fs::write(dir.join("system.img"), vec![1u8; 8192]).unwrap();
        let sdk = Sdk {
            root: root.join("sdk"),
        };
        let image = sdk.system_images().remove(0);
        let store = DeviceStore::open(root.join("devices")).unwrap();
        let device = store
            .create("Uses it", &image, crate::device::Profile::Phone)
            .unwrap();

        let users = image_users(&sdk, &store, &image).unwrap();
        assert_eq!(users.devices, vec!["Uses it"]);
        assert!(matches!(
            remove(&sdk, &store, &image),
            Err(Error::ImageInUse(..))
        ));
        assert!(dir.exists());

        store.delete(&device).unwrap();
        assert!(remove(&sdk, &store, &image).unwrap() >= 8192);
        assert!(!root.join("sdk/system-images/android-35").exists());
        assert!(root.join("sdk/system-images").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn compares_image_folders() {
        assert!(same_sysdir(
            "system-images/android-30/google_apis_playstore/arm64-v8a/",
            "system-images\\android-30\\google_apis_playstore\\arm64-v8a"
        ));
        assert!(!same_sysdir(
            "system-images/android-30/default/arm64-v8a/",
            "system-images/android-30/google_apis/arm64-v8a/"
        ));
    }

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

#[cfg(test)]
mod release_tests {
    use super::*;

    fn package(
        path: &str,
        level: &str,
        codename: Option<&str>,
        page_16k: bool,
        revision: u32,
    ) -> String {
        let codename = codename
            .map(|c| format!("<codename>{c}</codename>"))
            .unwrap_or_default();
        let page = if page_16k {
            "<tag><id>page_size_16kb</id></tag>"
        } else {
            ""
        };
        format!(
            r#"<remotePackage path="{path}">
    <type-details xsi:type="sys-img:sysImgDetailsType"><api-level>{level}</api-level>{codename}<tag><id>google_apis</id></tag>{page}<abi>{abi}</abi></type-details>
    <revision><major>{revision}</major></revision><uses-license ref="l"/>
    <archives><archive><complete><size>1</size><checksum type="sha1">aa</checksum><url>x.zip</url></complete></archive></archives>
  </remotePackage>"#,
            abi = host_abi()
        )
    }

    fn catalogue() -> Catalogue {
        let abi = host_abi();
        let packages = [
            package(
                &format!("system-images;android-36;google_apis;{abi}"),
                "36",
                None,
                false,
                7,
            ),
            package(
                &format!("system-images;android-36.1;google_apis;{abi}"),
                "36.1",
                None,
                false,
                3,
            ),
            package(
                &format!("system-images;android-37.0;google_apis;{abi}"),
                "37.0",
                None,
                false,
                6,
            ),
            package(
                &format!("system-images;android-37.0;google_apis_ps16k;{abi}"),
                "37.0",
                None,
                true,
                7,
            ),
            package(
                &format!("system-images;android-37.1;google_apis_ps16k;{abi}"),
                "37.1",
                None,
                true,
                9,
            ),
            package(
                &format!("system-images;android-37.2-beta2;google_apis_ps16k;{abi}"),
                "37.1",
                Some("DEV"),
                true,
                2,
            ),
            package(
                &format!("system-images;android-37.2-beta3;google_apis_ps16k;{abi}"),
                "37.1",
                Some("DEV"),
                true,
                3,
            ),
            package(
                &format!("system-images;android-CANARY;google_apis_ps16k;{abi}"),
                "37.1",
                Some("CANARY"),
                true,
                15,
            ),
            package(
                &format!("system-images;android-canary-20260909;google_apis_ps16k;{abi}"),
                "37.2",
                Some("CANARY"),
                true,
                16,
            ),
        ];
        let xml = format!(
            r#"<?xml version='1.0' encoding='utf-8'?>
<sys-img:sdk-sys-img xmlns:sys-img="http://schemas.android.com/sdk/android/repo/sys-img2/04" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
  <license id="l" type="text">Terms</license>
  <channel id="channel-0">stable</channel>
  {}
</sys-img:sdk-sys-img>"#,
            packages.join("\n  ")
        );
        let mut catalogue = Catalogue::default();
        catalogue.add("google_apis", &xml).unwrap();
        catalogue.tidy();
        catalogue
    }

    #[test]
    fn offers_updates_and_newer_versions_with_decimal_api_levels() {
        let names: Vec<String> = catalogue()
            .images
            .iter()
            .map(RemoteImage::describe)
            .collect();
        assert_eq!(
            names,
            [
                "Android 17 (API 37.1), With Google services, 16 KB pages",
                "Android 17 (API 37), With Google services",
                "Android 17 Canary preview, 9 September 2026, With Google services, 16 KB pages",
                "Android 17 Beta 3 preview, With Google services, 16 KB pages",
                "Android 16 (API 36.1), With Google services",
                "Android 16 (API 36), With Google services",
            ]
        );
    }

    #[test]
    fn finds_releases_by_name() {
        let catalogue = catalogue();
        let abi = host_abi();
        assert_eq!(
            catalogue.find_release("37", "google_apis").unwrap().path,
            format!("system-images;android-37.0;google_apis;{abi}")
        );
        assert_eq!(
            catalogue
                .find_release("36.1", "google_apis")
                .unwrap()
                .release
                .minor,
            1
        );
        assert_eq!(
            catalogue
                .find_release("37.2-beta3", "google_apis")
                .unwrap()
                .release
                .preview
                .as_deref(),
            Some("Beta 3")
        );
        assert!(
            catalogue
                .find_release("37.2-beta2", "google_apis")
                .is_none()
        );
        let image = catalogue.find_release("36.1", "google_apis").unwrap();
        assert!(
            image
                .install_dir(&Sdk {
                    root: "/sdk".into()
                })
                .ends_with(format!("android-36.1/google_apis/{abi}"))
        );
    }
}
