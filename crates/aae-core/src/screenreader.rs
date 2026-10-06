//! The screen reader AAE installs when none is chosen: Backtalk, from its
//! project's development builds.
//!
//! Backtalk publishes each build as `backtalk.apk` in its "dev" release on
//! GitHub, replacing the previous one. AAE keeps a copy, checks at most once a
//! day whether a newer build is out, and only keeps a download that is signed
//! with Backtalk's own key.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

use crate::error::{Error, IoContext, Result};
use crate::paths;
use crate::sdk::Sdk;

const RELEASE: &str = "https://api.github.com/repos/trypsynth/backtalk/releases/tags/dev";
const ASSET: &str = "backtalk.apk";
/// SHA-256 of the certificate Backtalk's builds are signed with.
const CERTIFICATE_SHA256: &str = "8ddba1d5edf9b7470850b19f756e4755b6a4d99ee793d5d9b608918a91ba0eb0";
/// Backtalk needs Android 8.0 or later.
pub const BACKTALK_MIN_API: u32 = 26;
const CHECK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug, Default, Serialize, Deserialize)]
struct CacheInfo {
    /// When the release asset we have was published, as GitHub reports it.
    updated_at: String,
}

#[derive(Deserialize)]
struct Release {
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    updated_at: String,
    browser_download_url: String,
    size: u64,
}

/// Backtalk's latest development build, downloading it if AAE has none or a
/// newer one is out. Works offline from the copy AAE has. Blocks; call it off
/// the async runtime.
pub fn backtalk_apk(sdk: &Sdk) -> Result<PathBuf> {
    let dir = paths::data_dir().join("screen-readers");
    let apk = dir.join("backtalk-dev.apk");
    let info_path = dir.join("backtalk-dev.toml");
    let have = apk.is_file();
    let checked_recently = std::fs::metadata(&info_path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_some_and(|age| age < CHECK_EVERY);
    if have && checked_recently {
        return Ok(apk);
    }

    let latest = match latest_asset() {
        Ok(asset) => asset,
        Err(e) if have => {
            tracing::warn!("couldn't check for a newer Backtalk, so using the copy AAE has: {e}");
            return Ok(apk);
        }
        Err(e) => return Err(e),
    };
    let current: CacheInfo = std::fs::read_to_string(&info_path)
        .ok()
        .and_then(|text| toml::from_str(&text).ok())
        .unwrap_or_default();
    if have && current.updated_at == latest.updated_at {
        // Up to date: note when we checked.
        write_info(&info_path, &current)?;
        return Ok(apk);
    }

    std::fs::create_dir_all(&dir).context(|| format!("Creating {}", dir.display()))?;
    let part = dir.join("backtalk-dev.apk.part");
    download(&latest, &part)?;
    if let Err(e) = verify_signature(sdk, &part) {
        let _ = std::fs::remove_file(&part);
        return Err(e);
    }
    std::fs::rename(&part, &apk).context(|| format!("Saving {}", apk.display()))?;
    write_info(
        &info_path,
        &CacheInfo {
            updated_at: latest.updated_at,
        },
    )?;
    Ok(apk)
}

fn latest_asset() -> Result<Asset> {
    let failed =
        |e: String| Error::Download(format!("Couldn't check Backtalk's latest build: {e}"));
    let body = ureq::get(RELEASE)
        .header("User-Agent", "accessible-android-emulator")
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| failed(e.to_string()))?
        .body_mut()
        .read_to_string()
        .map_err(|e| failed(e.to_string()))?;
    let release: Release = serde_json::from_str(&body).map_err(|e| failed(e.to_string()))?;
    release
        .assets
        .into_iter()
        .find(|a| a.name == ASSET)
        .ok_or_else(|| failed("its release has no backtalk.apk".into()))
}

fn download(asset: &Asset, path: &Path) -> Result<()> {
    let failed = |e: String| Error::Download(format!("Backtalk didn't download: {e}"));
    let mut response = ureq::get(&asset.browser_download_url)
        .header("User-Agent", "accessible-android-emulator")
        .call()
        .map_err(|e| failed(e.to_string()))?;
    let mut bytes = Vec::with_capacity(asset.size as usize);
    response
        .body_mut()
        .with_config()
        .limit(asset.size + 1024 * 1024)
        .reader()
        .read_to_end(&mut bytes)
        .map_err(|e| failed(e.to_string()))?;
    std::fs::write(path, bytes).context(|| format!("Saving {}", path.display()))
}

/// Checks the APK is signed with Backtalk's key, using the SDK's apksigner.
fn verify_signature(sdk: &Sdk, apk: &Path) -> Result<()> {
    let fail = |reason: String| Error::Apk {
        path: apk.to_path_buf(),
        reason,
    };
    let apksigner = sdk.apksigner_bin().ok_or_else(|| {
        fail("the SDK has no build tools, which AAE needs to check downloads".into())
    })?;
    let out = Command::new(apksigner)
        .args(["verify", "--print-certs"])
        .arg(apk)
        .output()
        .map_err(|e| fail(format!("apksigner could not run: {e}")))?;
    let text = String::from_utf8_lossy(&out.stdout);
    let signed_by_backtalk = out.status.success()
        && text.lines().any(|line| {
            line.contains("certificate SHA-256 digest:")
                && line.trim_end().ends_with(CERTIFICATE_SHA256)
        });
    if signed_by_backtalk {
        Ok(())
    } else {
        Err(fail(
            "it isn't signed with Backtalk's key, so AAE won't install it".into(),
        ))
    }
}

fn write_info(path: &Path, info: &CacheInfo) -> Result<()> {
    let text = toml::to_string(info).map_err(|e| Error::Config {
        path: path.to_path_buf(),
        reason: e.to_string(),
    })?;
    std::fs::write(path, text).context(|| format!("Writing {}", path.display()))
}
