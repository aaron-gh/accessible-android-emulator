//! The bridge between AAE's Rust core and its native host apps.
//!
//! UniFFI generates Swift bindings from this file (C# and Kotlin later), so the
//! Mac app calls the core as an ordinary Swift library. Everything here is
//! coarse-grained: one call per user action, with progress reported as
//! sentences ready to show or announce.
//!
//! Async calls run on a tokio runtime owned by this crate, so the host
//! language's own async machinery never needs to know about tokio.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use aae_core::adb::Adb;
use aae_core::apk::ServiceKind;
use aae_core::audio::AudioPlayer;
use aae_core::catalog::{self, Catalogue, InstallProgress};
use aae_core::control::Controller;
use aae_core::device::{Device, DeviceStore, Profile, human_size};
use aae_core::emulator::{self, StartOptions};
use aae_core::gestures::{self, Gesture};
use aae_core::logcat::{self, LogStream};
use aae_core::provision::{self, ProvisionOptions};
use aae_core::sdk::{Sdk, android_name, image_kind};
use aae_core::setup;
use aae_core::speech::{Announcer, Route};
use aae_core::{inspector, tts};
use aae_core::{keys, lifecycle};
use tokio::sync::mpsc;

uniffi::setup_scaffolding!();

fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .thread_name("aae-core")
            .enable_all()
            .build()
            .expect("the tokio runtime could not start")
    })
}

/// Runs a future on AAE's runtime and waits for it from any async context.
async fn on_runtime<T: Send + 'static>(
    future: impl Future<Output = Result<T, AaeError>> + Send + 'static,
) -> Result<T, AaeError> {
    runtime()
        .spawn(future)
        .await
        .map_err(|e| AaeError::Failed {
            message: e.to_string(),
        })?
}

/// Every error, as a message written to be read aloud.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum AaeError {
    #[error("{message}")]
    Failed { message: String },
}

impl From<aae_core::Error> for AaeError {
    fn from(e: aae_core::Error) -> Self {
        AaeError::Failed {
            message: e.to_string(),
        }
    }
}

/// Receives progress sentences during long operations.
#[uniffi::export(with_foreign)]
pub trait ProgressListener: Send + Sync {
    fn progress(&self, message: String);
}

/// A device, as the host app lists it.
#[derive(uniffi::Record)]
pub struct DeviceInfo {
    /// The emulator's name for it. Stable; use it to refer to the device.
    pub id: String,
    /// The name the user chose.
    pub name: String,
    /// Such as "Android 16 (API 36)".
    pub android: String,
    /// Such as "With Google Play".
    pub kind: String,
    /// Such as "Phone".
    pub profile: String,
    pub running: bool,
    /// The screen reader's package, if one is set up.
    pub screen_reader: Option<String>,
    /// The user chose to go without a screen reader; don't offer one again.
    pub screen_reader_declined: bool,
    /// The speech log is recording what the screen reader says.
    pub speech_log: bool,
    /// Backtalk runs on this device's Android version (8.0 and later).
    pub backtalk_supported: bool,
    /// How loud AAE plays it on the Mac, from 0 to 1.
    pub volume: f32,
}

impl DeviceInfo {
    fn from_device(device: &Device) -> Self {
        DeviceInfo {
            id: device.id.clone(),
            name: device.meta.name.clone(),
            android: android_name(device.meta.api),
            kind: image_kind(&device.meta.tag).to_string(),
            profile: device.meta.profile.describe().to_string(),
            running: emulator::running(device).is_ok(),
            screen_reader: device
                .meta
                .screen_reader
                .as_deref()
                .and_then(|c| c.split('/').next())
                .map(String::from),
            screen_reader_declined: device.meta.screen_reader_declined,
            speech_log: device.meta.speech_log_engine.is_some(),
            backtalk_supported: device.meta.api >= aae_core::screenreader::BACKTALK_MIN_API,
            volume: device.meta.playback_volume.unwrap_or(1.0),
        }
    }
}

/// An Android version installed on this computer.
#[derive(uniffi::Record)]
pub struct ImageInfo {
    /// Pass this to `create_device`.
    pub sysdir: String,
    pub api: u32,
    /// Such as "Android 16 (API 36), With Google Play".
    pub description: String,
}

/// An installed Android version, with what it costs and who uses it.
#[derive(uniffi::Record)]
pub struct InstalledImageInfo {
    /// Pass this to `remove_image`.
    pub sysdir: String,
    /// Such as "Android 16 (API 36), With Google Play".
    pub description: String,
    /// Its disk space, in words.
    pub size: String,
    /// AAE's devices made from it. It can't be deleted while there are any.
    pub devices: Vec<String>,
    /// Other emulator devices, such as Android Studio's, that use it.
    pub other_devices: Vec<String>,
}

/// An Android version AAE can create devices from: installed, or downloadable.
#[derive(uniffi::Record)]
pub struct VersionInfo {
    pub api: u32,
    /// The image type's tag, such as "google_apis".
    pub tag: String,
    /// Such as "Android 15 (API 35), With Google services".
    pub description: String,
    pub installed: bool,
    /// The download size in words, such as "1.7 gigabytes". Empty when installed.
    pub size: String,
    /// Pass this to `create_device` when installed. Empty when not.
    pub sysdir: String,
}

/// A licence the user has to accept before a download.
#[derive(uniffi::Record)]
pub struct LicenceInfo {
    pub id: String,
    pub text: String,
}

/// One of the SDK tools AAE downloads.
#[derive(uniffi::Record)]
pub struct ToolInfo {
    /// Such as "Android Emulator".
    pub name: String,
    pub revision: String,
    /// Download size, in words.
    pub size: String,
}

/// What setting up the Android SDK still needs.
#[derive(uniffi::Record)]
pub struct SetupStatus {
    pub sdk_path: String,
    /// True for AAE's own SDK folder, false for one shared with Android Studio.
    pub own_sdk: bool,
    /// Why this computer can't run the emulator, if it can't.
    pub virtualisation_problem: Option<String>,
    /// Tools to download before AAE can work.
    pub missing: Vec<ToolInfo>,
    /// Tools AAE installed that have newer versions available.
    pub updates: Vec<ToolInfo>,
    /// Installed tools AAE didn't install, such as Android Studio's, which
    /// AAE leaves to whatever installed them.
    pub managed_elsewhere: Vec<String>,
    /// The download size of the missing tools, in words.
    pub missing_size: String,
}

/// Receives download progress.
#[uniffi::export(with_foreign)]
pub trait DownloadListener: Send + Sync {
    /// The percentage downloaded, from 0 to 100.
    fn downloaded(&self, percent: u32);
    /// A stage after the download, in words, such as "Checking the download."
    fn stage(&self, message: String);
}

/// Where a screen reader comes from.
#[derive(uniffi::Enum)]
pub enum ScreenReaderSource {
    /// Backtalk's latest development build, downloaded from its project.
    Backtalk,
    /// An APK on this computer.
    Apk { path: String },
}

/// One row of the accessibility inspector: a window or an element.
#[derive(uniffi::Record)]
pub struct InspectorRow {
    pub index: u32,
    /// The row this one is inside, or none for a window.
    pub parent: Option<u32>,
    /// How the row reads, such as "Send, button, disabled".
    pub summary: String,
    /// Every property, one per line.
    pub details: Vec<String>,
}

/// An accessibility problem found on the screen.
#[derive(uniffi::Record)]
pub struct IssueInfo {
    /// An error, rather than a warning.
    pub error: bool,
    pub message: String,
    pub element: String,
    pub id: Option<String>,
}

/// The screen's accessibility tree and the problems found in it.
#[derive(uniffi::Record)]
pub struct Inspection {
    /// Windows and elements, each after the row it's inside.
    pub rows: Vec<InspectorRow>,
    pub issues: Vec<IssueInfo>,
    /// The tree as indented text, for copying.
    pub text: String,
    /// The tree as JSON, with every property, for saving.
    pub json: String,
}

/// One thing the screen reader said.
#[derive(uniffi::Record)]
pub struct UtteranceInfo {
    /// When, in milliseconds since 1970.
    pub time: u64,
    /// The time as a local clock time, such as "17:42:06.250".
    pub clock: String,
    pub text: String,
}

/// What kind of special part an app has.
#[derive(uniffi::Enum, Clone, Copy)]
pub enum AppPartKind {
    AccessibilityService,
    Keyboard,
    NotificationListener,
    DeviceAdministrator,
}

/// A part of an app that needs the user's say before it runs.
#[derive(uniffi::Record, Clone)]
pub struct AppPartInfo {
    pub kind: AppPartKind,
    /// `package/class`.
    pub component: String,
    /// The class name without its package.
    pub name: String,
    /// In words, such as "an accessibility service".
    pub kind_description: String,
    /// What the user chose before on this device; None if not asked yet.
    pub choice: Option<bool>,
}

impl From<provision::AppPart> for AppPartInfo {
    fn from(part: provision::AppPart) -> Self {
        AppPartInfo {
            kind: match part.kind {
                ServiceKind::Accessibility => AppPartKind::AccessibilityService,
                ServiceKind::InputMethod => AppPartKind::Keyboard,
                ServiceKind::NotificationListener => AppPartKind::NotificationListener,
                ServiceKind::DeviceAdmin => AppPartKind::DeviceAdministrator,
            },
            kind_description: part.kind.describe().to_string(),
            component: part.component,
            name: part.name,
            choice: part.choice,
        }
    }
}

impl From<AppPartInfo> for provision::AppPart {
    fn from(part: AppPartInfo) -> Self {
        provision::AppPart {
            kind: match part.kind {
                AppPartKind::AccessibilityService => ServiceKind::Accessibility,
                AppPartKind::Keyboard => ServiceKind::InputMethod,
                AppPartKind::NotificationListener => ServiceKind::NotificationListener,
                AppPartKind::DeviceAdministrator => ServiceKind::DeviceAdmin,
            },
            component: part.component,
            name: part.name,
            choice: part.choice,
        }
    }
}

/// Whether to turn on an app part.
#[derive(uniffi::Record)]
pub struct AppChoice {
    pub part: AppPartInfo,
    pub on: bool,
}

/// An installed app and its special parts.
#[derive(uniffi::Record)]
pub struct InstallResult {
    pub package: String,
    pub parts: Vec<AppPartInfo>,
}

/// A text extra for an intent.
#[derive(uniffi::Record)]
pub struct IntentExtra {
    pub key: String,
    pub value: String,
}

/// An intent to send.
#[derive(uniffi::Record)]
pub struct IntentInfo {
    pub action: Option<String>,
    pub data: Option<String>,
    /// An app's package, or `package/class` for one of its screens or receivers.
    pub target: Option<String>,
    pub extras: Vec<IntentExtra>,
    pub broadcast: bool,
}

/// An installed accessibility service.
#[derive(uniffi::Record)]
pub struct ServiceInfo {
    /// As `package/class`.
    pub component: String,
    pub label: String,
    pub description: String,
    pub screen_reader: bool,
    pub on: bool,
    /// The device's screen reader, as AAE keeps it.
    pub current_screen_reader: bool,
}

/// An installed app.
#[derive(uniffi::Record)]
pub struct AppInfo {
    pub package: String,
    /// The name people see.
    pub label: String,
    pub version: String,
    pub system: bool,
    pub enabled: bool,
    pub launchable: bool,
}

/// A permission an app asks the user for.
#[derive(uniffi::Record)]
pub struct PermissionInfo {
    pub name: String,
    /// In words, such as "take pictures and videos".
    pub label: String,
    pub granted: bool,
}

/// A kind of special access.
#[derive(uniffi::Enum, Clone, Copy)]
pub enum AccessKind {
    Battery,
    Overlay,
    Usage,
    WriteSettings,
}

impl From<aae_core::apps::Access> for AccessKind {
    fn from(a: aae_core::apps::Access) -> Self {
        use aae_core::apps::Access;
        match a {
            Access::Battery => AccessKind::Battery,
            Access::Overlay => AccessKind::Overlay,
            Access::Usage => AccessKind::Usage,
            Access::WriteSettings => AccessKind::WriteSettings,
        }
    }
}

impl From<AccessKind> for aae_core::apps::Access {
    fn from(a: AccessKind) -> Self {
        use aae_core::apps::Access;
        match a {
            AccessKind::Battery => Access::Battery,
            AccessKind::Overlay => Access::Overlay,
            AccessKind::Usage => Access::Usage,
            AccessKind::WriteSettings => Access::WriteSettings,
        }
    }
}

/// Special access an app has or hasn't.
#[derive(uniffi::Record)]
pub struct AccessInfo {
    pub kind: AccessKind,
    /// In words, such as "Unrestricted battery use".
    pub name: String,
    pub allowed: bool,
}

/// An app's permissions and special access.
#[derive(uniffi::Record)]
pub struct AppPermissions {
    pub permissions: Vec<PermissionInfo>,
    pub access: Vec<AccessInfo>,
}

/// A saved snapshot of a device.
#[derive(uniffi::Record)]
pub struct SnapshotInfo {
    pub id: String,
    pub name: String,
    pub notes: String,
    /// When it was taken, such as "6 Oct 2026, 20:41".
    pub taken: Option<String>,
    /// Its size, in words.
    pub size: String,
    /// The device started from it, or it was the last one restored.
    pub loaded: bool,
    /// False when this emulator won't restore it.
    pub compatible: bool,
}

/// What the other end of a phone call does.
#[derive(uniffi::Enum)]
pub enum CallAction {
    /// Call the device from a number, so it rings.
    Ring,
    /// End the call.
    HangUp,
    /// Answer a call the device is making.
    Answer,
    /// Be busy for a call the device is making.
    Busy,
    Hold,
    Resume,
}

/// A point on the screen as the user sees it, in pixels.
#[derive(uniffi::Record, Clone, Copy)]
pub struct ScreenPoint {
    pub x: i32,
    pub y: i32,
}

/// Something on the screen that can be touched.
#[derive(uniffi::Record)]
pub struct TouchTarget {
    /// What a screen reader would say, such as "Send, button".
    pub label: String,
    /// Its centre.
    pub x: i32,
    pub y: i32,
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// The things on the screen that can be touched, and the screen's size as
/// the user sees it.
#[derive(uniffi::Record)]
pub struct TouchTargets {
    pub width: i32,
    pub height: i32,
    pub targets: Vec<TouchTarget>,
}

/// How a self-test check came out.
#[derive(uniffi::Enum, Clone, Copy, PartialEq)]
pub enum CheckOutcome {
    Passed,
    Warning,
    Failed,
}

/// One self-test check.
#[derive(uniffi::Record)]
pub struct CheckInfo {
    pub name: String,
    pub outcome: CheckOutcome,
    pub detail: String,
}

/// What the audio check found.
#[derive(uniffi::Record)]
pub struct AudioCheck {
    pub working: bool,
    /// In words.
    pub message: String,
}

/// What a shell command printed, and how it ended.
#[derive(uniffi::Record)]
pub struct CommandResult {
    /// Everything it printed, errors included.
    pub output: String,
    /// 0 when it succeeded.
    pub status: i32,
}

/// How important a log line is.
#[derive(uniffi::Enum, Clone, Copy)]
pub enum LogLevel {
    Verbose,
    Debug,
    Info,
    Warning,
    Error,
    Fatal,
}

impl From<logcat::Level> for LogLevel {
    fn from(level: logcat::Level) -> Self {
        match level {
            logcat::Level::Verbose => LogLevel::Verbose,
            logcat::Level::Debug => LogLevel::Debug,
            logcat::Level::Info => LogLevel::Info,
            logcat::Level::Warning => LogLevel::Warning,
            logcat::Level::Error => LogLevel::Error,
            logcat::Level::Fatal => LogLevel::Fatal,
        }
    }
}

impl From<LogLevel> for logcat::Level {
    fn from(level: LogLevel) -> Self {
        match level {
            LogLevel::Verbose => logcat::Level::Verbose,
            LogLevel::Debug => logcat::Level::Debug,
            LogLevel::Info => logcat::Level::Info,
            LogLevel::Warning => logcat::Level::Warning,
            LogLevel::Error => logcat::Level::Error,
            LogLevel::Fatal => logcat::Level::Fatal,
        }
    }
}

/// Which log lines to show. Empty fields match everything.
#[derive(uniffi::Record)]
pub struct LogFilter {
    /// An app's package name, or another process name.
    pub process: Option<String>,
    pub tag: Option<String>,
    /// The least important level shown.
    pub level: Option<LogLevel>,
    /// Text to find in the tag or message.
    pub text: Option<String>,
}

/// One line of the device log.
#[derive(uniffi::Record)]
pub struct LogEntryInfo {
    /// Counts up for each line, for asking what's new.
    pub seq: u64,
    /// The time of day, such as "18:27:36.037".
    pub clock: String,
    pub level: LogLevel,
    pub tag: String,
    pub message: String,
    pub process: Option<String>,
    /// The line as a screen reader user wants it, most important part first.
    pub spoken: String,
    /// The line as text, for copying and saving.
    pub line: String,
}

impl Session {
    /// The screen, with its size cached and its rotation read fresh.
    async fn screen(&self) -> Result<gestures::Screen, AaeError> {
        let mut cached = self.screen.lock().await;
        let mut screen = match *cached {
            Some(screen) => screen,
            None => gestures::Screen::read(&self.adb).await?,
        };
        screen.orientation = gestures::display_orientation(&self.adb).await;
        *cached = Some(screen);
        Ok(screen)
    }

    async fn gesture(
        self: Arc<Self>,
        name: String,
        at: Option<ScreenPoint>,
        hold: bool,
    ) -> Result<(), AaeError> {
        let at = at.map(|p| (p.x, p.y));
        let gesture = Gesture::parse(&name).ok_or_else(|| AaeError::Failed {
            message: format!("\"{name}\" is not a gesture."),
        })?;
        on_runtime(async move {
            // Holding this lock for the whole gesture keeps gestures in order
            // and stops two from mixing their fingers.
            let mut held = self.held.lock().await;
            // Fingers still down from a held gesture lift first.
            if let Some(release) = held.take() {
                gestures::lift(&self.controller, &release).await?;
            }
            let screen = self.screen().await?;
            if hold {
                let release = gestures::press(&self.controller, &screen, &gesture, at).await?;
                *held = Some(release);
            } else {
                gestures::perform(&self.controller, &screen, &gesture, at).await?;
            }
            Ok(())
        })
        .await
    }
}

/// The kind of hardware a new device has.
#[derive(uniffi::Enum)]
pub enum DeviceProfile {
    SmallPhone,
    Phone,
    Tablet,
}

impl From<DeviceProfile> for Profile {
    fn from(p: DeviceProfile) -> Self {
        match p {
            DeviceProfile::SmallPhone => Profile::SmallPhone,
            DeviceProfile::Phone => Profile::Phone,
            DeviceProfile::Tablet => Profile::Tablet,
        }
    }
}

/// AAE's core: the SDK, the devices, and announcements.
#[derive(uniffi::Object)]
pub struct Engine {
    sdk: Sdk,
    store: DeviceStore,
    announcer: Announcer,
}

#[uniffi::export]
impl Engine {
    #[uniffi::constructor]
    pub fn new() -> Result<Arc<Self>, AaeError> {
        {
            use aae_core::diagnostics::{LOG_FILTER, LogFile};
            use tracing_subscriber::{EnvFilter, Layer, fmt, prelude::*};
            // To standard error as AAE_LOG asks, and always to AAE's log
            // file, which a diagnostic report includes.
            let _ = tracing_subscriber::registry()
                .with(
                    fmt::layer()
                        .with_writer(std::io::stderr)
                        .with_filter(EnvFilter::from_env("AAE_LOG")),
                )
                .with(
                    fmt::layer()
                        .with_ansi(false)
                        .with_writer(LogFile::open)
                        .with_filter(EnvFilter::new(LOG_FILTER)),
                )
                .try_init();
        }
        tracing::info!("the Mac app started");
        Ok(Arc::new(Engine {
            sdk: Sdk::locate_or_new(),
            store: DeviceStore::open_default()?,
            announcer: Announcer::new(Route::Best),
        }))
    }

    /// Checks everything AAE needs, without making a sound.
    pub async fn self_test(&self) -> Vec<CheckInfo> {
        let (sdk, store) = (self.sdk.clone(), self.store.clone());
        on_runtime(async move {
            Ok(aae_core::diagnostics::self_test(&sdk, &store)
                .await
                .into_iter()
                .map(|c| CheckInfo {
                    name: c.name,
                    outcome: match c.outcome {
                        aae_core::diagnostics::Outcome::Passed => CheckOutcome::Passed,
                        aae_core::diagnostics::Outcome::Warning => CheckOutcome::Warning,
                        aae_core::diagnostics::Outcome::Failed => CheckOutcome::Failed,
                    },
                    detail: c.detail,
                })
                .collect())
        })
        .await
        .unwrap_or_default()
    }

    /// A diagnostic report for a bug report, as plain text with personal
    /// details taken out. `version` is the app's version.
    pub fn diagnostic_report(&self, version: String) -> String {
        aae_core::diagnostics::report(&self.sdk, &self.store, &version)
    }

    /// Records a problem the app showed the user in AAE's log.
    pub fn log_problem(&self, message: String) {
        tracing::warn!("{message}");
    }

    /// Speaks through the user's screen reader, or a system voice when none is
    /// running. `interrupt` is for failures, which cut off other speech.
    pub fn announce(&self, text: String, interrupt: bool) {
        if interrupt {
            self.announcer.failure(text);
        } else {
            self.announcer.info(text);
        }
    }

    /// True when the emulator or SDK tools AAE needs aren't installed. Quick,
    /// and works offline; `setup_status` says what exactly.
    pub fn needs_setup(&self) -> bool {
        self.sdk.emulator_bin().is_err()
            || self.sdk.adb_bin().is_err()
            || self.sdk.aapt2_bin().is_none()
            || self.sdk.apksigner_bin().is_none()
    }

    /// What setting up the SDK still needs, from Google's list of tools.
    pub async fn setup_status(&self, refresh: bool) -> Result<SetupStatus, AaeError> {
        let sdk = self.sdk.clone();
        on_runtime(async move {
            let tools = load_tools(refresh).await?;
            let info = |t: &setup::Tool| ToolInfo {
                name: t.name.clone(),
                revision: t.revision.clone(),
                size: human_size(t.size),
            };
            let missing = tools.missing(&sdk);
            Ok(SetupStatus {
                sdk_path: sdk.root.display().to_string(),
                own_sdk: setup::is_own_sdk(&sdk.root),
                virtualisation_problem: match setup::virtualisation() {
                    setup::Virtualisation::Missing(how) => Some(how),
                    _ => None,
                },
                missing_size: human_size(missing.iter().map(|t| t.size).sum()),
                missing: missing.into_iter().map(info).collect(),
                updates: tools.updates(&sdk).into_iter().map(info).collect(),
                managed_elsewhere: tools
                    .managed_elsewhere(&sdk)
                    .into_iter()
                    .map(|t| t.name.clone())
                    .collect(),
            })
        })
        .await
    }

    /// The licence the missing tools (and, with `update`, the updates) are
    /// under, if it hasn't been accepted yet.
    pub async fn tools_licence(&self, update: bool) -> Result<Option<LicenceInfo>, AaeError> {
        let sdk = self.sdk.clone();
        on_runtime(async move {
            let tools = load_tools(false).await?;
            let wanted = wanted_tools(&tools, &sdk, update);
            Ok(tools
                .licences_to_accept(&sdk, &wanted)
                .into_iter()
                .next()
                .map(|(id, text)| LicenceInfo { id, text }))
        })
        .await
    }

    /// Downloads and installs the missing tools, and with `update`, newer
    /// versions of installed ones, which needs every device stopped. Their
    /// licence must be accepted first. Progress is for all of them together.
    pub async fn install_tools(
        &self,
        update: bool,
        listener: Arc<dyn DownloadListener>,
    ) -> Result<(), AaeError> {
        let (sdk, store) = (self.sdk.clone(), self.store.clone());
        on_runtime(async move {
            let tools = load_tools(false).await?;
            let wanted: Vec<setup::Tool> = wanted_tools(&tools, &sdk, update)
                .into_iter()
                .cloned()
                .collect();
            if update {
                let running: Vec<String> = store
                    .list()?
                    .into_iter()
                    .filter(|d| emulator::running(d).is_ok())
                    .map(|d| d.meta.name)
                    .collect();
                if !running.is_empty() {
                    return Err(AaeError::Failed {
                        message: format!(
                            "Stop these devices first, as the emulator and adb are replaced: {}.",
                            running.join(", ")
                        ),
                    });
                }
            }
            tokio::task::spawn_blocking(move || {
                let total: u64 = wanted.iter().map(|t| t.size).sum::<u64>().max(1);
                let mut before = 0u64;
                let mut last = u32::MAX;
                for tool in &wanted {
                    listener.stage(format!("Downloading {}.", tool.name));
                    setup::install(&sdk, &tools, tool, |p| match p {
                        InstallProgress::Downloading { done, .. } => {
                            let percent = ((before + done) * 100 / total) as u32;
                            if percent != last {
                                last = percent;
                                listener.downloaded(percent);
                            }
                        }
                        InstallProgress::Verifying => {
                            listener.stage(format!("Checking {}.", tool.name))
                        }
                        InstallProgress::Unpacking => {
                            listener.stage(format!("Unpacking {}.", tool.name))
                        }
                    })?;
                    before += tool.size;
                }
                Ok::<(), aae_core::error::Error>(())
            })
            .await
            .map_err(|e| AaeError::Failed {
                message: e.to_string(),
            })??;
            Ok(())
        })
        .await
    }

    pub fn devices(&self) -> Result<Vec<DeviceInfo>, AaeError> {
        Ok(self
            .store
            .list()?
            .iter()
            .map(DeviceInfo::from_device)
            .collect())
    }

    /// Installed Android versions that run at full speed on this computer, newest first.
    pub fn images(&self) -> Vec<ImageInfo> {
        self.sdk
            .system_images()
            .into_iter()
            .filter(|i| i.runs_natively())
            .map(|i| ImageInfo {
                sysdir: i.sysdir.clone(),
                api: i.api,
                description: i.to_string(),
            })
            .collect()
    }

    /// Every Android version for this computer: installed ones, and ones Google
    /// offers to download. Newest first. Reads Google's list at most once a day
    /// unless `refresh` is true; works offline from the last list.
    pub async fn versions(&self, refresh: bool) -> Result<Vec<VersionInfo>, AaeError> {
        let sdk = self.sdk.clone();
        on_runtime(async move {
            let catalogue = tokio::task::spawn_blocking(move || Catalogue::load(refresh))
                .await
                .map_err(|e| AaeError::Failed {
                    message: e.to_string(),
                })?;
            let mut versions: Vec<VersionInfo> = sdk
                .system_images()
                .into_iter()
                .filter(|i| i.runs_natively())
                .map(|i| VersionInfo {
                    api: i.api,
                    tag: i.tag.clone(),
                    description: i.to_string(),
                    installed: true,
                    size: String::new(),
                    sysdir: i.sysdir.clone(),
                })
                .collect();
            // Without the internet, the installed versions are still offered.
            if let Ok(catalogue) = catalogue {
                for image in catalogue.images.iter().filter(|i| !i.is_installed(&sdk)) {
                    versions.push(VersionInfo {
                        api: image.api,
                        tag: image.tag.clone(),
                        description: image.describe(),
                        installed: false,
                        size: human_size(image.size),
                        sysdir: String::new(),
                    });
                }
            }
            versions.sort_by(|a, b| b.api.cmp(&a.api).then(a.tag.cmp(&b.tag)));
            Ok(versions)
        })
        .await
    }

    /// The licence that must be accepted before downloading this version, or
    /// nothing if it's accepted already or the version is installed.
    pub async fn licence_to_accept(
        &self,
        api: u32,
        tag: String,
    ) -> Result<Option<LicenceInfo>, AaeError> {
        let sdk = self.sdk.clone();
        on_runtime(async move {
            let catalogue = load_catalogue().await?;
            let Some(image) = catalogue.find(api, &tag) else {
                return Ok(None);
            };
            if image.is_installed(&sdk) {
                return Ok(None);
            }
            let text = catalogue
                .licences
                .get(&image.licence_id)
                .cloned()
                .unwrap_or_default();
            Ok(
                (!catalog::licence_accepted(&sdk, &image.licence_id, &text)).then(|| LicenceInfo {
                    id: image.licence_id.clone(),
                    text,
                }),
            )
        })
        .await
    }

    /// Records that the user accepted this licence text.
    pub fn accept_licence(&self, licence: LicenceInfo) -> Result<(), AaeError> {
        Ok(catalog::accept_licence(
            &self.sdk,
            &licence.id,
            &licence.text,
        )?)
    }

    /// Downloads and installs an Android version. Its licence must be accepted first.
    pub async fn install_version(
        &self,
        api: u32,
        tag: String,
        listener: Arc<dyn DownloadListener>,
    ) -> Result<ImageInfo, AaeError> {
        let sdk = self.sdk.clone();
        on_runtime(async move {
            let catalogue = load_catalogue().await?;
            let image = catalogue
                .find(api, &tag)
                .cloned()
                .ok_or_else(|| AaeError::Failed {
                    message: format!(
                        "Google doesn't offer {} of that kind for this computer.",
                        android_name(api)
                    ),
                })?;
            let installed = tokio::task::spawn_blocking(move || {
                let mut last = u32::MAX;
                catalog::install(&sdk, &catalogue, &image, |p| match p {
                    InstallProgress::Downloading { .. } => {
                        let percent = p.percent().unwrap_or(0);
                        if percent != last {
                            last = percent;
                            listener.downloaded(percent);
                        }
                    }
                    InstallProgress::Verifying => listener.stage("Checking the download.".into()),
                    InstallProgress::Unpacking => listener.stage("Unpacking.".into()),
                })
            })
            .await
            .map_err(|e| AaeError::Failed {
                message: e.to_string(),
            })??;
            Ok(ImageInfo {
                sysdir: installed.sysdir.clone(),
                api: installed.api,
                description: installed.to_string(),
            })
        })
        .await
    }

    /// The screen reader APK used when none is chosen, if there is one.
    pub fn default_screen_reader(&self) -> Option<String> {
        provision::default_screen_reader_apk().map(|p| p.display().to_string())
    }

    pub fn create_device(
        &self,
        name: String,
        sysdir: String,
        profile: DeviceProfile,
    ) -> Result<DeviceInfo, AaeError> {
        let image = self
            .sdk
            .system_images()
            .into_iter()
            .find(|i| i.sysdir == sysdir)
            .ok_or_else(|| AaeError::Failed {
                message: "That Android version is no longer installed.".into(),
            })?;
        let device = self.store.create(&name, &image, profile.into())?;
        Ok(DeviceInfo::from_device(&device))
    }

    /// Starts a device and waits until it is ready, setting it up on its first
    /// start. `screen_reader_apk` and `volume_boost` apply only to that first start.
    pub async fn start_device(
        &self,
        id: String,
        screen_reader_apk: Option<String>,
        volume_boost: bool,
        listener: Arc<dyn ProgressListener>,
    ) -> Result<(), AaeError> {
        let (sdk, store) = (self.sdk.clone(), self.store.clone());
        on_runtime(async move {
            let mut device = store.get(&id)?;
            let name = device.meta.name.clone();
            let setup = ProvisionOptions {
                screen_reader_apk: screen_reader_apk.map(PathBuf::from),
                full_volume: volume_boost,
                ..Default::default()
            };
            lifecycle::start_device(
                &sdk,
                &store,
                &mut device,
                &StartOptions::default(),
                &setup,
                |p| listener.progress(p.describe(&name)),
            )
            .await?;
            Ok(())
        })
        .await
    }

    /// Restarts Android on a running device, keeping everything on it, then
    /// checks the screen reader, keyboard and speech as a start does.
    pub async fn restart_device(
        &self,
        id: String,
        listener: Arc<dyn ProgressListener>,
    ) -> Result<(), AaeError> {
        let (sdk, store) = (self.sdk.clone(), self.store.clone());
        on_runtime(async move {
            let mut device = store.get(&id)?;
            let name = device.meta.name.clone();
            listener.progress(format!("Restarting Android on {name}."));
            emulator::reboot(&sdk, &device, lifecycle::BOOT_TIMEOUT).await?;
            lifecycle::start_device(
                &sdk,
                &store,
                &mut device,
                &StartOptions::default(),
                &ProvisionOptions::default(),
                |p| listener.progress(p.describe(&name)),
            )
            .await?;
            Ok(())
        })
        .await
    }

    /// Wipes a device back to its first-boot state and sets it up again,
    /// with its screen reader. Its apps, data and snapshots go; its name,
    /// hardware and volume stay.
    pub async fn wipe_device(
        &self,
        id: String,
        listener: Arc<dyn ProgressListener>,
    ) -> Result<(), AaeError> {
        let (sdk, store) = (self.sdk.clone(), self.store.clone());
        on_runtime(async move {
            let mut device = store.get(&id)?;
            let name = device.meta.name.clone();
            lifecycle::wipe_device(&sdk, &store, &mut device, |p| {
                listener.progress(p.describe(&name))
            })
            .await?;
            Ok(())
        })
        .await
    }

    /// Stops a device, saving its state so it starts quickly next time.
    pub async fn stop_device(&self, id: String) -> Result<(), AaeError> {
        let (sdk, store) = (self.sdk.clone(), self.store.clone());
        on_runtime(async move {
            let device = store.get(&id)?;
            emulator::stop(&sdk, &device, std::time::Duration::from_secs(60)).await?;
            Ok(())
        })
        .await
    }

    /// Every installed Android version, newest first, with its size and the
    /// devices that use it.
    pub async fn installed_images(&self) -> Result<Vec<InstalledImageInfo>, AaeError> {
        let (sdk, store) = (self.sdk.clone(), self.store.clone());
        on_runtime(async move {
            tokio::task::spawn_blocking(move || {
                sdk.system_images()
                    .into_iter()
                    .map(|image| {
                        let users = catalog::image_users(&sdk, &store, &image)?;
                        Ok(InstalledImageInfo {
                            sysdir: image.sysdir.clone(),
                            description: image.to_string(),
                            size: human_size(catalog::image_size(&image)),
                            devices: users.devices,
                            other_devices: users.others,
                        })
                    })
                    .collect::<Result<Vec<_>, AaeError>>()
            })
            .await
            .map_err(|e| AaeError::Failed {
                message: e.to_string(),
            })?
        })
        .await
    }

    /// Deletes an installed Android version. Refused while any of AAE's
    /// devices use it. Returns the space freed, in words.
    pub async fn remove_image(&self, sysdir: String) -> Result<String, AaeError> {
        let (sdk, store) = (self.sdk.clone(), self.store.clone());
        on_runtime(async move {
            tokio::task::spawn_blocking(move || {
                let image = sdk
                    .system_images()
                    .into_iter()
                    .find(|i| i.sysdir == sysdir)
                    .ok_or_else(|| AaeError::Failed {
                        message: "That Android version isn't installed any more.".into(),
                    })?;
                Ok(human_size(catalog::remove(&sdk, &store, &image)?))
            })
            .await
            .map_err(|e| AaeError::Failed {
                message: e.to_string(),
            })?
        })
        .await
    }

    /// Deletes a device and its files. Returns how much space was freed, in words.
    pub fn delete_device(&self, id: String) -> Result<String, AaeError> {
        let device = self.store.get(&id)?;
        Ok(human_size(self.store.delete(&device)?))
    }

    /// How much disk space a device uses, in words.
    pub fn device_size(&self, id: String) -> Result<String, AaeError> {
        Ok(human_size(self.store.get(&id)?.disk_usage()))
    }

    pub fn clone_device(&self, id: String, new_name: String) -> Result<DeviceInfo, AaeError> {
        let source = self.store.get(&id)?;
        Ok(DeviceInfo::from_device(
            &self.store.clone_device(&source, &new_name)?,
        ))
    }

    pub fn rename_device(&self, id: String, new_name: String) -> Result<DeviceInfo, AaeError> {
        let mut device = self.store.get(&id)?;
        self.store.rename(&mut device, &new_name)?;
        Ok(DeviceInfo::from_device(&device))
    }

    /// Installs a screen reader on a running device and makes it the device's
    /// screen reader. Returns its package.
    pub async fn add_screen_reader(
        &self,
        id: String,
        source: ScreenReaderSource,
    ) -> Result<String, AaeError> {
        let (sdk, store) = (self.sdk.clone(), self.store.clone());
        on_runtime(async move {
            let mut device = store.get(&id)?;
            let (_, _, adb) = emulator::attach(&sdk, &device).await?;
            let apk = match source {
                ScreenReaderSource::Backtalk => {
                    provision::download_backtalk(&sdk, device.meta.api).await?
                }
                ScreenReaderSource::Apk { path } => PathBuf::from(path),
            };
            Ok(provision::add_screen_reader(&sdk, &mut device, &adb, &apk, false).await?)
        })
        .await
    }

    /// The package name of an app file.
    pub fn apk_package(&self, path: String) -> Result<String, AaeError> {
        Ok(provision::read_apk(&self.sdk, std::path::Path::new(&path))?.package)
    }

    /// Installs a screen reader build on a device and makes it its screen
    /// reader; a new build of the same screen reader keeps its settings. A
    /// stopped device gets it when it next starts. With `replace`, a copy
    /// signed differently is removed first, losing its settings. Says what
    /// happened.
    pub async fn install_screen_reader_build(
        &self,
        id: String,
        path: String,
        replace: bool,
    ) -> Result<String, AaeError> {
        let (sdk, store) = (self.sdk.clone(), self.store.clone());
        on_runtime(async move {
            let mut device = store.get(&id)?;
            let name = device.meta.name.clone();
            let apk = PathBuf::from(path);
            if emulator::running(&device).is_ok() {
                let (_, _, adb) = emulator::attach(&sdk, &device).await?;
                let package =
                    provision::add_screen_reader(&sdk, &mut device, &adb, &apk, replace).await?;
                Ok(format!("Installed {package} on {name}."))
            } else {
                let package = provision::queue_screen_reader(&sdk, &mut device, &apk)?;
                Ok(format!(
                    "{name} is stopped, so {package} will be installed when it next starts."
                ))
            }
        })
        .await
    }

    /// Records that the user wants no screen reader on this device, so AAE
    /// doesn't offer one again.
    pub fn decline_screen_reader(&self, id: String) -> Result<(), AaeError> {
        let mut device = self.store.get(&id)?;
        device.meta.screen_reader_declined = true;
        Ok(device.save_meta()?)
    }

    /// Connects to a running device, for its keyboard, audio and controls.
    pub async fn open_session(&self, id: String) -> Result<Arc<Session>, AaeError> {
        let (sdk, store) = (self.sdk.clone(), self.store.clone());
        on_runtime(async move {
            let device = store.get(&id)?;
            let (_, controller, adb) = emulator::attach(&sdk, &device).await?;
            Ok(Session::new(sdk, device, controller, adb))
        })
        .await
    }
}

/// True when AAE_KEYLOG=1 asks for keys to be logged, for diagnosing keyboard problems.
fn key_logging() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("AAE_KEYLOG").as_deref() == Ok("1"))
}

fn add_rows(node: &inspector::Node, parent: u32, rows: &mut Vec<InspectorRow>) {
    let index = rows.len() as u32;
    rows.push(InspectorRow {
        index,
        parent: Some(parent),
        summary: node.summary(),
        details: node.details(),
    });
    for child in &node.children {
        add_rows(child, index, rows);
    }
}

async fn load_tools(refresh: bool) -> Result<setup::Tools, AaeError> {
    tokio::task::spawn_blocking(move || setup::Tools::load(refresh))
        .await
        .map_err(|e| AaeError::Failed {
            message: e.to_string(),
        })?
        .map_err(AaeError::from)
}

/// The tools to install: the missing ones, and with `update`, newer versions.
fn wanted_tools<'a>(tools: &'a setup::Tools, sdk: &Sdk, update: bool) -> Vec<&'a setup::Tool> {
    let mut wanted = tools.missing(sdk);
    if update {
        wanted.extend(tools.updates(sdk));
    }
    wanted
}

async fn load_catalogue() -> Result<Catalogue, AaeError> {
    tokio::task::spawn_blocking(|| Catalogue::load(false))
        .await
        .map_err(|e| AaeError::Failed {
            message: e.to_string(),
        })?
        .map_err(AaeError::from)
}

enum KeyMessage {
    Evdev(i32, bool),
}

/// A connection to one running device.
#[derive(uniffi::Object)]
pub struct Session {
    sdk: Sdk,
    device: Mutex<Device>,
    controller: Controller,
    adb: Adb,
    keys: mpsc::UnboundedSender<KeyMessage>,
    audio: Mutex<Option<AudioPlayer>>,
    logs: Mutex<Option<LogStream>>,
    /// The screen's size and density, read on the first gesture.
    screen: tokio::sync::Mutex<Option<gestures::Screen>>,
    /// The touches that lift fingers a held gesture left down.
    held: tokio::sync::Mutex<Option<Vec<aae_core::control::TouchPoint>>>,
}

impl Session {
    fn new(sdk: Sdk, device: Device, controller: Controller, adb: Adb) -> Arc<Self> {
        // Keys go through one queue, sent one at a time, so they reach the
        // device in the order they were pressed however fast they come.
        let (tx, mut rx) = mpsc::unbounded_channel::<KeyMessage>();
        let sender = controller.clone();
        runtime().spawn(async move {
            while let Some(KeyMessage::Evdev(code, down)) = rx.recv().await {
                if let Err(e) = sender.evdev_key(code, down).await {
                    tracing::warn!("a key could not be sent: {e}");
                }
            }
        });
        Arc::new(Session {
            sdk,
            device: Mutex::new(device),
            controller,
            adb,
            keys: tx,
            audio: Mutex::new(None),
            logs: Mutex::new(None),
            screen: tokio::sync::Mutex::new(None),
            held: tokio::sync::Mutex::new(None),
        })
    }
}

#[uniffi::export]
impl Session {
    pub fn device_name(&self) -> String {
        self.device.lock().unwrap().meta.name.clone()
    }

    /// Sends a key by its macOS virtual key code. Returns false if Android has
    /// no such key. Never blocks.
    pub fn mac_key(&self, keycode: u16, down: bool) -> bool {
        let code = keys::mac_to_evdev(keycode);
        // Key codes reveal what was typed, so they're only logged on request.
        if key_logging() {
            tracing::debug!(
                "Mac key {keycode:#04x} {} -> Android key {code:?}",
                if down { "down" } else { "up" }
            );
        }
        match code {
            Some(code) => self.keys.send(KeyMessage::Evdev(code, down)).is_ok(),
            None => false,
        }
    }

    /// Presses a key by name, such as "back", "home" or "meta+right".
    pub async fn press(&self, name: String) -> Result<(), AaeError> {
        let key = keys::parse(&name).ok_or_else(|| AaeError::Failed {
            message: format!("\"{name}\" is not a key name."),
        })?;
        let controller = self.controller.clone();
        on_runtime(async move { Ok(controller.press(&key).await?) }).await
    }

    /// Starts playing the device's audio on the Mac.
    /// Starts playing the device's audio. With `correct_pitch`, older Android
    /// versions that play slowly have their pitch raised back to normal.
    pub async fn start_audio(&self, correct_pitch: bool) -> Result<(), AaeError> {
        if self.audio.lock().unwrap().is_some() {
            return Ok(());
        }
        let controller = self.controller.clone();
        let measured = self.device.lock().unwrap().meta.audio_speed.unwrap_or(1.0);
        let speed = if correct_pitch { measured } else { 1.0 };
        let player =
            on_runtime(async move { Ok(AudioPlayer::start_with_speed(&controller, speed).await?) })
                .await?;
        player.set_volume(self.audio_volume());
        *self.audio.lock().unwrap() = Some(player);
        Ok(())
    }

    /// How loud AAE plays this device on the Mac, from 0 to 1.
    pub fn audio_volume(&self) -> f32 {
        self.device
            .lock()
            .unwrap()
            .meta
            .playback_volume
            .unwrap_or(1.0)
    }

    /// Checks the device's sound is reaching AAE. Without `probe`, only
    /// what can be told without a sound; with it, AAE's helper also plays a
    /// test tone, with AAE's playback muted so nobody hears it.
    pub async fn check_audio(&self, probe: bool) -> Result<AudioCheck, AaeError> {
        let adb = self.adb.clone();
        // The player can't cross to the runtime, so wait here, off the lock.
        let player = self.audio.lock().unwrap().take();
        let Some(player) = player else {
            return Ok(AudioCheck {
                working: false,
                message: "AAE isn't playing this device's audio.".into(),
            });
        };
        let (player, health) = on_runtime(async move {
            let health = if probe {
                player.probe(&adb).await
            } else {
                player.check(&adb).await
            };
            Ok((player, health))
        })
        .await?;
        *self.audio.lock().unwrap() = Some(player);
        Ok(match health {
            aae_core::audio::AudioHealth::Working => AudioCheck {
                working: true,
                message: "The device's sound is reaching AAE.".into(),
            },
            aae_core::audio::AudioHealth::Quiet => AudioCheck {
                working: true,
                message: "Nothing's wrong, but the device hasn't played anything lately.".into(),
            },
            aae_core::audio::AudioHealth::Broken(why) => AudioCheck {
                working: false,
                message: why,
            },
        })
    }

    /// Restarts AAE's side of the device's audio: a new connection to the
    /// emulator and to the Mac's output. The device itself isn't touched.
    pub async fn restart_audio(&self, correct_pitch: bool) -> Result<(), AaeError> {
        let muted = self
            .audio
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|p| p.is_muted());
        self.stop_audio();
        self.start_audio(correct_pitch).await?;
        if muted {
            self.toggle_mute();
        }
        Ok(())
    }

    pub fn stop_audio(&self) {
        if let Some(mut player) = self.audio.lock().unwrap().take() {
            player.stop();
        }
    }

    /// Sets how loud AAE plays this device on the Mac, from 0 to 1, and
    /// remembers it for the device.
    pub fn set_audio_volume(&self, volume: f32) -> Result<(), AaeError> {
        let volume = volume.clamp(0.0, 1.0);
        if let Some(player) = self.audio.lock().unwrap().as_ref() {
            player.set_volume(volume);
        }
        let mut device = self.device.lock().unwrap();
        device.meta.playback_volume = (volume < 1.0).then_some(volume);
        Ok(device.save_meta()?)
    }

    /// Silences this device while another is the one in use, or plays it
    /// again. Separate from the user's own mute.
    pub fn set_background(&self, background: bool) {
        if let Some(player) = self.audio.lock().unwrap().as_ref() {
            player.set_background(background);
        }
    }

    /// Mutes or unmutes the device's audio. Returns true if now muted.
    pub fn toggle_mute(&self) -> bool {
        self.audio
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(AudioPlayer::toggle_mute)
    }

    /// Sets the screen reader's volume on the device, from 0 to 100.
    pub async fn set_screen_reader_volume(&self, percent: u8) -> Result<i32, AaeError> {
        let mut device = self.device.lock().unwrap().clone();
        let adb = self.adb.clone();
        let index = on_runtime(async move {
            let index = provision::boost_volume(&mut device, &adb, percent).await?;
            Ok((index, device))
        })
        .await?;
        *self.device.lock().unwrap() = index.1;
        Ok(index.0)
    }

    /// Rotates the device a quarter turn. Returns the new orientation in words.
    pub async fn rotate(&self, left: bool) -> Result<String, AaeError> {
        let controller = self.controller.clone();
        on_runtime(async move {
            let now = controller.orientation().await?;
            let next = if left {
                now.turned_left()
            } else {
                now.turned_right()
            };
            controller.set_orientation(next).await?;
            Ok(next.describe().to_string())
        })
        .await
    }

    /// Reads the screen's accessibility tree, as a screen reader sees it, and
    /// checks it for common problems.
    pub async fn inspect(&self) -> Result<Inspection, AaeError> {
        let device = self.device.lock().unwrap().clone();
        let (sdk, adb) = (self.sdk.clone(), self.adb.clone());
        on_runtime(async move {
            let helper = provision::HELPER_COMPONENT.to_string();
            let kept = device.meta.keep_enabled.contains(&helper);
            provision::update_helper(&sdk, &adb).await?;
            adb.ensure_services(std::slice::from_ref(&helper), provision::SERVICE_TIMEOUT)
                .await?;
            let tree = inspector::read_tree(&adb).await;
            if !kept {
                adb.disable_service(&helper).await?;
            }
            let tree = tree?;
            let mut rows = Vec::new();
            for window in &tree.windows {
                let index = rows.len() as u32;
                rows.push(InspectorRow {
                    index,
                    parent: None,
                    summary: window.describe(),
                    details: vec![format!("Window type: {}", window.kind)],
                });
                add_rows(&window.root, index, &mut rows);
            }
            let issues = inspector::check(&tree)
                .into_iter()
                .map(|i| IssueInfo {
                    error: i.severity == inspector::Severity::Error,
                    message: i.message,
                    element: i.element,
                    id: i.id,
                })
                .collect();
            Ok(Inspection {
                rows,
                issues,
                text: inspector::to_text(&tree),
                json: serde_json::to_string_pretty(&tree).unwrap_or_default(),
            })
        })
        .await
    }

    /// Turns the speech log on or off. Returns what happened, in words.
    pub async fn set_speech_log(&self, on: bool) -> Result<String, AaeError> {
        let mut device = self.device.lock().unwrap().clone();
        let (sdk, adb) = (self.sdk.clone(), self.adb.clone());
        let (message, device) = on_runtime(async move {
            let reader = device.meta.screen_reader.clone();
            let message = match (on, device.meta.speech_log_engine.clone()) {
                (true, Some(_)) => "The speech log is already on.".to_string(),
                (false, None) => "The speech log is already off.".to_string(),
                (true, None) => {
                    provision::update_helper(&sdk, &adb).await?;
                    let engine = tts::start_speech_log(&adb, reader.as_deref()).await?;
                    device.meta.speech_log_engine = Some(engine);
                    device.save_meta()?;
                    "The speech log is on.".to_string()
                }
                (false, Some(engine)) => {
                    tts::stop_speech_log(&adb, &engine, reader.as_deref()).await?;
                    device.meta.speech_log_engine = None;
                    device.save_meta()?;
                    "The speech log is off.".to_string()
                }
            };
            Ok((message, device))
        })
        .await?;
        *self.device.lock().unwrap() = device;
        Ok(message)
    }

    /// Whether the speech log is on for this device.
    pub fn speech_log_on(&self) -> bool {
        self.device.lock().unwrap().meta.speech_log_engine.is_some()
    }

    /// What the screen reader said after `since` (milliseconds since 1970).
    /// With `clear`, the log is emptied too.
    pub async fn speech_log(
        &self,
        since: u64,
        clear: bool,
    ) -> Result<Vec<UtteranceInfo>, AaeError> {
        let adb = self.adb.clone();
        on_runtime(async move {
            Ok(tts::speech_log(&adb, since, clear)
                .await?
                .into_iter()
                .map(|u| UtteranceInfo {
                    time: u.time,
                    clock: tts::clock_time(u.time),
                    text: u.text,
                })
                .collect())
        })
        .await
    }

    /// Runs a command typed by the user in the device's shell, giving up after
    /// two minutes. A command that fails still returns what it printed.
    pub async fn run_command(&self, command: String) -> Result<CommandResult, AaeError> {
        let adb = self.adb.clone();
        on_runtime(async move {
            let (output, status) = adb
                .run_command(&command, std::time::Duration::from_secs(120))
                .await?;
            Ok(CommandResult { output, status })
        })
        .await
    }

    /// Performs a screen reader gesture by name, such as "swipe-right",
    /// "swipe-up-then-left", "double-tap" or "two-finger-swipe-down".
    /// Gestures run one at a time, in the order asked for.
    /// It happens at `at`, in pixels of the screen as the user sees it, or
    /// in the middle of the screen.
    pub async fn perform_gesture(
        self: Arc<Self>,
        name: String,
        at: Option<ScreenPoint>,
    ) -> Result<(), AaeError> {
        self.gesture(name, at, false).await
    }

    /// Performs a gesture, such as "double-tap-hold", but leaves its last
    /// touch down until `release_gesture`.
    pub async fn press_gesture(
        self: Arc<Self>,
        name: String,
        at: Option<ScreenPoint>,
    ) -> Result<(), AaeError> {
        self.gesture(name, at, true).await
    }

    /// The things on the screen that can be touched, in reading order, and
    /// the screen's size as the user sees it. AAE's helper must be on; see
    /// `use_helper`.
    pub async fn touch_targets(self: Arc<Self>) -> Result<TouchTargets, AaeError> {
        on_runtime(async move {
            let screen = self.screen().await?;
            let tree = inspector::read_tree(&self.adb).await?;
            let (width, height) = gestures::user_size(&screen);
            Ok(TouchTargets {
                width: width as i32,
                height: height as i32,
                targets: inspector::targets(&tree)
                    .into_iter()
                    .map(|t| {
                        let (x, y) = t.centre();
                        let [left, top, right, bottom] = t.bounds;
                        TouchTarget {
                            label: t.label,
                            x,
                            y,
                            left,
                            top,
                            right,
                            bottom,
                        }
                    })
                    .collect(),
            })
        })
        .await
    }

    /// Turns AAE's helper on (true), for reading the screen, or back to how
    /// the device keeps it (false).
    pub async fn use_helper(&self, on: bool) -> Result<(), AaeError> {
        let device = self.device.lock().unwrap().clone();
        let adb = self.adb.clone();
        on_runtime(async move {
            let helper = provision::HELPER_COMPONENT.to_string();
            if on {
                adb.ensure_services(std::slice::from_ref(&helper), provision::SERVICE_TIMEOUT)
                    .await?;
            } else if !device.meta.keep_enabled.contains(&helper) {
                adb.disable_service(&helper).await?;
            }
            Ok(())
        })
        .await
    }

    /// Lifts the fingers `press_gesture` left down, if any are.
    pub async fn release_gesture(self: Arc<Self>) -> Result<(), AaeError> {
        on_runtime(async move {
            if let Some(release) = self.held.lock().await.take() {
                gestures::lift(&self.controller, &release).await?;
            }
            Ok(())
        })
        .await
    }

    /// The snapshots saved of this device, newest first.
    pub async fn snapshots(&self) -> Result<Vec<SnapshotInfo>, AaeError> {
        let controller = self.controller.clone();
        on_runtime(async move {
            Ok(controller
                .snapshots()
                .await?
                .into_iter()
                .map(|s| SnapshotInfo {
                    taken: s.taken(),
                    size: human_size(s.size),
                    id: s.id,
                    name: s.name,
                    notes: s.notes,
                    loaded: s.loaded,
                    compatible: s.compatible,
                })
                .collect())
        })
        .await
    }

    /// Saves the device as it is now, under a name, with notes.
    pub async fn save_snapshot(&self, name: String, notes: String) -> Result<(), AaeError> {
        let controller = self.controller.clone();
        on_runtime(async move {
            controller.save_named_snapshot(&name, &notes).await?;
            Ok(())
        })
        .await
    }

    /// Puts the device back as it was in a snapshot.
    pub async fn load_snapshot(&self, id: String) -> Result<(), AaeError> {
        let controller = self.controller.clone();
        on_runtime(async move { Ok(controller.load_snapshot(&id).await?) }).await
    }

    /// Changes a snapshot's name and notes.
    pub async fn update_snapshot(
        &self,
        id: String,
        name: String,
        notes: String,
    ) -> Result<(), AaeError> {
        let controller = self.controller.clone();
        on_runtime(async move { Ok(controller.update_snapshot(&id, &name, &notes).await?) }).await
    }

    pub async fn delete_snapshot(&self, id: String) -> Result<(), AaeError> {
        let controller = self.controller.clone();
        on_runtime(async move { Ok(controller.delete_snapshot(&id).await?) }).await
    }

    /// The text on the device's clipboard.
    pub async fn device_clipboard(&self) -> Result<String, AaeError> {
        let controller = self.controller.clone();
        on_runtime(async move { Ok(controller.clipboard().await?) }).await
    }

    /// Puts text on the device's clipboard.
    pub async fn set_device_clipboard(&self, text: String) -> Result<(), AaeError> {
        let controller = self.controller.clone();
        on_runtime(async move { Ok(controller.set_clipboard(&text).await?) }).await
    }

    /// Types text on the device as key presses, for fields that block pasting.
    pub async fn type_text(&self, text: String) -> Result<(), AaeError> {
        let controller = self.controller.clone();
        on_runtime(async move { Ok(controller.type_text(&text).await?) }).await
    }

    /// Sets the battery level, from 0 to 100, and whether it's charging.
    pub async fn set_battery(&self, level: u32, charging: bool) -> Result<(), AaeError> {
        let controller = self.controller.clone();
        on_runtime(async move {
            Ok(controller
                .set_battery(level.min(100) as i32, charging)
                .await?)
        })
        .await
    }

    /// Sets where the device thinks it is.
    pub async fn set_location(&self, latitude: f64, longitude: f64) -> Result<(), AaeError> {
        let controller = self.controller.clone();
        on_runtime(async move { Ok(controller.set_location(latitude, longitude).await?) }).await
    }

    /// Sends the device a text message.
    pub async fn send_sms(&self, from: String, text: String) -> Result<(), AaeError> {
        let controller = self.controller.clone();
        on_runtime(async move { Ok(controller.send_sms(&from, &text).await?) }).await
    }

    /// Plays the other end of a phone call.
    pub async fn phone_call(&self, action: CallAction, number: String) -> Result<(), AaeError> {
        use aae_core::proto::android::emulation::control::phone_call::Operation;
        let operation = match action {
            CallAction::Ring => Operation::InitCall,
            CallAction::HangUp => Operation::DisconnectCall,
            CallAction::Answer => Operation::AcceptCall,
            CallAction::Busy => Operation::RejectCallBusy,
            CallAction::Hold => Operation::PlaceCallOnHold,
            CallAction::Resume => Operation::TakeCallOffHold,
        };
        let controller = self.controller.clone();
        on_runtime(async move { Ok(controller.phone(operation, &number).await?) }).await
    }

    /// The accessibility services installed on the device, screen readers first.
    pub async fn list_services(&self) -> Result<Vec<ServiceInfo>, AaeError> {
        let device = self.device.lock().unwrap().clone();
        let (sdk, adb) = (self.sdk.clone(), self.adb.clone());
        on_runtime(async move {
            provision::update_helper(&sdk, &adb).await?;
            Ok(aae_core::services::list(&adb, &device)
                .await?
                .into_iter()
                .map(|s| ServiceInfo {
                    component: s.component,
                    label: s.label,
                    description: s.description,
                    screen_reader: s.screen_reader,
                    on: s.on,
                    current_screen_reader: s.current_screen_reader,
                })
                .collect())
        })
        .await
    }

    /// Turns a service on, to stay on, or off. Turning on a screen reader
    /// makes it the device's screen reader instead of the one it had.
    pub async fn set_service(&self, component: String, on: bool) -> Result<(), AaeError> {
        let mut device = self.device.lock().unwrap().clone();
        let adb = self.adb.clone();
        let device = on_runtime(async move {
            use aae_core::services;
            let list = services::list(&adb, &device).await?;
            let service =
                services::find(&list, &component)
                    .cloned()
                    .ok_or_else(|| AaeError::Failed {
                        message: "That service isn't installed any more.".into(),
                    })?;
            services::set(&mut device, &adb, &service, on).await?;
            Ok(device)
        })
        .await?;
        *self.device.lock().unwrap() = device;
        Ok(())
    }

    /// The installed apps, by name; with `system`, Android's own too.
    pub async fn list_apps(&self, system: bool) -> Result<Vec<AppInfo>, AaeError> {
        let (sdk, adb) = (self.sdk.clone(), self.adb.clone());
        on_runtime(async move {
            provision::update_helper(&sdk, &adb).await?;
            Ok(aae_core::apps::list(&adb, system)
                .await?
                .into_iter()
                .map(|a| AppInfo {
                    package: a.package,
                    label: a.label,
                    version: a.version,
                    system: a.system,
                    enabled: a.enabled,
                    launchable: a.launchable,
                })
                .collect())
        })
        .await
    }

    /// An app's permissions and special access, and which it has.
    pub async fn app_permissions(&self, package: String) -> Result<AppPermissions, AaeError> {
        let adb = self.adb.clone();
        on_runtime(async move {
            use aae_core::apps;
            let permissions = apps::permissions(&adb, &package)
                .await?
                .into_iter()
                .map(|p| PermissionInfo {
                    name: p.name,
                    label: p.label,
                    granted: p.granted,
                })
                .collect();
            let mut access = Vec::new();
            for kind in apps::Access::ALL {
                access.push(AccessInfo {
                    kind: kind.into(),
                    name: kind.describe().to_string(),
                    allowed: apps::has_access(&adb, &package, kind).await?,
                });
            }
            Ok(AppPermissions {
                permissions,
                access,
            })
        })
        .await
    }

    pub async fn set_app_permission(
        &self,
        package: String,
        permission: String,
        grant: bool,
    ) -> Result<(), AaeError> {
        let adb = self.adb.clone();
        on_runtime(async move {
            Ok(aae_core::apps::set_permission(&adb, &package, &permission, grant).await?)
        })
        .await
    }

    /// Grants every permission the app asks for. Returns how many.
    pub async fn grant_all_permissions(&self, package: String) -> Result<u32, AaeError> {
        let adb = self.adb.clone();
        on_runtime(async move { Ok(aae_core::apps::grant_all(&adb, &package).await? as u32) }).await
    }

    pub async fn set_app_access(
        &self,
        package: String,
        kind: AccessKind,
        allowed: bool,
    ) -> Result<(), AaeError> {
        let adb = self.adb.clone();
        on_runtime(async move {
            Ok(aae_core::apps::set_access(&adb, &package, kind.into(), allowed).await?)
        })
        .await
    }

    pub async fn open_app(&self, package: String) -> Result<(), AaeError> {
        let adb = self.adb.clone();
        on_runtime(async move { Ok(aae_core::apps::open(&adb, &package).await?) }).await
    }

    /// Opens one of an app's screens, such as `com.example/.Settings`.
    pub async fn open_app_screen(&self, component: String) -> Result<(), AaeError> {
        let adb = self.adb.clone();
        on_runtime(async move { Ok(aae_core::apps::open_activity(&adb, &component).await?) }).await
    }

    /// Opens a link, in the given app's package or whichever Android chooses.
    pub async fn open_link(&self, link: String, package: Option<String>) -> Result<(), AaeError> {
        let adb = self.adb.clone();
        on_runtime(async move {
            aae_core::apps::open_link(&adb, &link, package.as_deref()).await?;
            Ok(())
        })
        .await
    }

    /// Sends an intent, and returns what Android said.
    pub async fn send_intent(&self, intent: IntentInfo) -> Result<String, AaeError> {
        let adb = self.adb.clone();
        let non_empty =
            |s: Option<String>| s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        let intent = aae_core::apps::Intent {
            action: non_empty(intent.action),
            data: non_empty(intent.data),
            target: non_empty(intent.target),
            extras: intent
                .extras
                .into_iter()
                .map(|e| (e.key, e.value))
                .collect(),
            broadcast: intent.broadcast,
        };
        on_runtime(async move { Ok(aae_core::apps::send(&adb, &intent).await?) }).await
    }

    pub async fn force_stop_app(&self, package: String) -> Result<(), AaeError> {
        let adb = self.adb.clone();
        on_runtime(async move { Ok(aae_core::apps::force_stop(&adb, &package).await?) }).await
    }

    pub async fn clear_app_data(&self, package: String) -> Result<(), AaeError> {
        let adb = self.adb.clone();
        on_runtime(async move { Ok(aae_core::apps::clear_data(&adb, &package).await?) }).await
    }

    pub async fn uninstall_app(&self, package: String) -> Result<(), AaeError> {
        let adb = self.adb.clone();
        on_runtime(async move { Ok(adb.uninstall(&package).await?) }).await
    }

    /// Starts reading the device log, if it isn't being read already.
    pub fn start_logs(&self) {
        let mut logs = self.logs.lock().unwrap();
        if logs.is_none() {
            let _runtime = runtime().enter();
            *logs = Some(LogStream::start(self.adb.clone(), 5000));
        }
    }

    /// Stops reading the device log and forgets the lines read.
    pub fn stop_logs(&self) {
        self.logs.lock().unwrap().take();
    }

    /// The last `limit` log lines after `since` that match the filter, oldest first.
    pub fn log_entries(&self, since: u64, filter: LogFilter, limit: u32) -> Vec<LogEntryInfo> {
        let logs = self.logs.lock().unwrap();
        let Some(stream) = logs.as_ref() else {
            return Vec::new();
        };
        let filter = logcat::Filter {
            process: filter.process,
            tag: filter.tag,
            level: filter.level.map(Into::into),
            text: filter.text,
        };
        stream
            .entries(since, &filter, limit as usize)
            .into_iter()
            .map(|e| LogEntryInfo {
                seq: e.seq,
                clock: e.clock().to_string(),
                level: e.level.into(),
                spoken: e.spoken(),
                line: e.to_line(),
                tag: e.tag,
                message: e.message,
                process: e.process,
            })
            .collect()
    }

    /// The number of the last log line read, or 0.
    pub fn log_latest(&self) -> u64 {
        self.logs
            .lock()
            .unwrap()
            .as_ref()
            .map_or(0, LogStream::latest)
    }

    /// The processes that have logged something, by name.
    pub fn log_processes(&self) -> Vec<String> {
        self.logs
            .lock()
            .unwrap()
            .as_ref()
            .map(LogStream::processes)
            .unwrap_or_default()
    }

    /// Forgets the log lines read so far.
    pub fn clear_logs(&self) {
        if let Some(stream) = self.logs.lock().unwrap().as_ref() {
            stream.clear();
        }
    }

    /// Why the log isn't being read right now, if it isn't.
    pub fn log_problem(&self) -> Option<String> {
        self.logs
            .lock()
            .unwrap()
            .as_ref()
            .and_then(LogStream::problem)
    }

    /// Runs a shell command on the device and returns what it printed.
    pub async fn shell(&self, command: String) -> Result<String, AaeError> {
        let adb = self.adb.clone();
        on_runtime(async move { Ok(adb.shell(&command).await?) }).await
    }

    /// Whether the device's screen reader is running right now, in words.
    pub async fn screen_reader_status(&self) -> Result<String, AaeError> {
        let device = self.device.lock().unwrap().clone();
        let adb = self.adb.clone();
        on_runtime(async move {
            let Some(reader) = device.meta.screen_reader.clone() else {
                return Ok("No screen reader is set up.".to_string());
            };
            let running = adb.running_services().await?;
            let package = reader.split('/').next().unwrap_or(&reader).to_string();
            Ok(
                if running
                    .iter()
                    .any(|c| aae_core::adb::same_component(c, &reader))
                {
                    format!("{package} is on.")
                } else {
                    format!("{package} is off.")
                },
            )
        })
        .await
    }

    /// Turns the device's kept services back on if Android turned any off.
    /// Returns what was turned back on, in words, or nothing.
    pub async fn guard_services(&self) -> Result<Option<String>, AaeError> {
        let device = self.device.lock().unwrap().clone();
        let adb = self.adb.clone();
        on_runtime(async move {
            let restored = provision::guard_services(&device, &adb).await?;
            Ok((!restored.is_empty()).then(|| format!("Turned back on: {}.", restored.join(", "))))
        })
        .await
    }

    /// Saves a screenshot as a PNG file.
    pub async fn screenshot(&self, path: String) -> Result<(), AaeError> {
        let controller = self.controller.clone();
        on_runtime(async move {
            let png = controller.screenshot_png().await?;
            std::fs::write(&path, png).map_err(|e| AaeError::Failed {
                message: format!("Could not save the screenshot to {path}: {e}"),
            })
        })
        .await
    }

    /// Installs an app and turns on any accessibility services it has. Returns
    /// what happened, in words.
    pub async fn install_apk(&self, path: String) -> Result<InstallResult, AaeError> {
        let mut device = self.device.lock().unwrap().clone();
        let (sdk, adb) = (self.sdk.clone(), self.adb.clone());
        let (result, device) = on_runtime(async move {
            let (info, parts) =
                provision::install_app(&sdk, &mut device, &adb, std::path::Path::new(&path))
                    .await?;
            let result = InstallResult {
                package: info.package,
                parts: parts.into_iter().map(AppPartInfo::from).collect(),
            };
            Ok((result, device))
        })
        .await?;
        *self.device.lock().unwrap() = device;
        Ok(result)
    }

    /// Turns app parts on or off as the user chose, and remembers the
    /// choices for this device.
    pub async fn set_app_choices(&self, choices: Vec<AppChoice>) -> Result<(), AaeError> {
        let mut device = self.device.lock().unwrap().clone();
        let adb = self.adb.clone();
        let device = on_runtime(async move {
            let choices: Vec<(provision::AppPart, bool)> =
                choices.into_iter().map(|c| (c.part.into(), c.on)).collect();
            provision::apply_choices(&mut device, &adb, &choices).await?;
            Ok(device)
        })
        .await?;
        *self.device.lock().unwrap() = device;
        Ok(())
    }
}
