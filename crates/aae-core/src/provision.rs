//! Setting up a new device, and keeping its accessibility services on.

use std::path::PathBuf;
use std::time::Duration;

use crate::adb::Adb;
use crate::keyboard_layouts::Layout;
use crate::apk::{ApkInfo, ServiceKind};
use crate::device::Device;
use crate::error::{Error, IoContext, Result};
use crate::paths;
use crate::sdk::Sdk;

/// How long to wait for Android to start running a service AAE turned on.
pub const SERVICE_TIMEOUT: Duration = Duration::from_secs(15);

/// AAE's own helper app, which sets the accessibility volume.
pub const HELPER_COMPONENT: &str =
    "io.github.aaron_gh.aae.helper/io.github.aaron_gh.aae.helper.HelperService";
const HELPER_PACKAGE: &str = "io.github.aaron_gh.aae.helper";
const HELPER_RECEIVER: &str = "io.github.aaron_gh.aae.helper/.CommandReceiver";
/// Runs the helper's shell tool, as the shell user, which may choose keyboard layouts.
const HELPER_SHELL_TOOL: &str = "CLASSPATH=$(pm path io.github.aaron_gh.aae.helper | cut -d: -f2) \
     app_process / io.github.aaron_gh.aae.helper.ShellTool";
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
    /// When no screen reader is chosen, download and install Backtalk, rather
    /// than using the image's own TalkBack or none.
    pub backtalk: bool,
    /// Install the helper and set the accessibility volume to full.
    pub full_volume: bool,
    /// Keep the screen on and the lock screen off.
    pub stay_awake: bool,
}

impl Default for ProvisionOptions {
    fn default() -> Self {
        ProvisionOptions {
            screen_reader_apk: None,
            disable_animations: false,
            backtalk: false,
            full_volume: true,
            stay_awake: true,
        }
    }
}

/// One step of setup, for progress announcements.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Helper,
    Keyboard,
    SetupWizard,
    StayAwake,
    Settings,
    Animations,
    ScreenReader,
    Volume,
    Verify,
    Audio,
    Speech,
}

impl Step {
    pub fn describe(self) -> &'static str {
        match self {
            Step::Helper => "Installing AAE's helper",
            Step::Keyboard => "Setting up the full keyboard",
            Step::SetupWizard => "Skipping the setup wizard",
            Step::StayAwake => "Keeping the screen on",
            Step::Settings => "Applying your settings for new devices",
            Step::Animations => "Turning off animations",
            Step::ScreenReader => "Setting up the screen reader",
            Step::Volume => "Turning the screen reader's volume up to full",
            Step::Verify => "Checking the services are on",
            Step::Audio => "Checking the device's audio",
            Step::Speech => "Checking the device can speak",
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
/// next to the program or in the Mac app's Resources folder, then AAE's data
/// folder, then the build output in a source checkout.
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
        candidates.push(dir.join("../Resources/aae-helper.apk"));
    }
    candidates.push(paths::data_dir().join("aae-helper.apk"));
    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../android/helper/build/outputs/apk/release/helper-release.apk"),
    );
    candidates.into_iter().find(|p| p.is_file())
}

/// Installs AAE's helper app, or updates it if this AAE has a newer one.
pub async fn install_helper(adb: &Adb) -> Result<()> {
    let helper = helper_apk().ok_or_else(|| Error::Apk {
        path: paths::data_dir().join("aae-helper.apk"),
        reason: "AAE's helper app was not found. Build it with android/gradlew :helper:assembleRelease, \
                 or put it at this path"
            .into(),
    })?;
    adb.install_own(&helper, HELPER_PACKAGE).await
}

/// Installs AAE's helper if it is missing, or updates it if this AAE has a
/// newer one. Returns true if it installed something.
pub async fn update_helper(sdk: &Sdk, adb: &Adb) -> Result<bool> {
    let installed = adb.version_code(HELPER_PACKAGE).await?;
    let bundled = helper_apk()
        .and_then(|apk| read_apk(sdk, &apk).ok())
        .and_then(|info| info.version_code);
    let needed = match (installed, bundled) {
        (None, _) => true,
        (Some(have), Some(new)) => new > have,
        (Some(_), None) => false,
    };
    if needed {
        install_helper(adb).await?;
        // Android only gives a service its abilities, such as reading the
        // screen, when it's switched on, so switch an updated one off and on.
        let enabled = adb.enabled_services().await?;
        if enabled
            .iter()
            .any(|c| crate::adb::same_component(c, HELPER_COMPONENT))
        {
            adb.disable_service(HELPER_COMPONENT).await?;
            tokio::time::sleep(Duration::from_secs(1)).await;
            adb.ensure_services(&[HELPER_COMPONENT.to_string()], SERVICE_TIMEOUT)
                .await?;
        }
    }
    Ok(needed)
}

/// Makes the emulator's keyboard behave like a PC keyboard, with the layout
/// AAE's helper provides. Without it, Android gets no Meta key, so screen
/// reader shortcuts can't work, and Escape, Home and End act as phone buttons.
///
/// Android may forget the choice when it restarts, so AAE applies it every
/// time a device starts.
pub async fn apply_keyboard_layout(sdk: &Sdk, adb: &Adb, layout: &Layout) -> Result<()> {
    update_helper(sdk, adb).await?;
    let descriptor = layout.full_descriptor();
    // After a cold boot the package manager can take half a minute to list
    // the helper again, and until then the tool can't start, so keep trying.
    let mut last = String::new();
    for _ in 0..45 {
        match adb
            .shell(&format!("{HELPER_SHELL_TOOL} keyboard-layout {descriptor}"))
            .await
        {
            Ok(out) if out.contains("Keyboard layout set") => return Ok(()),
            Ok(out) => last = out,
            Err(e) => last = e.to_string(),
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    Err(Error::Adb(format!(
        "The full keyboard could not be set up: {}",
        last.trim()
    )))
}

/// Selects AAE's full keyboard again, in one quick try, when AAE connects to
/// a device that's already running. Android has been seen to drop the
/// choice while a device runs, for reasons not yet found, which leaves Meta
/// and the Home button dead until it's selected again. Selecting it when
/// it's still selected changes nothing.
pub async fn reselect_keyboard_layout(adb: &Adb, layout: &Layout) -> Result<()> {
    let out = adb
        .shell(&format!(
            "{HELPER_SHELL_TOOL} keyboard-layout {}",
            layout.full_descriptor()
        ))
        .await?;
    if out.contains("Keyboard layout set") {
        Ok(())
    } else {
        Err(Error::Adb(format!(
            "The full keyboard could not be selected: {}",
            out.trim()
        )))
    }
}

/// Same as [`reselect_keyboard_layout`], but a failure is only logged: the
/// keyboard mostly works without it, and what AAE was connecting for
/// shouldn't fail because of it.
pub async fn reselect_keyboard_layout_quietly(adb: &Adb, layout: &Layout) {
    if let Err(e) = reselect_keyboard_layout(adb, layout).await {
        tracing::warn!("{e}");
    }
}

/// Makes sure the device can speak (see [`crate::tts`]). When AAE had to
/// switch speech engines, the screen reader is restarted so it uses the new one.
pub async fn ensure_device_speech(device: &Device, adb: &Adb) -> Result<crate::tts::SpeechFix> {
    let fix = crate::tts::ensure_speech(adb).await?;
    if fix == crate::tts::SpeechFix::InstalledEspeak {
        if let Some(reader) = &device.meta.screen_reader {
            adb.disable_service(reader).await?;
            tokio::time::sleep(Duration::from_secs(1)).await;
            adb.ensure_services(std::slice::from_ref(reader), SERVICE_TIMEOUT)
                .await?;
        }
    }
    Ok(fix)
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

/// What first-boot setup found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProvisionOutcome {
    /// No screen reader was chosen and the Android image has none. The
    /// device is otherwise set up; ask the user what to do (see [`add_screen_reader`]).
    pub needs_screen_reader: bool,
}

/// Runs first-boot setup on a running device. `progress` is called as each step starts.
pub async fn provision(
    sdk: &Sdk,
    device: &mut Device,
    adb: &Adb,
    options: &ProvisionOptions,
    mut progress: impl FnMut(Step),
) -> Result<ProvisionOutcome> {
    progress(Step::Helper);
    update_helper(sdk, adb).await?;

    progress(Step::Keyboard);
    // Don't show the on-screen keyboard while a hardware keyboard is connected.
    adb.put_setting("secure", "show_ime_with_hard_keyboard", "0")
        .await?;
    apply_keyboard_layout(sdk, adb, crate::keyboard_layouts::for_device(&device.meta)).await?;

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

    // Before turning animations off, so asking for that wins.
    let chosen = crate::device_settings::for_new_devices();
    if !chosen.is_empty() {
        progress(Step::Settings);
        for (name, value) in &chosen {
            // One that doesn't apply, such as dark theme on Android 9, is
            // left out rather than stopping setup.
            if let Err(e) = crate::device_settings::change(adb, name, value).await {
                tracing::info!("not applying {name} {value}: {e}");
            }
        }
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
    let component = find_screen_reader(sdk, device.meta.api, adb, options).await?;
    if let Some(component) = &component {
        device.meta.screen_reader = Some(component.clone());
        device.keep_service_enabled(component);
    }

    if options.full_volume {
        progress(Step::Volume);
        device.keep_service_enabled(HELPER_COMPONENT);
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

    progress(Step::Audio);
    measure_audio(device, adb).await;

    progress(Step::Speech);
    let fix = ensure_device_speech(device, adb).await?;
    if let Some(what) = fix.describe() {
        tracing::info!("{what}");
    }

    device.meta.provisioned = true;
    device.save_meta()?;
    Ok(ProvisionOutcome {
        needs_screen_reader: component.is_none(),
    })
}

/// The screen reader for a new device: the one chosen, or Backtalk if asked
/// for, or the image's own TalkBack. None if there is none of these.
async fn find_screen_reader(
    sdk: &Sdk,
    api: u32,
    adb: &Adb,
    options: &ProvisionOptions,
) -> Result<Option<String>> {
    if let Some(apk) = options
        .screen_reader_apk
        .clone()
        .or_else(default_screen_reader_apk)
    {
        return Ok(Some(
            install_screen_reader_apk(sdk, adb, &apk, false).await?,
        ));
    }
    if options.backtalk {
        let apk = download_backtalk(api).await?;
        return Ok(Some(
            install_screen_reader_apk(sdk, adb, &apk, false).await?,
        ));
    }
    if adb
        .is_installed("com.google.android.marvin.talkback")
        .await?
    {
        return Ok(Some(TALKBACK_COMPONENT.to_string()));
    }
    Ok(None)
}

/// Measures how fast the device plays audio and remembers it, so playback can
/// correct the pitch of devices that play slowly. A failed measurement is
/// logged and left for the next start: it must never stop a device starting.
pub async fn measure_audio(device: &mut Device, adb: &Adb) {
    // The check waits for sound from the device; never let it hold up a start.
    if tokio::time::timeout(Duration::from_secs(20), measure_audio_inner(device, adb))
        .await
        .is_err()
    {
        tracing::warn!(
            "the audio check took too long, so it was skipped; it will be tried on the next start"
        );
    }
}

async fn measure_audio_inner(device: &mut Device, adb: &Adb) {
    let Ok(info) = crate::emulator::running(device) else {
        return;
    };
    let controller = match crate::control::Controller::for_runtime(&info, adb.clone()).await {
        Ok(controller) => controller,
        Err(e) => {
            tracing::warn!("couldn't measure the device's audio: {e}");
            return;
        }
    };
    match crate::audio::measure_speed(&controller, adb).await {
        Ok(speed) => {
            if (speed - 1.0).abs() > 0.01 {
                tracing::info!(
                    "{} plays audio at {:.1}% speed; AAE corrects the pitch",
                    device.meta.name,
                    speed * 100.0
                );
            }
            device.meta.audio_speed = Some(speed);
            if let Err(e) = device.save_meta() {
                tracing::warn!("couldn't save the audio measurement: {e}");
            }
        }
        Err(e) => tracing::warn!("couldn't measure the device's audio: {e}"),
    }
}

/// Downloads Backtalk's latest development build, if this Android version can run it.
pub async fn download_backtalk(api: u32) -> Result<PathBuf> {
    if api < crate::screenreader::BACKTALK_MIN_API {
        return Err(Error::Download(format!(
            "Backtalk needs Android 8.0 or later, and this device is {}.",
            crate::sdk::android_name(api)
        )));
    }
    tokio::task::spawn_blocking(crate::screenreader::backtalk_apk)
        .await
        .map_err(|e| Error::Download(e.to_string()))?
}

/// Installs a screen reader APK and returns its accessibility service.
async fn install_screen_reader_apk(
    sdk: &Sdk,
    adb: &Adb,
    apk: &std::path::Path,
    replace: bool,
) -> Result<String> {
    let info = read_apk(sdk, apk)?;
    let component = info
        .accessibility_components()
        .into_iter()
        .next()
        .ok_or_else(|| Error::Apk {
            path: apk.to_path_buf(),
            reason: "it has no accessibility service, so it can't be a screen reader".into(),
        })?;
    if replace && adb.is_installed(&info.package).await? {
        adb.uninstall(&info.package).await?;
    }
    // Installing over the same app keeps its data, so its settings stay.
    let installed = adb.install(apk).await;
    if installed.is_ok() {
        keep_screen_reader_copy(apk, &info.package);
    }
    match installed {
        Err(Error::Adb(message)) if message.contains("INSTALL_FAILED_UPDATE_INCOMPATIBLE") => {
            Err(Error::Apk {
                path: apk.to_path_buf(),
                reason: format!(
                    "it's signed differently from the {} already on the device, so it can't be \
                     installed over it. Replacing it removes the old one first, and with it the \
                     screen reader's settings",
                    info.package
                ),
            })
        }
        other => other.map(|_| component),
    }
}

/// Installs a screen reader on a set-up device and makes it the device's
/// screen reader, turning off the one it had. Returns its package.
/// With `replace`, an installed copy of the same app is removed first, with
/// its settings, for a build signed differently.
pub async fn add_screen_reader(
    sdk: &Sdk,
    device: &mut Device,
    adb: &Adb,
    apk: &std::path::Path,
    replace: bool,
) -> Result<String> {
    let component = install_screen_reader_apk(sdk, adb, apk, replace).await?;
    if let Some(old) = device.meta.screen_reader.take() {
        if !crate::adb::same_component(&old, &component) {
            device.meta.keep_enabled.retain(|c| c != &old);
            adb.disable_service(&old).await?;
        }
    }
    device.meta.screen_reader = Some(component.clone());
    device.meta.screen_reader_declined = false;
    device.keep_service_enabled(&component);
    device.save_meta()?;
    adb.ensure_services(std::slice::from_ref(&component), SERVICE_TIMEOUT)
        .await?;
    Ok(component
        .split('/')
        .next()
        .unwrap_or(&component)
        .to_string())
}

/// Where AAE keeps a copy of a screen reader it installed, so a wiped
/// device can have it again.
fn screen_reader_copy(package: &str) -> PathBuf {
    paths::data_dir()
        .join("screen-readers")
        .join("installed")
        .join(format!("{package}.apk"))
}

fn keep_screen_reader_copy(apk: &std::path::Path, package: &str) {
    let copy = screen_reader_copy(package);
    if copy.as_path() == apk {
        return;
    }
    if let Some(dir) = copy.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::copy(apk, &copy) {
        tracing::warn!("couldn't keep a copy of the screen reader: {e}");
    }
}

/// The screen reader APK to set a device up with again after it's wiped:
/// AAE's copy, else the one on the device if it's running, else, for
/// Backtalk, a fresh download. None if the device has none, or it can't be
/// found.
pub async fn screen_reader_for_wipe(device: &Device, adb: Option<&Adb>) -> Option<PathBuf> {
    let package = device
        .meta
        .screen_reader
        .as_deref()?
        .split('/')
        .next()?
        .to_string();
    let copy = screen_reader_copy(&package);
    if copy.is_file() {
        return Some(copy);
    }
    if let Some(adb) = adb {
        // pm path lists the app's files; the first is its main APK.
        if let Ok(out) = adb.shell(&format!("pm path {}", crate::adb::name(&package).ok()?)).await {
            if let Some(path) = out.lines().find_map(|l| l.trim().strip_prefix("package:")) {
                if let Some(dir) = copy.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let target = copy.to_string_lossy().to_string();
                if adb.raw(&["pull", path, &target]).await.is_ok() && copy.is_file() {
                    return Some(copy);
                }
            }
        }
    }
    if package == crate::screenreader::BACKTALK_PACKAGE {
        return download_backtalk(device.meta.api).await.ok();
    }
    None
}

/// Queues a screen reader build for a stopped device, to install when it
/// next starts. The APK is copied, so it can be moved or rebuilt meanwhile.
pub fn queue_screen_reader(
    sdk: &Sdk,
    device: &mut Device,
    apk: &std::path::Path,
) -> Result<String> {
    let info = read_apk(sdk, apk)?;
    let dir = crate::paths::data_dir()
        .join("screen-readers")
        .join("queued");
    std::fs::create_dir_all(&dir).context(|| format!("Creating {}", dir.display()))?;
    let copy = dir.join(format!("{}.apk", device.id));
    std::fs::copy(apk, &copy).context(|| format!("Copying {}", apk.display()))?;
    device.meta.pending_screen_reader = Some(copy);
    device.save_meta()?;
    Ok(info.package)
}

/// Installs a screen reader build queued while the device was stopped, if
/// there is one. Returns its package, or the reason it couldn't be installed.
pub async fn install_queued_screen_reader(
    sdk: &Sdk,
    device: &mut Device,
    adb: &Adb,
) -> Option<std::result::Result<String, String>> {
    let apk = device.meta.pending_screen_reader.take()?;
    let result = add_screen_reader(sdk, device, adb, &apk, false)
        .await
        .map_err(|e| e.to_string());
    let _ = std::fs::remove_file(&apk);
    let _ = device.save_meta();
    Some(result)
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

/// One of an app's parts that needs the user's say before it runs, and what
/// the user chose for it on this device.
#[derive(Debug, Clone)]
pub struct AppPart {
    pub kind: ServiceKind,
    /// `package/class`.
    pub component: String,
    /// The class name without its package.
    pub name: String,
    /// On or off, if the user has chosen; None if not asked yet.
    pub choice: Option<bool>,
}

/// Installs or updates an app. Parts of it the user has already chosen
/// about on this device, such as an accessibility service, are turned on or
/// off again as chosen. Returns the app's details and all its parts; the
/// ones with no choice yet are for asking about, then [`apply_choices`].
pub async fn install_app(
    sdk: &Sdk,
    device: &mut Device,
    adb: &Adb,
    apk: &std::path::Path,
) -> Result<(ApkInfo, Vec<AppPart>)> {
    let info = read_apk(sdk, apk)?;
    adb.install(apk).await?;
    let parts: Vec<AppPart> = info
        .services
        .iter()
        .map(|s| {
            let component = s.component(&info.package);
            AppPart {
                kind: s.kind,
                choice: device.meta.app_choices.get(&component).copied(),
                name: s.short_name().to_string(),
                component,
            }
        })
        .collect();
    let decided: Vec<(AppPart, bool)> = parts
        .iter()
        .filter_map(|p| p.choice.map(|on| (p.clone(), on)))
        .collect();
    apply_choices(device, adb, &decided).await?;
    // An update can turn other services off, so check them all.
    guard_services(device, adb).await?;
    Ok((info, parts))
}

/// Turns app parts on or off as the user chose, and remembers the choices
/// for this device. Accessibility services that are on are kept on.
pub async fn apply_choices(
    device: &mut Device,
    adb: &Adb,
    choices: &[(AppPart, bool)],
) -> Result<()> {
    if choices.is_empty() {
        return Ok(());
    }
    let mut turn_on = Vec::new();
    for (part, on) in choices {
        let component = &part.component;
        device.meta.app_choices.insert(component.clone(), *on);
        match (part.kind, on) {
            (ServiceKind::Accessibility, true) => {
                device.keep_service_enabled(component);
                turn_on.push(component.clone());
            }
            (ServiceKind::Accessibility, false) => {
                device.meta.keep_enabled.retain(|c| c != component);
                adb.disable_service(component).await?;
            }
            (ServiceKind::InputMethod, on) => {
                let verb = if *on { "enable" } else { "disable" };
                adb.shell(&format!("ime {verb} {}", crate::adb::name(component)?))
                    .await?;
            }
            (ServiceKind::NotificationListener, on) => {
                set_listed(adb, "enabled_notification_listeners", component, *on).await?;
            }
            (ServiceKind::DeviceAdmin, true) => {
                let out = adb
                    .shell(&format!(
                        "dpm set-active-admin --user 0 {}",
                        crate::adb::name(component)?
                    ))
                    .await?;
                if !out.contains("Success") {
                    return Err(Error::Adb(format!(
                        "Android didn't make {component} a device administrator: {}",
                        out.trim()
                    )));
                }
            }
            (ServiceKind::DeviceAdmin, false) => {
                // Android only lets some administrators be removed this way;
                // the rest are removed in the device's security settings.
                if let Ok(component) = crate::adb::name(component) {
                    let _ = adb
                        .shell(&format!("dpm remove-active-admin --user 0 {component}"))
                        .await;
                }
            }
        }
    }
    device.save_meta()?;
    if !turn_on.is_empty() {
        adb.ensure_services(&turn_on, SERVICE_TIMEOUT).await?;
    }
    Ok(())
}

/// Adds a component to, or removes it from, a colon-separated secure setting.
async fn set_listed(adb: &Adb, key: &str, component: &str, on: bool) -> Result<()> {
    let current = adb.setting("secure", key).await?.unwrap_or_default();
    let mut list: Vec<String> = current
        .split(':')
        .filter(|c| !c.is_empty() && !crate::adb::same_component(c, component))
        .map(String::from)
        .collect();
    if on {
        list.push(component.to_string());
    }
    adb.put_setting("secure", key, &list.join(":")).await
}

/// Installs AAE's helper if needed, keeps it on, and sets the screen reader's
/// volume through it. Returns the volume index Android reports.
pub async fn boost_volume(device: &mut Device, adb: &Adb, percent: u8) -> Result<i32> {
    if !adb.is_installed(HELPER_PACKAGE).await? {
        install_helper(adb).await?;
    }
    if device.keep_service_enabled(HELPER_COMPONENT) {
        device.save_meta()?;
    }
    adb.ensure_services(&[HELPER_COMPONENT.to_string()], SERVICE_TIMEOUT)
        .await?;
    set_volume(adb, percent).await
}
