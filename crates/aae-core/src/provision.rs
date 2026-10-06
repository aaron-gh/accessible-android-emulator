//! Setting up a new device, and keeping its accessibility services on.

use std::path::PathBuf;
use std::time::Duration;

use crate::adb::Adb;
use crate::apk::ApkInfo;
use crate::device::Device;
use crate::error::{Error, Result};
use crate::paths;
use crate::sdk::Sdk;

/// How long to wait for Android to start running a service AAE turned on.
pub const SERVICE_TIMEOUT: Duration = Duration::from_secs(15);

/// AAE's own helper app, which sets the accessibility volume.
pub const HELPER_COMPONENT: &str =
    "io.github.aaron_gh.aae.helper/io.github.aaron_gh.aae.helper.HelperService";
const HELPER_RECEIVER: &str = "io.github.aaron_gh.aae.helper/.CommandReceiver";
const HELPER_SET_VOLUME: &str = "io.github.aaron_gh.aae.helper.SET_VOLUME";

/// Google's TalkBack, preinstalled on some images.
pub const TALKBACK_COMPONENT: &str =
    "com.google.android.marvin.talkback/com.google.android.marvin.talkback.TalkBackService";

/// Choices for first-boot setup.
#[derive(Debug, Clone)]
pub struct ProvisionOptions {
    /// The screen reader to install. When `None`, AAE looks for one (see
    /// [`default_screen_reader_apk`]) and otherwise uses TalkBack if the image has it.
    pub screen_reader_apk: Option<PathBuf>,
    pub disable_animations: bool,
    /// Install AAE's helper and use it to turn the screen reader's volume to
    /// full. AAE controls loudness on the computer, so this gives the clearest sound.
    pub full_volume: bool,
    /// Keep the screen on and the lock screen off, so the screen reader never
    /// goes quiet because the device went to sleep.
    pub stay_awake: bool,
}

impl Default for ProvisionOptions {
    fn default() -> Self {
        ProvisionOptions {
            screen_reader_apk: None,
            disable_animations: false,
            full_volume: true,
            stay_awake: true,
        }
    }
}

/// One step of setup, for progress announcements.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Keyboard,
    SetupWizard,
    StayAwake,
    Animations,
    ScreenReader,
    Volume,
    Verify,
}

impl Step {
    pub fn describe(self) -> &'static str {
        match self {
            Step::Keyboard => "Setting up the hardware keyboard",
            Step::SetupWizard => "Skipping the setup wizard",
            Step::StayAwake => "Keeping the screen on",
            Step::Animations => "Turning off animations",
            Step::ScreenReader => "Installing and turning on the screen reader",
            Step::Volume => "Turning the screen reader's volume up to full",
            Step::Verify => "Checking the screen reader is on",
        }
    }
}

/// Where AAE looks for a screen reader when none is given: `AAE_SCREEN_READER_APK`,
/// then `screen-readers/default.apk` in AAE's data folder.
pub fn default_screen_reader_apk() -> Option<PathBuf> {
    std::env::var_os("AAE_SCREEN_READER_APK")
        .map(PathBuf::from)
        .or_else(|| Some(paths::data_dir().join("screen-readers/default.apk")))
        .filter(|p| p.is_file())
}

/// Where AAE looks for its helper app: `AAE_HELPER_APK`, then `aae-helper.apk`
/// next to the program, then AAE's data folder, then the build output in a
/// source checkout.
pub fn helper_apk() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("AAE_HELPER_APK") {
        candidates.push(PathBuf::from(path));
    }
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(PathBuf::from))
    {
        candidates.push(dir.join("aae-helper.apk"));
    }
    candidates.push(paths::data_dir().join("aae-helper.apk"));
    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../android/helper/build/outputs/apk/release/helper-release.apk"),
    );
    candidates.into_iter().find(|p| p.is_file())
}

/// Sets the screen reader's volume, from 0 to 100 percent, through AAE's helper.
/// Returns the volume index Android reports afterwards.
pub async fn set_volume(adb: &Adb, percent: u8) -> Result<i32> {
    let out = adb
        .shell(&format!(
            "am broadcast -n {HELPER_RECEIVER} -a {HELPER_SET_VOLUME} --ei percent {}",
            percent.min(100)
        ))
        .await?;
    // am prints "Broadcast completed: result=12, data=...".
    out.split("result=")
        .nth(1)
        .and_then(|rest| rest.split([',', ' ', '\n']).next())
        .and_then(|n| n.trim().parse().ok())
        .ok_or_else(|| Error::Adb(format!("AAE's helper did not answer: {}", out.trim())))
}

/// Runs first-boot setup on a running device. `progress` is called as each step starts.
pub async fn provision(
    sdk: &Sdk,
    device: &mut Device,
    adb: &Adb,
    options: &ProvisionOptions,
    mut progress: impl FnMut(Step),
) -> Result<()> {
    progress(Step::Keyboard);
    // Don't show the on-screen keyboard while a hardware keyboard is connected.
    adb.put_setting("secure", "show_ime_with_hard_keyboard", "0")
        .await?;

    progress(Step::SetupWizard);
    adb.put_setting("global", "device_provisioned", "1").await?;
    adb.put_setting("secure", "user_setup_complete", "1")
        .await?;

    if options.stay_awake {
        progress(Step::StayAwake);
        adb.put_setting("global", "stay_on_while_plugged_in", "7")
            .await?;
        adb.put_setting("system", "screen_off_timeout", "2147483647")
            .await?;
        // Not every version has locksettings; the lock screen is only a nuisance.
        let _ = adb.shell("locksettings set-disabled true").await;
        let _ = adb.shell("input keyevent KEYCODE_WAKEUP").await;
        let _ = adb.shell("wm dismiss-keyguard").await;
    }

    if options.disable_animations {
        progress(Step::Animations);
        for key in [
            "window_animation_scale",
            "transition_animation_scale",
            "animator_duration_scale",
        ] {
            adb.put_setting("global", key, "0").await?;
        }
    }

    progress(Step::ScreenReader);
    let component = install_screen_reader(sdk, adb, options).await?;
    device.meta.screen_reader = Some(component.clone());
    device.keep_service_enabled(&component);

    if options.full_volume {
        if let Some(helper) = helper_apk() {
            progress(Step::Volume);
            adb.install(&helper).await?;
            device.keep_service_enabled(HELPER_COMPONENT);
        } else {
            tracing::warn!(
                "AAE's helper app was not found, so the screen reader's volume was left as it is"
            );
        }
    }

    progress(Step::Verify);
    adb.ensure_services(&device.meta.keep_enabled, SERVICE_TIMEOUT)
        .await?;
    if device
        .meta
        .keep_enabled
        .iter()
        .any(|c| c == HELPER_COMPONENT)
    {
        set_volume(adb, 100).await?;
    }

    device.meta.provisioned = true;
    device.save_meta()
}

async fn install_screen_reader(sdk: &Sdk, adb: &Adb, options: &ProvisionOptions) -> Result<String> {
    let apk = options
        .screen_reader_apk
        .clone()
        .or_else(default_screen_reader_apk);
    if let Some(apk) = apk {
        let info = read_apk(sdk, &apk)?;
        let component = info
            .accessibility_components()
            .into_iter()
            .next()
            .ok_or_else(|| Error::Apk {
                path: apk.clone(),
                reason: "it has no accessibility service, so it can't be a screen reader".into(),
            })?;
        adb.install(&apk).await?;
        return Ok(component);
    }
    if adb
        .is_installed("com.google.android.marvin.talkback")
        .await?
    {
        return Ok(TALKBACK_COMPONENT.to_string());
    }
    Err(Error::Apk {
        path: paths::data_dir().join("screen-readers/default.apk"),
        reason: "no screen reader was given and this Android image has no TalkBack. \
                 Give one with --screen-reader, or put it at this path"
            .into(),
    })
}

/// Reads an APK's package name and services.
pub fn read_apk(sdk: &Sdk, apk: &std::path::Path) -> Result<ApkInfo> {
    let aapt2 = sdk.aapt2_bin().ok_or_else(|| Error::Apk {
        path: apk.to_path_buf(),
        reason: "the SDK has no build tools, which AAE uses to read app packages. \
                 Install the Android SDK Build-Tools package"
            .into(),
    })?;
    ApkInfo::read(&aapt2, apk)
}

/// Turns the device's keep-enabled services back on if Android turned any off.
/// Returns the ones that had to be turned back on.
pub async fn guard_services(device: &Device, adb: &Adb) -> Result<Vec<String>> {
    if device.meta.keep_enabled.is_empty() {
        return Ok(Vec::new());
    }
    adb.ensure_services(&device.meta.keep_enabled, SERVICE_TIMEOUT)
        .await
}

/// Installs an app and, if asked, turns on its accessibility services and
/// remembers to keep them on. Returns the app's details and the services turned on.
pub async fn install_app(
    sdk: &Sdk,
    device: &mut Device,
    adb: &Adb,
    apk: &std::path::Path,
    enable_services: bool,
) -> Result<(ApkInfo, Vec<String>)> {
    let info = read_apk(sdk, apk)?;
    adb.install(apk).await?;
    let mut enabled = Vec::new();
    if enable_services {
        let components = info.accessibility_components();
        for component in &components {
            device.keep_service_enabled(component);
        }
        device.save_meta()?;
        adb.ensure_services(&components, SERVICE_TIMEOUT).await?;
        enabled = components;
    }
    // An update can turn other services off, so check them all.
    guard_services(device, adb).await?;
    Ok((info, enabled))
}

/// Installs AAE's helper if needed, keeps it on, and sets the screen reader's
/// volume through it. Returns the volume index Android reports.
pub async fn boost_volume(device: &mut Device, adb: &Adb, percent: u8) -> Result<i32> {
    if !adb.is_installed("io.github.aaron_gh.aae.helper").await? {
        let helper = helper_apk().ok_or_else(|| Error::Apk {
            path: paths::data_dir().join("aae-helper.apk"),
            reason: "AAE's helper app was not found. Build it with android/gradlew :helper:assembleRelease, \
                     or put it at this path"
                .into(),
        })?;
        adb.install(&helper).await?;
    }
    if device.keep_service_enabled(HELPER_COMPONENT) {
        device.save_meta()?;
    }
    adb.ensure_services(&[HELPER_COMPONENT.to_string()], SERVICE_TIMEOUT)
        .await?;
    set_volume(adb, percent).await
}
