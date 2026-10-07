//! `aae`: create, run and test accessible Android virtual devices from the terminal.
//!
//! Every message is a full sentence, printed on its own line, so a screen reader
//! reads it as it arrives. Errors say what went wrong and what to do.

mod attach;

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use aae_core::adb::{Adb, same_component};
use aae_core::catalog::{self, Catalogue, InstallProgress, RemoteImage};
use aae_core::control::Orientation;
use aae_core::device::{Device, DeviceStore, Profile, human_size};
use aae_core::emulator::{self, StartOptions};
use aae_core::lifecycle;
use aae_core::proto::android::emulation::control::phone_call::Operation;
use aae_core::provision::{self, ProvisionOptions};
use aae_core::sdk::{self, Sdk};
use aae_core::{control::Controller, keys};
use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(
    name = "aae",
    version,
    about = "Accessible Android Emulator",
    propagate_version = true
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, PartialEq, clap::ValueEnum)]
enum BridgeAction {
    On,
    Off,
    Listen,
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum MicAction {
    On,
    Check,
    Level,
    Play,
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum DaemonAction {
    Install,
    Uninstall,
    Status,
}

#[derive(Subcommand)]
enum Command {
    /// Check the Android SDK, emulator and audio, and say what is missing.
    Doctor,
    /// Run AAE's MCP server for AI agents, over standard input and output.
    Mcp {
        /// Also offer tools that delete devices, apps or data, or run shell
        /// commands on a device.
        #[arg(long)]
        allow_destructive: bool,
    },
    /// Serve this computer's devices to AAE Remote.
    ///
    /// P and Enter makes a new pairing code; Q and Enter stops.
    Serve {
        /// The port to listen on.
        #[arg(long, default_value_t = aae_remote::DEFAULT_PORT)]
        port: u16,
        /// Don't announce this computer on the local network; phones then
        /// need its address typed in.
        #[arg(long)]
        no_discovery: bool,
        /// Print events as JSON lines, for AAE's apps, which run aae serve:
        /// {"event":"serving",…}, {"event":"code","code":…}, {"event":"notice","text":…}.
        #[arg(long)]
        json: bool,
    },
    /// Make a pairing code for the running aae serve. Valid once, for 10 minutes.
    Pair {
        /// Print it as JSON, for AAE's apps.
        #[arg(long)]
        json: bool,
    },
    /// Serve devices to AAE Remote at login: install, uninstall or status.
    Daemon {
        #[arg(value_enum)]
        action: DaemonAction,
        /// Print the status as JSON, for AAE's apps.
        #[arg(long)]
        json: bool,
    },
    /// List the phones paired with this computer's AAE, or unpair one.
    Phones {
        /// The id of a phone to unpair, as the list shows it.
        #[arg(long)]
        unpair: Option<String>,
        /// List them as JSON, for AAE's apps.
        #[arg(long)]
        json: bool,
    },
    /// Check virtualisation, audio, the SDK, AAE's parts and running devices.
    SelfTest,
    /// Save a diagnostic report. Home folder, computer name and full name are removed.
    Report {
        /// Where to save it. Defaults to a dated file in this folder.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Download the emulator and SDK tools AAE needs.
    Setup {
        /// Accept the Google licence the tools are under, after reading it.
        #[arg(long)]
        accept_licence: bool,
        /// Also update the emulator and platform tools AAE installed to
        /// Google's newest stable versions. Ones installed another way, such
        /// as by Android Studio, are left to it. No device can be running.
        #[arg(long)]
        update: bool,
        /// Fetch Google's list again, instead of using the copy from today.
        #[arg(long)]
        refresh: bool,
    },
    /// List the Android versions installed on this computer.
    Images,
    /// Delete an installed Android version. Refused while a device uses it.
    RemoveImage {
        /// The version, as its API level, such as 35 or 36.1, or a preview's
        /// name, such as 37.2-beta3.
        #[arg(long)]
        api: String,
        /// The kind of Android image, when more than one is installed for this API level.
        #[arg(long, value_enum)]
        kind: Option<Kind>,
        /// Delete without asking.
        #[arg(long)]
        yes: bool,
    },
    /// List the Android versions you can download for this computer.
    Available {
        /// Fetch Google's list again, instead of using the copy from today.
        #[arg(long)]
        refresh: bool,
        /// Include previews of upcoming Android releases.
        #[arg(long)]
        previews: bool,
    },
    /// Download and install an Android version.
    Download {
        /// The version, as its API level, such as 37, or 36.1 for a later
        /// update, or a preview's name, as available --previews lists it.
        #[arg(long)]
        api: String,
        /// The kind of Android image. Defaults to Android with Google services,
        /// which needs no account and includes Google's speech engine.
        #[arg(long, value_enum)]
        kind: Option<Kind>,
        /// Accept Google's licence for the download. Read it first: AAE saves it
        /// to a file and says where.
        #[arg(long)]
        accept_licence: bool,
    },
    /// List your devices and whether each is running.
    List,
    /// Create a device, set it up with a screen reader, and start it.
    Create {
        /// A name for the device, such as "Android 15 clean".
        name: String,
        /// The version, as its API level, such as 35 or 36.1, or a preview's
        /// name. Defaults to the newest installed.
        #[arg(long)]
        api: Option<String>,
        /// The kind of Android image.
        #[arg(long, value_enum)]
        kind: Option<Kind>,
        #[arg(long, value_enum, default_value = "phone")]
        profile: ProfileArg,
        /// The screen reader APK to install, such as a Backtalk build.
        #[arg(long)]
        screen_reader: Option<PathBuf>,
        /// If the Android image has no screen reader, download and install Backtalk.
        #[arg(long)]
        backtalk: bool,
        /// Turn off animations.
        #[arg(long)]
        no_animations: bool,
        /// Leave the screen reader's volume as Android sets it, instead of
        /// turning it up to full with AAE's helper.
        #[arg(long)]
        no_volume_boost: bool,
        /// Create the device without starting it.
        #[arg(long)]
        no_start: bool,
        /// If the Android version has to be downloaded, accept Google's licence for it.
        #[arg(long)]
        accept_licence: bool,
        /// After it starts, send this terminal's keyboard and the device's audio to it.
        #[arg(long)]
        attach: bool,
    },
    /// Copy a device, with its apps and data, under a new name.
    Clone { device: String, new_name: String },
    /// Rename a device.
    Rename { device: String, new_name: String },
    /// Reset a device to first setup. Keeps its name, hardware and volume.
    Wipe {
        device: String,
        /// Wipe without asking.
        #[arg(long)]
        yes: bool,
    },
    /// Delete a device and its files.
    Delete {
        device: String,
        /// Delete without asking.
        #[arg(long)]
        yes: bool,
    },
    /// Start a device and make sure its screen reader is on.
    Start {
        device: String,
        /// Boot from scratch instead of resuming the quick-boot state.
        #[arg(long)]
        cold: bool,
        /// On a device's first start, the screen reader APK to install.
        #[arg(long)]
        screen_reader: Option<PathBuf>,
        /// On a device's first start, if the Android image has no screen reader,
        /// download and install Backtalk.
        #[arg(long)]
        backtalk: bool,
        /// On a device's first start, leave the screen reader's volume as Android sets it.
        #[arg(long)]
        no_volume_boost: bool,
        /// Pass an extra option to the emulator, for troubleshooting. Can be repeated.
        #[arg(long = "emulator-arg", allow_hyphen_values = true, hide = true)]
        emulator_args: Vec<String>,
        /// Let the emulator play audio itself too, for troubleshooting.
        #[arg(long, hide = true)]
        emulator_audio: bool,
        /// After it starts, send this terminal's keyboard and the device's audio to it.
        #[arg(long)]
        attach: bool,
    },
    /// Restart Android on a running device, keeping everything on it.
    Restart { device: String },
    /// Stop a running device, saving its state for a quick start next time.
    Stop { device: String },
    /// Send this terminal's keys to the device and play its audio. Control-] exits.
    Attach {
        device: String,
        /// Send Option as Alt. By default it is sent as Meta, the screen reader's modifier.
        #[arg(long)]
        keep_alt: bool,
        /// Play older Android versions' audio as it comes, without raising its pitch.
        #[arg(long)]
        no_pitch_correction: bool,
    },
    /// Play the device's audio until Control-C.
    Listen {
        device: String,
        /// Play older Android versions' audio as it comes, without raising its pitch.
        #[arg(long)]
        no_pitch_correction: bool,
    },
    /// Say what a device is doing.
    Status { device: String },
    /// Set a device's playback volume on this computer, 0 to 100. No value prints it.
    PlaybackVolume { device: String, percent: Option<u8> },
    /// Set a device's audio output: a name, or "default". No value lists outputs.
    AudioOutput {
        device: String,
        /// The output's name, as listed, or "default".
        output: Option<String>,
    },
    /// Check the device's audio reaches AAE, with a test tone it doesn't play.
    SoundCheck { device: String },
    /// Measure the device's audio speed, for pitch correction.
    AudioCheck { device: String },
    /// Record what the screen reader says, and show it.
    SpeechLog {
        device: String,
        #[arg(value_enum, default_value = "show")]
        action: SpeechLogAction,
    },
    /// Check that the device can speak, and repair it if it can't.
    Speech { device: String },
    /// Show the device's log, filtered by app, tag, level or text.
    Logs {
        device: String,
        /// Only lines from this app's package name (or another process name).
        #[arg(long)]
        app: Option<String>,
        /// Only lines with this tag.
        #[arg(long)]
        tag: Option<String>,
        /// Only lines at this level or more important: verbose, debug, info,
        /// warning, error or fatal (or their first letters).
        #[arg(long)]
        level: Option<String>,
        /// Only lines whose tag or message contains this text.
        #[arg(long)]
        search: Option<String>,
        /// How many of the latest matching lines to show.
        #[arg(long, default_value_t = 100)]
        lines: usize,
        /// Keep showing new lines as they're written, until Control-C.
        #[arg(long)]
        follow: bool,
    },
    /// Print the screen's accessibility tree, as a screen reader sees it.
    Inspect {
        device: String,
        /// Print it as JSON, with every property.
        #[arg(long)]
        json: bool,
        /// Print it as a web page, with the problems found and every
        /// element's properties, for saving: aae inspect Pixel --html > screen.html
        #[arg(long, conflicts_with_all = ["json", "follow"])]
        html: bool,
        /// List only the things that can be touched, in the order gesture
        /// mode's Tab visits them, with their centres.
        #[arg(long)]
        targets: bool,
        /// Keep following the screen: print it again each time it changes,
        /// until stopped with Control-C.
        #[arg(long)]
        follow: bool,
    },
    /// Check the screen for common accessibility problems.
    Check { device: String },
    /// Check that keys reach Android as the keys you pressed, Meta included.
    Keytest { device: String },
    /// Measure the time from a key press to hearing the device respond.
    Latency {
        device: String,
        /// The keys to press, in turn, separated by commas. The default moves
        /// TalkBack to the next item and back, so it never runs off the end.
        #[arg(long, default_value = "meta+right,meta+left")]
        keys: String,
        /// How many presses. It stops after a minute whatever the number,
        /// and Control-C stops it early, keeping the results so far.
        #[arg(long, default_value_t = 5)]
        trials: usize,
    },
    /// Press keys on the device, in order.
    #[command(after_help = keys::HELP)]
    Key { device: String, keys: Vec<String> },
    /// Perform screen reader gestures in order. No gestures lists them.
    Gesture {
        device: String,
        gestures: Vec<String>,
        /// Where to perform them, as X,Y in the pixel positions the
        /// accessibility inspector reports. Defaults to the middle of the screen.
        #[arg(long, value_parser = parse_point)]
        at: Option<(i32, i32)>,
    },
    /// Type text on the device.
    Type { device: String, text: String },
    /// Install apps, and choose which of their services to turn on.
    Install {
        /// The device, or several separated by commas, such as
        /// "Android 16 test,Android 14 test".
        device: String,
        apks: Vec<PathBuf>,
        /// Leave every such part off, without asking.
        #[arg(long)]
        no_services: bool,
        /// Turn every such part on, without asking.
        #[arg(long, conflicts_with = "no_services")]
        yes: bool,
    },
    /// List the apps installed on a device, by name.
    Apps {
        device: String,
        /// Include Android's own apps.
        #[arg(long)]
        system: bool,
    },
    /// Open, stop, clear, uninstall, or change permissions of one app.
    App {
        device: String,
        /// The app's name, such as Gmail, or its package name.
        app: String,
        #[command(subcommand)]
        action: AppAction,
    },
    /// Install each new build of an APK, or the newest in a folder, until Control-C.
    Watch {
        /// The device, or several separated by commas.
        device: String,
        path: PathBuf,
        /// Install the build that's there now as well.
        #[arg(long)]
        now: bool,
    },
    /// Open a link on the device.
    Link {
        device: String,
        link: String,
        /// The app to open it in, by name or package.
        #[arg(long)]
        app: Option<String>,
    },
    /// Send an intent on the device, for testing how an app answers it.
    Intent {
        device: String,
        /// Such as android.intent.action.VIEW.
        #[arg(long)]
        action: Option<String>,
        /// A link or other address.
        #[arg(long)]
        data: Option<String>,
        /// The app, by name or package, or one of its screens or receivers
        /// as package/class.
        #[arg(long)]
        to: Option<String>,
        /// A text extra, as key=value. Can be repeated.
        #[arg(long = "extra")]
        extras: Vec<String>,
        /// Send it as a broadcast instead of to open a screen.
        #[arg(long)]
        broadcast: bool,
    },
    /// Install a screen reader build and make it the screen reader.
    ///
    /// A new build of the same screen reader keeps its settings. Stopped
    /// devices get it at next start.
    #[command(
        after_help = "Instead of an APK: backtalk downloads Backtalk's latest development build; an installed screen reader's name, such as talkback, switches to it."
    )]
    ScreenReader {
        /// The device, several separated by commas, or all, for every
        /// device that uses this screen reader.
        device: String,
        apk: String,
        /// For a build signed differently: remove the old one first, which
        /// loses the screen reader's settings.
        #[arg(long)]
        replace: bool,
    },
    /// List accessibility services, or turn one on or off.
    Services {
        device: String,
        #[command(subcommand)]
        action: Option<ServiceAction>,
    },
    /// Save, restore, list or delete named snapshots.
    Snapshot {
        device: String,
        #[command(subcommand)]
        action: SnapshotAction,
    },
    /// Rotate the device.
    Rotate {
        device: String,
        #[arg(value_enum)]
        direction: Rotation,
    },
    /// Set battery level (0 to 100), charging and health.
    Battery {
        device: String,
        level: i32,
        #[arg(long)]
        charging: bool,
        /// Its health: good, failed, dead, overvoltage or overheated.
        #[arg(long)]
        health: Option<String>,
    },
    /// Touch the fingerprint sensor with a finger, 1 to 10.
    Fingerprint {
        device: String,
        /// Which finger, from 1 to 10.
        #[arg(default_value_t = 1)]
        finger: i32,
    },
    /// Shake the device, as for apps that act on a shake.
    Shake { device: String },
    /// Speech bridge: on, off, or listen (prints utterances, no audio).
    SpeechBridge {
        device: String,
        #[arg(value_enum, default_value = "listen")]
        action: BridgeAction,
    },
    /// Export a stopped device, with its named snapshots, to one file.
    Export { device: String, file: PathBuf },
    /// Import a device exported from AAE. Its Android version must be
    /// installed here.
    Import {
        file: PathBuf,
        /// A name for it, instead of the one it had.
        #[arg(long)]
        name: Option<String>,
    },
    /// Show or change a stopped device's hardware, from its next start.
    ///
    /// Example: aae hardware Pixel memory 4096 cores 6 storage 16G screen 1440x3120 density 560
    Hardware {
        device: String,
        /// Pairs of setting and value: memory (megabytes), cores (or auto,
        /// to suit this computer), storage (such as 16G or 8192M), screen
        /// (width x height), density.
        changes: Vec<String>,
    },
    /// Play a GPX route as the device's location until it ends or Control-C.
    Route {
        device: String,
        file: PathBuf,
        /// How many times faster than the route's own pace, such as 2 or 0.5.
        #[arg(long, default_value_t = 1.0)]
        speed: f64,
    },
    /// Record the screen and audio to a WebM file, up to three minutes.
    Record {
        device: String,
        file: PathBuf,
        /// Stop after this many seconds, up to 180.
        #[arg(long)]
        seconds: Option<u32>,
    },
    /// Show or change language, display and accessibility settings.
    ///
    /// Example: aae settings Pixel font-size 150 dark-theme on language fr-FR. --list prints every setting and its values.
    Settings {
        /// The device, unless listing.
        device: Option<String>,
        /// Pairs of setting and value.
        changes: Vec<String>,
        /// List every setting and its choices.
        #[arg(long)]
        list: bool,
        /// Say, or with pairs set, what new devices start with when they're
        /// first set up, instead of a device's: aae settings --for-new-devices
        /// language en-GB font-size 130.
        #[arg(long)]
        for_new_devices: bool,
        /// New devices start with Android's own settings again.
        #[arg(long)]
        forget_new_devices: bool,
    },
    /// Show or change airplane mode, Wi-Fi, mobile data and speed.
    ///
    /// Speeds: full, lte, 3g, slow-3g, edge, gprs. Example: aae network Pixel wifi off speed edge
    Network {
        device: String,
        /// Pairs of setting and value: airplane on|off, wifi on|off,
        /// data on|off, speed <name>.
        settings: Vec<String>,
    },
    /// Microphone: on (until Control-C), check, level, or play <file>.
    ///
    /// play: WAV, MP3, FLAC, Ogg Vorbis or M4A, from the next recording.
    Mic {
        device: String,
        #[arg(value_enum)]
        action: MicAction,
        /// The audio file, for "play".
        file: Option<PathBuf>,
    },
    /// Set the device's location.
    Location {
        device: String,
        #[arg(allow_hyphen_values = true)]
        latitude: f64,
        #[arg(allow_hyphen_values = true)]
        longitude: f64,
    },
    /// Send the device a text message.
    Sms {
        device: String,
        from: String,
        text: String,
    },
    /// Simulate an incoming phone call, or end one.
    Call {
        device: String,
        number: String,
        #[arg(long)]
        end: bool,
    },
    /// Read the device's clipboard, or set it to some text.
    Clipboard {
        device: String,
        text: Option<String>,
    },
    /// Set the screen reader's volume on the device, 0 to 100.
    Volume {
        device: String,
        #[arg(value_parser = clap::value_parser!(u8).range(0..=100))]
        percent: u8,
    },
    /// Save a screenshot as a PNG file.
    Screenshot {
        device: String,
        path: Option<PathBuf>,
    },
    /// Run a shell command on the device.
    Shell {
        device: String,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, required = true)]
        command: Vec<String>,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum Kind {
    /// Plain Android, no Google account needed.
    Plain,
    /// Android with Google services.
    Google,
    /// Android with the Google Play Store.
    Play,
}

impl Kind {
    /// The image type's tag in Google's catalogue.
    fn tag(self) -> &'static str {
        match self {
            Kind::Plain => "default",
            Kind::Google => "google_apis",
            Kind::Play => "google_apis_playstore",
        }
    }

    fn matches(self, tag: &str) -> bool {
        match self {
            Kind::Plain => matches!(tag, "default" | "aosp_atd"),
            Kind::Google => matches!(tag, "google_apis" | "google_atd"),
            Kind::Play => tag.contains("playstore"),
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum ProfileArg {
    SmallPhone,
    Phone,
    Tablet,
}

impl From<ProfileArg> for Profile {
    fn from(p: ProfileArg) -> Self {
        match p {
            ProfileArg::SmallPhone => Profile::SmallPhone,
            ProfileArg::Phone => Profile::Phone,
            ProfileArg::Tablet => Profile::Tablet,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum Rotation {
    Left,
    Right,
    Portrait,
    Landscape,
}

#[derive(Subcommand)]
enum ServiceAction {
    /// Turn a service on and keep it on: by its name, package or component.
    /// A screen reader becomes the device's screen reader, instead of the
    /// one it had.
    #[command(alias = "enable")]
    On { service: String },
    /// Turn a service off and keep it off.
    #[command(alias = "disable")]
    Off { service: String },
}

#[derive(Clone, Copy, ValueEnum)]
enum SpeechLogAction {
    /// Start recording.
    On,
    /// Stop recording.
    Off,
    /// Print what was said.
    Show,
    /// Print what is said as it happens, until Control-C.
    Follow,
    /// Forget what was said.
    Clear,
}

#[derive(Subcommand)]
enum AppAction {
    /// Open it, as its icon would.
    Open,
    /// Open one of its screens by its class name, such as .SettingsActivity.
    Screen {
        name: String,
    },
    /// Force stop it.
    Stop,
    /// Delete its data, as if just installed.
    Clear,
    Uninstall,
    /// List its permissions and special access, and which it has.
    Permissions,
    /// Grant a permission, or all to grant every one it asks for.
    Grant {
        permission: String,
    },
    Revoke {
        permission: String,
    },
    /// Give special access: battery, overlay, usage or settings.
    Allow {
        access: AccessArg,
    },
    /// Take special access away.
    Disallow {
        access: AccessArg,
    },
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum AccessArg {
    /// Unrestricted battery use.
    Battery,
    /// Display over other apps.
    Overlay,
    /// Usage access.
    Usage,
    /// Modify system settings.
    Settings,
}

impl From<AccessArg> for aae_core::apps::Access {
    fn from(a: AccessArg) -> Self {
        use aae_core::apps::Access;
        match a {
            AccessArg::Battery => Access::Battery,
            AccessArg::Overlay => Access::Overlay,
            AccessArg::Usage => Access::Usage,
            AccessArg::Settings => Access::WriteSettings,
        }
    }
}

#[derive(Subcommand)]
enum SnapshotAction {
    /// List the snapshots, newest first, with when each was taken and its notes.
    List,
    /// Save the device's current state under a name.
    Save {
        name: String,
        /// Notes to keep with it, such as what it's for.
        #[arg(long, default_value = "")]
        notes: String,
    },
    /// Put the device back as it was in a snapshot.
    Load {
        name: String,
    },
    /// Change a snapshot's name, or its notes.
    Rename {
        name: String,
        new_name: String,
        #[arg(long)]
        notes: Option<String>,
    },
    Delete {
        name: String,
    },
}

#[tokio::main]
async fn main() {
    {
        use aae_core::diagnostics::{LOG_FILTER, LogFile};
        use tracing_subscriber::{EnvFilter, Layer, fmt, prelude::*};
        // To the terminal as AAE_LOG asks, and always to AAE's log file,
        // which a diagnostic report includes.
        tracing_subscriber::registry()
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
            .init();
    }
    // Only the command's name: its arguments can be text to type.
    let command = std::env::args().nth(1).unwrap_or_default();
    tracing::info!("aae {} {command}", env!("CARGO_PKG_VERSION"));
    if let Err(e) = run(Cli::parse()).await {
        eprintln!("Error: {e:#}");
        std::process::exit(1);
    }
}

struct Ctx {
    sdk: Sdk,
    store: DeviceStore,
}

impl Ctx {
    fn new() -> Result<Self> {
        Ok(Ctx {
            sdk: Sdk::locate_or_new(),
            store: DeviceStore::open_default()?,
        })
    }

    fn device(&self, name: &str) -> Result<Device> {
        Ok(self.store.get(name)?)
    }

    /// Connects to send keys, selecting AAE's full keyboard again first, in
    /// case Android dropped it.
    async fn connect_keyboard(&self, name: &str) -> Result<(Device, Controller, Adb)> {
        let (device, controller, adb) = self.connect(name).await?;
        provision::reselect_keyboard_layout_quietly(&adb).await;
        Ok((device, controller, adb))
    }

    async fn connect(&self, name: &str) -> Result<(Device, Controller, Adb)> {
        let device = self.device(name)?;
        let (_, controller, adb) = emulator::attach(&self.sdk, &device).await?;
        Ok((device, controller, adb))
    }
}

async fn run(cli: Cli) -> Result<()> {
    let ctx = Ctx::new()?;
    match cli.command {
        Command::Doctor => doctor(&ctx),
        Command::Serve {
            port,
            no_discovery,
            json,
        } => serve(port, !no_discovery, json).await,
        Command::Pair { json } => {
            let code = aae_remote::security::Pairing::default().new_code();
            let minutes = aae_remote::security::CODE_LIFETIME.as_secs() / 60;
            if json {
                println!(
                    "{}",
                    serde_json::json!({"code": code, "minutes": minutes, "serving": aae_remote::daemon::running_pid().is_some()})
                );
            } else {
                println!(
                    "Pairing code: {code}. Enter it in AAE Remote; it works once, for {minutes} minutes."
                );
                if aae_remote::daemon::running_pid().is_none() {
                    println!("Nothing is serving yet: run aae serve, or aae daemon install.");
                }
            }
            Ok(())
        }
        Command::Daemon { action, json } => {
            use aae_remote::daemon;
            match action {
                DaemonAction::Install => {
                    println!("{}", daemon::install().map_err(|e| anyhow::anyhow!(e))?)
                }
                DaemonAction::Uninstall => {
                    println!("{}", daemon::uninstall().map_err(|e| anyhow::anyhow!(e))?)
                }
                DaemonAction::Status => {
                    let status = daemon::status();
                    if json {
                        println!(
                            "{}",
                            serde_json::json!({"installed": status.installed, "running": status.running})
                        );
                    } else {
                        println!(
                            "{} {}",
                            if status.installed {
                                "AAE serves devices whenever you log in."
                            } else {
                                "AAE isn't set to serve at login."
                            },
                            if status.running {
                                "It's serving now."
                            } else {
                                "Nothing is serving now."
                            }
                        );
                    }
                }
            }
            Ok(())
        }
        Command::Phones { unpair, json } => {
            let clients = aae_remote::security::Clients::load();
            if json && unpair.is_none() {
                let list: Vec<_> = clients
                    .list()
                    .iter()
                    .map(|c| serde_json::json!({"id": c.id, "name": c.name, "paired": c.paired}))
                    .collect();
                println!("{}", serde_json::Value::from(list));
                return Ok(());
            }
            match unpair {
                Some(id) => {
                    if clients.remove(&id) {
                        println!("Unpaired {id}. It can't connect until it pairs again.");
                    } else {
                        bail!("No phone with the id {id} is paired.");
                    }
                }
                None => {
                    let list = clients.list();
                    if list.is_empty() {
                        println!("No phones are paired. Run aae serve to pair one.");
                    }
                    for client in list {
                        println!("{}: {}", client.id, client.name);
                    }
                }
            }
            Ok(())
        }
        Command::Mcp { allow_destructive } => {
            aae_mcp::serve(aae_mcp::Options { allow_destructive }).await
        }
        Command::SelfTest => {
            use aae_core::diagnostics::Outcome;
            let checks = aae_core::diagnostics::self_test(&ctx.sdk, &ctx.store).await;
            for check in &checks {
                let label = match check.outcome {
                    Outcome::Passed => "OK",
                    Outcome::Warning => "Warning",
                    Outcome::Failed => "Problem",
                };
                println!("{label}: {}. {}", check.name, check.detail);
            }
            let failed = checks
                .iter()
                .filter(|c| c.outcome == Outcome::Failed)
                .count();
            let warned = checks
                .iter()
                .filter(|c| c.outcome == Outcome::Warning)
                .count();
            println!(
                "{} checks: {} passed, {warned} warnings, {failed} problems.",
                checks.len(),
                checks.len() - failed - warned
            );
            if failed > 0 {
                bail!("The self-test found {failed} problems.");
            }
            Ok(())
        }
        Command::PlaybackVolume { device, percent } => {
            let mut device = ctx.device(&device)?;
            match percent {
                Some(percent) => {
                    let volume = f32::from(percent.min(100)) / 100.0;
                    device.meta.playback_volume = (volume < 1.0).then_some(volume);
                    device.save_meta()?;
                    println!(
                        "{}: playback volume {}%.",
                        device.meta.name,
                        percent.min(100)
                    );
                }
                None => println!(
                    "{} plays at {}%.",
                    device.meta.name,
                    (device.meta.playback_volume.unwrap_or(1.0) * 100.0).round()
                ),
            }
            Ok(())
        }
        Command::AudioOutput { device, output } => {
            let mut device = ctx.device(&device)?;
            let outputs = aae_core::audio::output_names();
            match output {
                Some(name) => {
                    let chosen = if name.eq_ignore_ascii_case("default") {
                        None
                    } else {
                        Some(
                            outputs
                                .iter()
                                .find(|o| o.eq_ignore_ascii_case(&name))
                                .or_else(|| {
                                    outputs
                                        .iter()
                                        .find(|o| o.to_lowercase().contains(&name.to_lowercase()))
                                })
                                .cloned()
                                .ok_or_else(|| {
                                    anyhow!(
                                        "There's no output called {name}. The outputs are: {}.",
                                        outputs.join(", ")
                                    )
                                })?,
                        )
                    };
                    device.meta.audio_output = chosen.clone();
                    device.save_meta()?;
                    println!(
                        "{}: audio output {}.",
                        device.meta.name,
                        chosen.as_deref().unwrap_or("system default")
                    );
                }
                None => {
                    println!(
                        "{}: audio output {}.",
                        device.meta.name,
                        device
                            .meta
                            .audio_output
                            .as_deref()
                            .unwrap_or("system default")
                    );
                    println!("Outputs: {}.", outputs.join(", "));
                }
            }
            Ok(())
        }
        Command::SoundCheck { device } => {
            let (device, controller, adb) = ctx.connect(&device).await?;
            let mut player = aae_core::audio::AudioPlayer::start(&controller).await?;
            let health = player.probe(&adb).await;
            player.stop();
            match health {
                aae_core::audio::AudioHealth::Broken(why) => bail!("{why}"),
                _ => println!("{}'s sound is reaching AAE.", device.meta.name),
            }
            Ok(())
        }
        Command::Report { out } => {
            let version = format!("{} (aae command)", env!("CARGO_PKG_VERSION"));
            let report = aae_core::diagnostics::report(&ctx.sdk, &ctx.store, &version);
            let path = out.unwrap_or_else(|| {
                let stamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs());
                PathBuf::from(format!("aae-report-{stamp}.txt"))
            });
            std::fs::write(&path, report)?;
            println!("Saved the report to {}, as plain text.", path.display());
            Ok(())
        }
        Command::Setup {
            accept_licence,
            update,
            refresh,
        } => setup(&ctx, accept_licence, update, refresh).await,
        Command::Available { refresh, previews } => {
            let catalogue = tokio::task::spawn_blocking(move || Catalogue::load(refresh)).await??;
            if catalogue.images.is_empty() {
                println!("Google offers no Android versions for this computer's processor.");
            }
            for image in catalogue
                .images
                .iter()
                .filter(|i| previews || i.release.preview.is_none())
            {
                let state = if image.is_installed(&ctx.sdk) {
                    "installed".to_string()
                } else {
                    format!(
                        "{} to download with --api {}",
                        human_size(image.size),
                        image.release.id
                    )
                };
                println!("{}, {state}.", image.describe());
            }
            if !previews {
                println!("Add --previews to include previews of upcoming Android releases.");
            }
            Ok(())
        }
        Command::Download {
            api,
            kind,
            accept_licence,
        } => {
            let image = download_image(&ctx, &api, kind, accept_licence).await?;
            println!("{image} is installed.");
            Ok(())
        }
        Command::Images => {
            let images = ctx.sdk.system_images();
            if images.is_empty() {
                println!("No Android versions are installed yet.");
            }
            for image in images {
                let users = catalog::image_users(&ctx.sdk, &ctx.store, &image)?;
                let size = human_size(catalog::image_size(&image));
                println!("{image}. {size}. {}", describe_users(&users));
            }
            Ok(())
        }
        Command::RemoveImage { api, kind, yes } => {
            let matching: Vec<_> = ctx
                .sdk
                .system_images()
                .into_iter()
                .filter(|i| i.release.matches(&api) && kind.is_none_or(|k| k.matches(&i.tag)))
                .collect();
            let image = match matching.as_slice() {
                [] => anyhow::bail!(
                    "No Android version {api} is installed. Use the images command to see what is."
                ),
                [image] => image.clone(),
                several => anyhow::bail!(
                    "More than one kind of Android version {api} is installed: {}. Choose one with --kind.",
                    several
                        .iter()
                        .map(|i| i.kind())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            };
            let users = catalog::image_users(&ctx.sdk, &ctx.store, &image)?;
            if !users.devices.is_empty() {
                anyhow::bail!(
                    "{image} can't be deleted while devices use it: {}. Delete those devices first.",
                    users.devices.join(", ")
                );
            }
            let size = human_size(catalog::image_size(&image));
            let mut question = format!("Delete {image} and its {size} of files?");
            if !users.others.is_empty() {
                question.push_str(&format!(
                    " Other emulator devices, such as Android Studio's, use it and won't start without it: {}.",
                    users.others.join(", ")
                ));
            }
            question.push_str(" You can download it again later.");
            if !yes && !confirm(&question)? {
                println!("Nothing was deleted.");
                return Ok(());
            }
            let freed = catalog::remove(&ctx.sdk, &ctx.store, &image)?;
            println!("Deleted {image}. Freed {}.", human_size(freed));
            Ok(())
        }
        Command::List => {
            let devices = ctx.store.list()?;
            if devices.is_empty() {
                println!("You have no devices yet. Create one with: aae create \"My phone\"");
            }
            for device in devices {
                let state = if emulator::running(&device).is_ok() {
                    "running"
                } else {
                    "stopped"
                };
                println!("{}, {state}.", device.describe());
            }
            Ok(())
        }
        Command::Create {
            name,
            api,
            kind,
            profile,
            screen_reader,
            backtalk,
            no_animations,
            no_volume_boost,
            no_start,
            accept_licence,
            attach,
        } => {
            let image = match (pick_image(&ctx.sdk, api.as_deref(), kind), api.as_deref()) {
                (Ok(image), _) => image,
                // Not installed: download it.
                (Err(_), Some(api)) => download_image(&ctx, api, kind, accept_licence).await?,
                (Err(e), None) => return Err(e),
            };
            let mut device = ctx.store.create(&name, &image, profile.into())?;
            println!("Created {}.", device.describe());
            if no_start {
                return Ok(());
            }
            let options = ProvisionOptions {
                screen_reader_apk: screen_reader,
                backtalk,
                disable_animations: no_animations,
                full_volume: !no_volume_boost,
                ..Default::default()
            };
            start(&ctx, &mut device, &StartOptions::default(), &options).await?;
            if attach {
                attach::run(&ctx.sdk, &device, false, true).await?;
            }
            Ok(())
        }
        Command::Clone { device, new_name } => {
            let source = ctx.device(&device)?;
            println!("Copying {}.", source.meta.name);
            let clone = ctx.store.clone_device(&source, &new_name)?;
            println!(
                "Created {}, a copy of {}.",
                clone.meta.name, source.meta.name
            );
            Ok(())
        }
        Command::Rename { device, new_name } => {
            let mut device = ctx.device(&device)?;
            let old = device.meta.name.clone();
            ctx.store.rename(&mut device, &new_name)?;
            println!("Renamed {old} to {}.", device.meta.name);
            Ok(())
        }
        Command::Wipe { device, yes } => {
            let mut device = ctx.device(&device)?;
            let name = device.meta.name.clone();
            if !yes
                && !confirm(&format!(
                    "Wipe {name}? Its apps, data and snapshots are deleted, and it's set up again \
                     with its screen reader. This can't be undone."
                ))?
            {
                println!("Nothing was wiped.");
                return Ok(());
            }
            println!("Wiping {name}.");
            lifecycle::wipe_device(&ctx.sdk, &ctx.store, &mut device, |p| {
                println!("{}", p.describe(&name))
            })
            .await?;
            Ok(())
        }
        Command::Delete { device, yes } => {
            let device = ctx.device(&device)?;
            let size = human_size(device.disk_usage());
            if !yes
                && !confirm(&format!(
                    "Delete {} and its {size} of files? This can't be undone.",
                    device.meta.name
                ))?
            {
                println!("Nothing was deleted.");
                return Ok(());
            }
            let freed = ctx.store.delete(&device)?;
            println!("Deleted {}. Freed {}.", device.meta.name, human_size(freed));
            Ok(())
        }
        Command::Start {
            device,
            cold,
            screen_reader,
            backtalk,
            no_volume_boost,
            emulator_args,
            emulator_audio,
            attach,
        } => {
            let mut device = ctx.device(&device)?;
            let options = ProvisionOptions {
                screen_reader_apk: screen_reader,
                backtalk,
                full_volume: !no_volume_boost,
                ..Default::default()
            };
            let start_options = StartOptions {
                cold_boot: cold,
                extra_args: emulator_args,
                emulator_audio,
                ..Default::default()
            };
            start(&ctx, &mut device, &start_options, &options).await?;
            if attach {
                attach::run(&ctx.sdk, &device, false, true).await?;
            }
            Ok(())
        }
        Command::Inspect {
            device,
            json,
            html,
            targets,
            follow,
        } => {
            let (device, _, adb) = ctx.connect(&device).await?;
            if follow {
                return follow_screen(&ctx, &device, &adb, json, targets).await;
            }
            let tree =
                with_helper(&ctx, &device, &adb, aae_core::inspector::read_tree(&adb)).await?;
            if targets {
                for target in aae_core::inspector::targets(&tree) {
                    let (x, y) = target.centre();
                    println!("{} (at {x}, {y})", target.label);
                }
            } else if html {
                let title = format!("{}: the screen's accessibility", device.meta.name);
                print!("{}", aae_core::inspector::to_html(&tree, &title));
            } else if json {
                println!("{}", serde_json::to_string_pretty(&tree)?);
            } else {
                print!("{}", aae_core::inspector::to_text(&tree));
            }
            Ok(())
        }
        Command::Check { device } => {
            let (device, _, adb) = ctx.connect(&device).await?;
            let tree =
                with_helper(&ctx, &device, &adb, aae_core::inspector::read_tree(&adb)).await?;
            let issues = aae_core::inspector::check(&tree);
            if issues.is_empty() {
                println!("No problems found on this screen.");
            }
            for issue in &issues {
                let severity = match issue.severity {
                    aae_core::inspector::Severity::Error => "Error",
                    aae_core::inspector::Severity::Warning => "Warning",
                };
                let id = issue
                    .id
                    .as_deref()
                    .map(|id| format!(" ({id})"))
                    .unwrap_or_default();
                println!(
                    "{severity}: {} Element: {}{id}.",
                    issue.message, issue.element
                );
            }
            Ok(())
        }
        Command::AudioCheck { device } => {
            let (mut device, _, adb) = ctx.connect(&device).await?;
            provision::update_helper(&ctx.sdk, &adb).await?;
            device.meta.audio_speed = None;
            provision::measure_audio(&mut device, &adb).await;
            match device.meta.audio_speed {
                Some(speed) if (speed - 1.0).abs() < 0.01 => {
                    println!("{} plays audio at the right speed.", device.meta.name)
                }
                Some(speed) => println!(
                    "{} plays audio at {:.1}% speed, which lowers its pitch. AAE corrects it when playing.",
                    device.meta.name,
                    speed * 100.0
                ),
                None => bail!(
                    "The audio couldn't be measured. Try again with AAE_LOG=info for details."
                ),
            }
            Ok(())
        }
        Command::Logs {
            device,
            app,
            tag,
            level,
            search,
            lines,
            follow,
        } => {
            use aae_core::logcat;
            let level = match level {
                Some(name) => Some(logcat::Level::parse(&name).ok_or_else(|| {
                    anyhow::anyhow!(
                        "\"{name}\" is not a log level. Use verbose, debug, info, warning, error or fatal."
                    )
                })?),
                None => None,
            };
            let filter = logcat::Filter {
                process: app,
                tag,
                level,
                text: search,
            };
            let (_, _, adb) = ctx.connect(&device).await?;
            let print = |e: &logcat::Entry| {
                println!(
                    "{} {} {} {}: {}",
                    e.clock(),
                    e.level.letter(),
                    e.process.as_deref().unwrap_or("?"),
                    e.tag,
                    e.message
                )
            };
            if !follow {
                let found: Vec<_> = logcat::dump(&adb)
                    .await?
                    .into_iter()
                    .filter(|e| filter.matches(e))
                    .collect();
                found[found.len().saturating_sub(lines)..]
                    .iter()
                    .for_each(print);
                return Ok(());
            }
            let stream = logcat::LogStream::start(adb, lines.max(1000));
            let everything = logcat::Filter::default();
            let (mut since, mut settled, mut seen) = (0, false, 0);
            loop {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => return Ok(()),
                    _ = tokio::time::sleep(Duration::from_millis(300)) => {}
                }
                let new = stream.entries(since, &everything, usize::MAX);
                if !settled {
                    // The earlier lines arrive in a burst; once it's over, show
                    // the latest few of them, then everything new.
                    if new.is_empty() || new.len() - seen > 20 {
                        seen = new.len();
                        continue;
                    }
                    settled = true;
                    let found: Vec<_> = new.iter().filter(|e| filter.matches(e)).collect();
                    found[found.len().saturating_sub(lines)..]
                        .iter()
                        .for_each(|e| print(e));
                } else {
                    new.iter().filter(|e| filter.matches(e)).for_each(print);
                }
                if let Some(last) = new.last() {
                    since = last.seq;
                }
            }
        }
        Command::SpeechLog { device, action } => {
            use aae_core::tts;
            let (mut device, _, adb) = ctx.connect(&device).await?;
            match action {
                SpeechLogAction::On | SpeechLogAction::Off => {
                    let on = matches!(action, SpeechLogAction::On);
                    if device.meta.speech_log_on() == on {
                        println!(
                            "The speech log is already {} for {}.",
                            if on { "on" } else { "off" },
                            device.meta.name
                        );
                        return Ok(());
                    }
                    provision::update_helper(&ctx.sdk, &adb).await?;
                    let bridge = device.meta.speech_bridge;
                    tts::set_relay(&adb, &mut device, on, bridge).await?;
                    println!("The speech log is {}.", if on { "on" } else { "off" });
                }
                SpeechLogAction::Show => {
                    let log = tts::speech_log(&adb, 0, false).await?;
                    if log.is_empty() {
                        println!(
                            "Nothing has been said{}.",
                            if !device.meta.speech_log_on() {
                                ", and the speech log is off. Turn it on with: aae speech-log <device> on"
                            } else {
                                " since the log was cleared"
                            }
                        );
                    }
                    for u in log {
                        println!("{}  {}", tts::clock_time(u.time), u.text);
                    }
                }
                SpeechLogAction::Follow => {
                    println!(
                        "Showing what {} says. Press Control-C to stop.",
                        device.meta.name
                    );
                    let mut since = tts::speech_log(&adb, 0, false)
                        .await?
                        .last()
                        .map_or(0, |u| u.time);
                    loop {
                        tokio::select! {
                            _ = tokio::signal::ctrl_c() => return Ok(()),
                            _ = tokio::time::sleep(Duration::from_millis(400)) => {}
                        }
                        for u in tts::speech_log(&adb, since, false).await? {
                            println!("{}  {}", tts::clock_time(u.time), u.text);
                            since = u.time;
                        }
                    }
                }
                SpeechLogAction::Clear => {
                    tts::speech_log(&adb, u64::MAX, true).await?;
                    println!("Cleared the speech log.");
                }
            }
            Ok(())
        }
        Command::Speech { device } => {
            let (device, _, adb) = ctx.connect(&device).await?;
            provision::update_helper(&ctx.sdk, &adb).await?;
            let status = aae_core::tts::check(&adb).await?;
            if status.ok {
                println!(
                    "{}: text-to-speech works, with {}.",
                    device.meta.name, status.engine
                );
                return Ok(());
            }
            println!(
                "{}: text-to-speech failed: {}. Repairing it.",
                device.meta.name, status.detail
            );
            let fix = provision::ensure_device_speech(&device, &adb).await?;
            println!("{}", fix.describe().unwrap_or("Speech works now."));
            Ok(())
        }
        Command::Restart { device } => {
            let mut device = ctx.device(&device)?;
            println!("Restarting Android on {}.", device.meta.name);
            emulator::reboot(&ctx.sdk, &device, lifecycle::BOOT_TIMEOUT).await?;
            start(
                &ctx,
                &mut device,
                &StartOptions::default(),
                &ProvisionOptions::default(),
            )
            .await
        }
        Command::Stop { device } => {
            let device = ctx.device(&device)?;
            println!("Stopping {}.", device.meta.name);
            emulator::stop(&ctx.sdk, &device, Duration::from_secs(60)).await?;
            println!("{} is stopped.", device.meta.name);
            Ok(())
        }
        Command::Attach {
            device,
            keep_alt,
            no_pitch_correction,
        } => {
            attach::run(
                &ctx.sdk,
                &ctx.device(&device)?,
                keep_alt,
                !no_pitch_correction,
            )
            .await
        }
        Command::Listen {
            device,
            no_pitch_correction,
        } => attach::listen(&ctx.sdk, &ctx.device(&device)?, !no_pitch_correction).await,
        Command::Status { device } => status(&ctx, &device).await,
        Command::Keytest { device } => keytest(&ctx, &device).await,
        Command::Latency {
            device,
            keys,
            trials,
        } => latency(&ctx, &device, &keys, trials).await,
        Command::Key {
            device,
            keys: names,
        } => {
            let (_, controller, adb) = ctx.connect_keyboard(&device).await?;
            for name in &names {
                if let Some(code) = keys::android_only(name) {
                    adb.shell(&format!("input keyevent {code}")).await?;
                    continue;
                }
                let key = keys::parse(name)
                    .ok_or_else(|| anyhow!("\"{name}\" is not a key name. {}", keys::HELP))?;
                controller.press(&key).await?;
            }
            Ok(())
        }
        Command::Gesture {
            device,
            gestures,
            at,
        } => {
            use aae_core::gestures::{self, Gesture, Screen};
            if gestures.is_empty() {
                println!("Gestures, named with hyphens or spaces:");
                for gesture in Gesture::all() {
                    println!("  {}", gesture.to_string().replace(' ', "-"));
                }
                return Ok(());
            }
            let parsed = gestures
                .iter()
                .map(|name| {
                    Gesture::parse(name).ok_or_else(|| {
                        anyhow!(
                            "\"{name}\" is not a gesture. Run aae gesture with only a device to list them."
                        )
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            let (_, controller, adb) = ctx.connect(&device).await?;
            let screen = Screen::read(&adb).await?;
            for (i, gesture) in parsed.iter().enumerate() {
                if i > 0 {
                    // Long enough that two gestures aren't read as one.
                    tokio::time::sleep(Duration::from_millis(700)).await;
                }
                gestures::perform(&controller, &screen, gesture, at).await?;
            }
            Ok(())
        }
        Command::Type { device, text } => {
            let (_, controller, _) = ctx.connect_keyboard(&device).await?;
            controller.type_text(&text).await?;
            Ok(())
        }
        Command::Install {
            device,
            apks,
            no_services,
            yes,
        } => {
            let names: Vec<&str> = device
                .split(',')
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .collect();
            // Answers about an app's parts, asked once and used on every device.
            let mut answers: HashMap<String, bool> = HashMap::new();
            for name in names {
                let (mut device, _, adb) = ctx.connect(name).await?;
                for apk in &apks {
                    println!("Installing {} on {}.", apk.display(), device.meta.name);
                    let (info, parts) =
                        provision::install_app(&ctx.sdk, &mut device, &adb, apk).await?;
                    println!("Installed {}.", info.package);
                    let mut choices = Vec::new();
                    for part in parts {
                        match part.choice {
                            Some(on) => println!(
                                "{} is {}, as you chose before.",
                                part.name,
                                if on { "on" } else { "off" }
                            ),
                            None => {
                                let on = if no_services {
                                    false
                                } else if yes {
                                    true
                                } else if let Some(&on) = answers.get(&part.component) {
                                    on
                                } else {
                                    ask_part(&part, &info.package)?
                                };
                                answers.insert(part.component.clone(), on);
                                choices.push((part, on));
                            }
                        }
                    }
                    for (part, on) in &choices {
                        let state = if *on { "Turning on" } else { "Leaving off" };
                        println!("{state} {}, {}.", part.name, part.kind.describe());
                    }
                    provision::apply_choices(&mut device, &adb, &choices).await?;
                }
            }
            Ok(())
        }
        Command::Watch { device, path, now } => {
            use aae_core::watch;
            let names: Vec<String> = device
                .split(',')
                .map(|n| n.trim().to_string())
                .filter(|n| !n.is_empty())
                .collect();
            let mut last = watch::current_build(&path);
            if last.is_none() && !path.exists() {
                bail!("{} doesn't exist.", path.display());
            }
            println!(
                "Watching {} for new builds. Press Control-C to stop.",
                path.display()
            );
            let mut pending = if now { last.clone() } else { None };
            loop {
                let build = match pending.take() {
                    Some(build) => build,
                    None => tokio::select! {
                        _ = tokio::signal::ctrl_c() => return Ok(()),
                        build = watch::next_build(&path, last.as_ref()) => build,
                    },
                };
                last = Some(build.clone());
                println!("New build: {}.", build.path.display());
                for name in &names {
                    let result: Result<()> = async {
                        let (mut device, _, adb) = ctx.connect(name).await?;
                        let (info, parts) =
                            provision::install_app(&ctx.sdk, &mut device, &adb, &build.path)
                                .await?;
                        let mut choices = Vec::new();
                        for part in parts.into_iter().filter(|p| p.choice.is_none()) {
                            let on = ask_part(&part, &info.package)?;
                            choices.push((part, on));
                        }
                        provision::apply_choices(&mut device, &adb, &choices).await?;
                        println!("Installed {} on {}.", info.package, device.meta.name);
                        Ok(())
                    }
                    .await;
                    if let Err(e) = result {
                        println!("{name}: {e:#}");
                    }
                }
            }
        }
        Command::Link { device, link, app } => {
            let (_, _, adb) = ctx.connect(&device).await?;
            let app = match app {
                Some(name) => Some(app_package(&ctx, &adb, &name).await?),
                None => None,
            };
            aae_core::apps::open_link(&adb, &link, app.as_deref()).await?;
            println!("Opened {link}.");
            Ok(())
        }
        Command::Intent {
            device,
            action,
            data,
            to,
            extras,
            broadcast,
        } => {
            let (_, _, adb) = ctx.connect(&device).await?;
            let target = match to {
                Some(t) if t.contains('/') => Some(t),
                Some(name) => Some(app_package(&ctx, &adb, &name).await?),
                None => None,
            };
            let extras = extras
                .iter()
                .map(|e| {
                    e.split_once('=')
                        .map(|(k, v)| (k.to_string(), v.to_string()))
                        .ok_or_else(|| anyhow!("\"{e}\" isn't key=value."))
                })
                .collect::<Result<Vec<_>>>()?;
            let said = aae_core::apps::send(
                &adb,
                &aae_core::apps::Intent {
                    action,
                    data,
                    target,
                    extras,
                    broadcast,
                },
            )
            .await?;
            println!("{said}");
            Ok(())
        }
        Command::Apps { device, system } => {
            let (_, _, adb) = ctx.connect(&device).await?;
            provision::update_helper(&ctx.sdk, &adb).await?;
            let apps = aae_core::apps::list(&adb, system).await?;
            if apps.is_empty() {
                println!("No apps are installed.");
            }
            for app in apps {
                let mut line = format!("{} ({}", app.label, app.package);
                if !app.version.is_empty() {
                    line.push_str(&format!(", version {}", app.version));
                }
                if !app.enabled {
                    line.push_str(", turned off");
                }
                println!("{line}).");
            }
            Ok(())
        }
        Command::App {
            device,
            app,
            action,
        } => {
            use aae_core::apps;
            let (_, _, adb) = ctx.connect(&device).await?;
            provision::update_helper(&ctx.sdk, &adb).await?;
            let all = apps::list(&adb, true).await?;
            let found = all
                .iter()
                .find(|a| a.package == app)
                .or_else(|| {
                    all.iter()
                        .find(|a| a.label.eq_ignore_ascii_case(app.trim()))
                })
                .cloned()
                .ok_or_else(|| {
                    anyhow!("No app called \"{app}\" is installed. aae apps lists them.")
                })?;
            let (name, package) = (found.label.clone(), found.package.clone());
            match action {
                AppAction::Open => {
                    apps::open(&adb, &package).await?;
                    println!("Opened {name}.");
                }
                AppAction::Screen { name: screen } => {
                    let component = if screen.contains('/') {
                        screen
                    } else {
                        format!("{package}/{screen}")
                    };
                    apps::open_activity(&adb, &component).await?;
                    println!("Opened {component}.");
                }
                AppAction::Stop => {
                    apps::force_stop(&adb, &package).await?;
                    println!("Stopped {name}.");
                }
                AppAction::Clear => {
                    apps::clear_data(&adb, &package).await?;
                    println!("Cleared {name}'s data.");
                }
                AppAction::Uninstall => {
                    adb.uninstall(&package).await?;
                    println!("Uninstalled {name}.");
                }
                AppAction::Permissions => {
                    let permissions = apps::permissions(&adb, &package).await?;
                    if permissions.is_empty() {
                        println!("{name} asks for no permissions.");
                    }
                    for p in permissions {
                        let state = if p.granted { "granted" } else { "not granted" };
                        println!("{}: {state} ({}).", p.label, p.name);
                    }
                    for access in apps::Access::ALL {
                        let on = apps::has_access(&adb, &package, access).await?;
                        println!(
                            "{}: {}.",
                            access.describe(),
                            if on { "allowed" } else { "not allowed" }
                        );
                    }
                }
                AppAction::Grant { permission } if permission.eq_ignore_ascii_case("all") => {
                    let n = apps::grant_all(&adb, &package).await?;
                    println!("Granted {n} permissions to {name}.");
                }
                AppAction::Grant { permission } => {
                    apps::set_permission(&adb, &package, &permission, true).await?;
                    println!("Granted {permission} to {name}.");
                }
                AppAction::Revoke { permission } => {
                    apps::set_permission(&adb, &package, &permission, false).await?;
                    println!("Revoked {permission} from {name}.");
                }
                AppAction::Allow { access } => {
                    let access = apps::Access::from(access);
                    apps::set_access(&adb, &package, access, true).await?;
                    println!("{name}: {} allowed.", access.describe());
                }
                AppAction::Disallow { access } => {
                    let access = apps::Access::from(access);
                    apps::set_access(&adb, &package, access, false).await?;
                    println!("{name}: {} not allowed.", access.describe());
                }
            }
            Ok(())
        }
        Command::ScreenReader {
            device,
            apk,
            replace,
        } => {
            // A screen reader already on the device, such as talkback, by name.
            if !apk.eq_ignore_ascii_case("backtalk") && !std::path::Path::new(&apk).exists() {
                use aae_core::services;
                let wanted = if apk.eq_ignore_ascii_case("talkback") {
                    "com.google.android.marvin.talkback".to_string()
                } else {
                    apk.clone()
                };
                for name in device.split(',').map(str::trim).filter(|n| !n.is_empty()) {
                    let (mut device, _, adb) = ctx.connect(name).await?;
                    provision::update_helper(&ctx.sdk, &adb).await?;
                    let list = services::list(&adb, &device).await?;
                    let service = services::find(&list, &wanted)
                        .filter(|s| s.screen_reader)
                        .cloned()
                        .ok_or_else(|| {
                            anyhow!(
                                "{} has no screen reader called \"{apk}\", and there's no file by that name. aae services lists what's installed.",
                                device.meta.name
                            )
                        })?;
                    services::use_screen_reader(&mut device, &adb, &service).await?;
                    println!(
                        "{} is now the screen reader on {}.",
                        service.label, device.meta.name
                    );
                }
                return Ok(());
            }
            let backtalk = apk.eq_ignore_ascii_case("backtalk");
            let mut path = (!backtalk).then(|| PathBuf::from(&apk));
            let devices: Vec<Device> = if device.trim().eq_ignore_ascii_case("all") {
                let package = match &path {
                    Some(path) => provision::read_apk(&ctx.sdk, path)?.package,
                    None => aae_core::screenreader::BACKTALK_PACKAGE.to_string(),
                };
                let users: Vec<Device> = ctx
                    .store
                    .list()?
                    .into_iter()
                    .filter(|d| {
                        d.meta
                            .screen_reader
                            .as_deref()
                            .is_some_and(|c| c.split('/').next() == Some(package.as_str()))
                    })
                    .collect();
                if users.is_empty() {
                    bail!("No device uses {package} as its screen reader.");
                }
                users
            } else {
                device
                    .split(',')
                    .map(str::trim)
                    .filter(|n| !n.is_empty())
                    .map(|n| ctx.device(n))
                    .collect::<Result<_>>()?
            };
            for mut device in devices {
                let name = device.meta.name.clone();
                let apk_path = match &path {
                    Some(path) => path.clone(),
                    None => {
                        println!("Getting Backtalk's latest development build.");
                        let downloaded = provision::download_backtalk(device.meta.api).await?;
                        path = Some(downloaded.clone());
                        downloaded
                    }
                };
                if emulator::running(&device).is_ok() {
                    let (_, _, adb) = emulator::attach(&ctx.sdk, &device).await?;
                    let package = provision::add_screen_reader(
                        &ctx.sdk,
                        &mut device,
                        &adb,
                        &apk_path,
                        replace,
                    )
                    .await?;
                    println!("{package} is installed and on, on {name}.");
                } else {
                    let package = provision::queue_screen_reader(&ctx.sdk, &mut device, &apk_path)?;
                    println!(
                        "{name} is stopped, so {package} will be installed when it next starts."
                    );
                }
            }
            Ok(())
        }
        Command::Services { device, action } => {
            use aae_core::services;
            let (mut device, _, adb) = ctx.connect(&device).await?;
            provision::update_helper(&ctx.sdk, &adb).await?;
            let list = services::list(&adb, &device).await?;
            let pick = |name: &str| {
                services::find(&list, name).cloned().ok_or_else(|| {
                    anyhow!("No accessibility service called \"{name}\" is installed. aae services lists them.")
                })
            };
            match action {
                None => {
                    for s in &list {
                        let mut line = format!("{}: {}", s.label, if s.on { "on" } else { "off" });
                        if s.current_screen_reader {
                            line.push_str(", the device's screen reader");
                        } else if s.screen_reader {
                            line.push_str(", a screen reader");
                        }
                        println!("{line} ({}).", s.component);
                        if !s.description.is_empty() {
                            println!("  {}", s.description.lines().next().unwrap_or(""));
                        }
                    }
                }
                Some(ServiceAction::On { service }) => {
                    let service = pick(&service)?;
                    services::set(&mut device, &adb, &service, true).await?;
                    if service.screen_reader {
                        println!("{} is now the screen reader.", service.label);
                    } else {
                        println!("Turned on {}, and AAE keeps it on.", service.label);
                    }
                }
                Some(ServiceAction::Off { service }) => {
                    let service = pick(&service)?;
                    services::set(&mut device, &adb, &service, false).await?;
                    println!("Turned off {}.", service.label);
                }
            }
            Ok(())
        }
        Command::Snapshot { device, action } => {
            let (device, controller, _) = ctx.connect(&device).await?;
            let find = |snapshots: &[aae_core::control::Snapshot], name: &str| {
                snapshots
                    .iter()
                    .find(|s| s.id == name || s.name.eq_ignore_ascii_case(name.trim()))
                    .cloned()
                    .ok_or_else(|| {
                        anyhow!(
                            "{} has no snapshot called \"{name}\". Use aae snapshot \"{}\" list to see them.",
                            device.meta.name, device.meta.name
                        )
                    })
            };
            match action {
                SnapshotAction::List => {
                    let snapshots = controller.snapshots().await?;
                    if snapshots.is_empty() {
                        println!("{} has no snapshots.", device.meta.name);
                    }
                    for s in snapshots {
                        let mut line = s.name.clone();
                        if let Some(taken) = s.taken() {
                            line.push_str(&format!(", taken {taken}"));
                        }
                        line.push_str(&format!(", {}", human_size(s.size)));
                        if s.loaded {
                            line.push_str(", restored last");
                        }
                        if !s.compatible {
                            line.push_str(", can't be restored by this emulator");
                        }
                        println!("{line}.");
                        if !s.notes.is_empty() {
                            println!("  {}", s.notes);
                        }
                    }
                }
                SnapshotAction::Save { name, notes } => {
                    println!("Saving snapshot {name}.");
                    controller.save_named_snapshot(&name, &notes).await?;
                    println!("Saved snapshot {name}.");
                }
                SnapshotAction::Load { name } => {
                    let snapshot = find(&controller.snapshots().await?, &name)?;
                    controller.load_snapshot(&snapshot.id).await?;
                    println!("Restored snapshot {}.", snapshot.name);
                }
                SnapshotAction::Rename {
                    name,
                    new_name,
                    notes,
                } => {
                    let snapshot = find(&controller.snapshots().await?, &name)?;
                    let notes = notes.unwrap_or(snapshot.notes);
                    controller
                        .update_snapshot(&snapshot.id, &new_name, &notes)
                        .await?;
                    println!("Renamed snapshot {} to {new_name}.", snapshot.name);
                }
                SnapshotAction::Delete { name } => {
                    let snapshot = find(&controller.snapshots().await?, &name)?;
                    controller.delete_snapshot(&snapshot.id).await?;
                    println!("Deleted snapshot {}.", snapshot.name);
                }
            }
            Ok(())
        }
        Command::Rotate { device, direction } => {
            let (_, controller, _) = ctx.connect(&device).await?;
            let current = controller.orientation().await?;
            let next = match direction {
                Rotation::Left => current.turned_left(),
                Rotation::Right => current.turned_right(),
                Rotation::Portrait => Orientation::Portrait,
                Rotation::Landscape => Orientation::LandscapeLeft,
            };
            controller.set_orientation(next).await?;
            println!("{}.", next.describe());
            Ok(())
        }
        Command::Mic {
            device,
            action,
            file,
        } => {
            use aae_core::microphone;
            let (_, controller, adb) = ctx.connect(&device).await?;
            provision::update_helper(&ctx.sdk, &adb).await?;
            match action {
                MicAction::Check => {
                    let heard = microphone::check(&controller, &adb).await?;
                    println!("{}", microphone::describe_check(heard)?);
                }
                MicAction::Level => {
                    let heard = microphone::level(&adb, 1500).await?;
                    println!("Microphone peak over 1.5 seconds: {heard} of 32767.");
                }
                MicAction::Play => {
                    let Some(file) = file else {
                        bail!("Give an audio file: aae mic <device> play <file>");
                    };
                    let sound = microphone::decode(&file)?;
                    println!(
                        "{} seconds of audio. It plays when an app on the device records. Control-C cancels.",
                        (sound.seconds() * 10.0).round() / 10.0
                    );
                    tokio::select! {
                        played = microphone::play(&controller, &adb, &sound) => match played? {
                            microphone::Played::Whole => println!("Played the whole file into the microphone."),
                            microphone::Played::Stopped(at) => println!(
                                "The app stopped recording {:.1} seconds in.",
                                at
                            ),
                        },
                        _ = tokio::signal::ctrl_c() => println!("Cancelled."),
                    }
                }
                MicAction::On => {
                    let mic = microphone::Microphone::start(&controller, &adb).await?;
                    println!("Microphone on: {}. Control-C stops.", mic.name);
                    tokio::signal::ctrl_c().await?;
                    mic.stop();
                    println!("Microphone off.");
                }
            }
            Ok(())
        }
        Command::Network { device, settings } => {
            use aae_core::network;
            let (_, _, adb) = ctx.connect(&device).await?;
            let on = |value: &str| -> Result<bool> {
                match value {
                    "on" | "yes" | "true" => Ok(true),
                    "off" | "no" | "false" => Ok(false),
                    other => bail!("Use on or off, not {other}."),
                }
            };
            for pair in settings.chunks(2) {
                let [what, value] = pair else {
                    bail!("{} needs a value, such as on or off.", pair[0]);
                };
                match what.as_str() {
                    "airplane" => network::set_airplane(&adb, on(value)?).await?,
                    "wifi" => network::set_wifi(&adb, on(value)?).await?,
                    "data" => network::set_data(&adb, on(value)?).await?,
                    "speed" => {
                        let speed = network::Speed::parse(value).ok_or_else(|| {
                            anyhow::anyhow!("Speeds are full, lte, 3g, slow-3g, edge and gprs.")
                        })?;
                        network::set_speed(&adb, speed).await?
                    }
                    other => {
                        bail!("{other} isn't a network setting: use airplane, wifi, data or speed.")
                    }
                }
            }
            // Android takes a moment to report changes.
            if !settings.is_empty() {
                tokio::time::sleep(std::time::Duration::from_millis(800)).await;
            }
            println!("{}", network::status(&adb).await?.describe());
            Ok(())
        }
        Command::Battery {
            device,
            level,
            charging,
            health,
        } => {
            use aae_core::control::BatteryHealth;
            let health = health
                .map(|h| {
                    BatteryHealth::from_name(&h).ok_or_else(|| {
                        anyhow::anyhow!(
                            "The battery's health is good, failed, dead, overvoltage or overheated."
                        )
                    })
                })
                .transpose()?;
            let (_, controller, _) = ctx.connect(&device).await?;
            controller.set_battery(level, charging).await?;
            if let Some(health) = health {
                controller.set_battery_health(health).await?;
            }
            println!(
                "Battery at {}%, {}, health {}.",
                level.clamp(0, 100),
                if charging { "charging" } else { "not charging" },
                controller.battery_health().await?.label().to_lowercase()
            );
            Ok(())
        }
        Command::Fingerprint { device, finger } => {
            if !(1..=10).contains(&finger) {
                bail!("Fingers are numbered from 1 to 10.");
            }
            let (_, controller, _) = ctx.connect(&device).await?;
            controller.touch_fingerprint(finger).await?;
            println!("Touched the fingerprint sensor with finger {finger}.");
            Ok(())
        }
        Command::Settings {
            device,
            changes,
            list,
            for_new_devices,
            forget_new_devices,
        } => {
            use aae_core::device_settings::{self as settings, SETTINGS};
            if forget_new_devices || for_new_devices {
                if forget_new_devices {
                    settings::set_for_new_devices(&[])?;
                } else {
                    // No device here: every word is a setting or a value.
                    let words: Vec<String> = device.into_iter().chain(changes).collect();
                    if words.len() % 2 != 0 {
                        bail!("Give each setting a value, such as: language en-GB font-size 130");
                    }
                    if !words.is_empty() {
                        let pairs: Vec<(String, String)> = words
                            .chunks(2)
                            .map(|p| {
                                let name = settings::setting(&p[0])
                                    .map_or(p[0].clone(), |s| s.name.to_string());
                                (name, p[1].clone())
                            })
                            .collect();
                        settings::set_for_new_devices(&pairs)?;
                    }
                }
                println!("{}", aae_ffi::new_device_settings_description());
                return Ok(());
            }
            if list || device.is_none() {
                for setting in SETTINGS {
                    println!("{} ({}):", setting.name, setting.label);
                    for (value, label) in setting.choices {
                        println!("  {value}: {label}");
                    }
                }
                println!(
                    "The language also takes any language tags, such as fr-CA or fr-FR,en-US."
                );
                return Ok(());
            }
            let device = device.unwrap_or_default();
            if changes.len() % 2 != 0 {
                bail!("Give each setting a value, such as: font-size 150 dark-theme on");
            }
            let (_, _, adb) = ctx.connect(&device).await?;
            // The helper sets the language.
            provision::update_helper(&ctx.sdk, &adb).await?;
            for pair in changes.chunks(2) {
                println!("{}", settings::change(&adb, &pair[0], &pair[1]).await?);
            }
            if changes.is_empty() {
                for (name, value) in settings::read(&adb).await? {
                    let label = settings::setting(name).map_or(name, |s| s.label);
                    println!("{label}: {}", settings::label_of(name, &value));
                }
            }
            Ok(())
        }
        Command::Record {
            device,
            file,
            seconds,
        } => {
            use aae_core::recording;
            let (_, _, adb) = ctx.connect(&device).await?;
            let limit = seconds
                .unwrap_or(recording::LONGEST)
                .clamp(1, recording::LONGEST);
            // The emulator wants the whole path.
            let file = std::path::absolute(&file)?;
            let file = recording::start(&adb, &file, limit).await?;
            println!(
                "Recording into {} for up to {limit} seconds. Press Control-C to stop.",
                file.display()
            );
            let started = std::time::Instant::now();
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = tokio::time::sleep(std::time::Duration::from_secs(limit as u64)) => {}
            }
            recording::stop(&adb).await?;
            println!(
                "Saved {}, {} seconds.",
                file.display(),
                started.elapsed().as_secs().min(limit as u64)
            );
            Ok(())
        }
        Command::Route {
            device,
            file,
            speed,
        } => {
            use aae_core::route;
            let text = std::fs::read_to_string(&file)
                .with_context(|| format!("{} can't be read", file.display()))?;
            let points = route::parse(&text)?;
            let (_, controller, _) = ctx.connect(&device).await?;
            println!(
                "{} points, taking {:.0} seconds. Press Control-C to stop.",
                points.len(),
                route::duration(&points, speed.clamp(0.1, 100.0))
            );
            tokio::select! {
                played = route::play(&controller, &points, speed) => {
                    played?;
                    println!("The route has finished.");
                }
                _ = tokio::signal::ctrl_c() => println!("Stopped the route."),
            }
            Ok(())
        }
        Command::Hardware { device, changes } => {
            use aae_core::hardware;
            let mut device = ctx.device(&device)?;
            let mut hw = hardware::read(&device)?;
            if changes.is_empty() {
                println!(
                    "{}: {} megabytes of memory, {} processor cores, {} megabytes of storage, a {} by {} screen at {} dots per inch.",
                    device.meta.name,
                    hw.memory_mb,
                    hw.cores,
                    hw.storage_mb,
                    hw.width,
                    hw.height,
                    hw.density
                );
                return Ok(());
            }
            if changes.len() % 2 != 0 {
                bail!("Give each setting a value, such as: memory 4096 cores 6");
            }
            for pair in changes.chunks(2) {
                let (name, value) = (pair[0].to_lowercase(), &pair[1]);
                let number = || {
                    value
                        .parse::<u32>()
                        .map_err(|_| anyhow!("{name} takes a number, not {value}."))
                };
                match name.as_str() {
                    "memory" | "ram" => {
                        hw.memory_mb = hardware::parse_size_mb(value)
                            .filter(|_| value.chars().any(|c| c.is_alphabetic()))
                            .map_or_else(number, Ok)?
                    }
                    "cores" | "cpus" if value.eq_ignore_ascii_case("auto") => {
                        // AAE chooses, for the computer, again.
                        device.meta.cores = None;
                        device.save_meta()?;
                        hw.cores = aae_core::setup::device_cores() as u32;
                    }
                    "cores" | "cpus" => hw.cores = number()?,
                    "storage" | "disk" => {
                        hw.storage_mb = if value.chars().any(|c| c.is_alphabetic()) {
                            hardware::parse_size_mb(value).ok_or_else(|| {
                                anyhow!("Storage is a size, such as 16G or 8192M.")
                            })?
                        } else {
                            number()?
                        }
                    }
                    "screen" | "size" => {
                        let (w, h) = value
                            .split_once(['x', 'X', '×'])
                            .and_then(|(w, h)| {
                                Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
                            })
                            .ok_or_else(|| {
                                anyhow!("The screen is width x height, such as 1080x2400.")
                            })?;
                        hw.width = w;
                        hw.height = h;
                    }
                    "density" | "dpi" => hw.density = number()?,
                    other => bail!(
                        "\"{other}\" isn't hardware AAE changes. Use memory, cores, storage, screen or density."
                    ),
                }
            }
            println!("{}", hardware::write(&mut device, &hw)?);
            Ok(())
        }
        Command::Export { device, file } => {
            let device = ctx.device(&device)?;
            let mut said = 0;
            let file = aae_core::transfer::export(&device, &file, |done, total| {
                let percent = (done * 100 / total.max(1)) as u32;
                if percent >= said + 10 {
                    said = percent / 10 * 10;
                    println!("{said}%");
                }
            })?;
            println!(
                "Exported {} to {}, {:.1} gigabytes.",
                device.meta.name,
                file.display(),
                std::fs::metadata(&file).map_or(0, |m| m.len()) as f64 / 1e9
            );
            Ok(())
        }
        Command::Import { file, name } => {
            let mut said = 0;
            let device = aae_core::transfer::import(
                &ctx.sdk,
                &ctx.store,
                &file,
                name.as_deref(),
                |done, total| {
                    let percent = (done * 100 / total.max(1)) as u32;
                    if percent >= said + 10 {
                        said = percent / 10 * 10;
                        println!("{said}%");
                    }
                },
            )?;
            println!("Imported {}.", device.describe());
            Ok(())
        }
        Command::SpeechBridge { device, action } => {
            use aae_core::speech_bridge::{Bridge, Request};
            let (mut device, _, adb) = ctx.connect(&device).await?;
            provision::update_helper(&ctx.sdk, &adb).await?;
            if action != BridgeAction::Listen {
                let on = action == BridgeAction::On;
                let log = device.meta.speech_log_on();
                aae_core::tts::set_relay(&adb, &mut device, log, on).await?;
                println!(
                    "The speech bridge is {} for {}. The speech log is {}.",
                    if on { "on" } else { "off" },
                    device.meta.name,
                    if log { "on" } else { "off" }
                );
                return Ok(());
            }
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Request>();
            let bridge = Bridge::start(adb, false, move |r| {
                let _ = tx.send(r);
            });
            println!("Listening. Press Control-C to stop.");
            let start = std::time::Instant::now();
            let (done_tx, mut done_rx) = tokio::sync::mpsc::unbounded_channel::<u64>();
            let mut speaking: Option<tokio::task::JoinHandle<()>> = None;
            loop {
                tokio::select! {
                    request = rx.recv() => match request {
                        Some(Request::Speak { id, text, language, rate, pitch }) => {
                            println!("{:7.2}s speak {id} [{language} rate {rate} pitch {pitch}] {text}", start.elapsed().as_secs_f64());
                            // About 15 characters a second.
                            let wait = std::time::Duration::from_millis(200 + text.len() as u64 * 65);
                            let done = done_tx.clone();
                            speaking = Some(tokio::spawn(async move {
                                tokio::time::sleep(wait).await;
                                let _ = done.send(id);
                            }));
                        }
                        Some(Request::Stop) => {
                            println!("{:7.2}s stop", start.elapsed().as_secs_f64());
                            if let Some(task) = speaking.take() { task.abort(); }
                        }
                        None => break,
                    },
                    id = done_rx.recv() => if let Some(id) = id {
                        println!("{:7.2}s done {id}", start.elapsed().as_secs_f64());
                        bridge.finished(id);
                    },
                    _ = tokio::signal::ctrl_c() => break,
                }
            }
            Ok(())
        }
        Command::Shake { device } => {
            let (_, controller, _) = ctx.connect(&device).await?;
            controller.shake().await?;
            println!("Shook the device.");
            Ok(())
        }
        Command::Location {
            device,
            latitude,
            longitude,
        } => {
            let (_, controller, _) = ctx.connect(&device).await?;
            controller.set_location(latitude, longitude).await?;
            println!("Location set to {latitude}, {longitude}.");
            Ok(())
        }
        Command::Sms { device, from, text } => {
            let (_, controller, _) = ctx.connect(&device).await?;
            controller.send_sms(&from, &text).await?;
            println!("Sent a text from {from}.");
            Ok(())
        }
        Command::Call {
            device,
            number,
            end,
        } => {
            let (_, controller, _) = ctx.connect(&device).await?;
            if end {
                controller.phone(Operation::DisconnectCall, &number).await?;
                println!("Ended the call from {number}.");
            } else {
                controller.phone(Operation::InitCall, &number).await?;
                println!("{number} is calling.");
            }
            Ok(())
        }
        Command::Clipboard { device, text } => {
            let (_, controller, _) = ctx.connect(&device).await?;
            match text {
                Some(text) => {
                    controller.set_clipboard(&text).await?;
                    println!("Copied to the device's clipboard.");
                }
                None => {
                    let text = controller.clipboard().await?;
                    if text.is_empty() {
                        println!("The device's clipboard is empty.");
                    } else {
                        println!("{text}");
                    }
                }
            }
            Ok(())
        }
        Command::Volume { device, percent } => {
            let (mut device, _, adb) = ctx.connect(&device).await?;
            let index = provision::boost_volume(&mut device, &adb, percent).await?;
            println!(
                "The screen reader's volume on {} is now {percent}%, step {index}.",
                device.meta.name
            );
            Ok(())
        }
        Command::Screenshot { device, path } => {
            let (device, controller, _) = ctx.connect(&device).await?;
            let png = controller.screenshot_png().await?;
            let path =
                path.unwrap_or_else(|| PathBuf::from(format!("{}-screenshot.png", device.id)));
            std::fs::write(&path, png).with_context(|| format!("Saving {}", path.display()))?;
            println!("Saved the screenshot as {}.", path.display());
            Ok(())
        }
        Command::Shell { device, command } => {
            let (_, _, adb) = ctx.connect(&device).await?;
            let out = adb.shell(&command.join(" ")).await?;
            if !out.is_empty() {
                println!("{out}");
            }
            Ok(())
        }
    }
}

/// Downloads and installs an Android version from Google, with progress every 10%.
async fn download_image(
    ctx: &Ctx,
    api: &str,
    kind: Option<Kind>,
    accept_licence: bool,
) -> Result<sdk::SystemImage> {
    let kind = kind.unwrap_or(Kind::Google);
    let catalogue = tokio::task::spawn_blocking(|| Catalogue::load(false)).await??;
    let Some(image) = catalogue.find_release(api, kind.tag()).cloned() else {
        let offered: Vec<String> = catalogue
            .images
            .iter()
            .filter(|i| i.release.matches(api))
            .map(RemoteImage::describe)
            .collect();
        if offered.is_empty() {
            bail!(
                "Google offers no Android version {api} for this computer. aae available lists those it does."
            );
        }
        bail!(
            "Google doesn't offer that kind of Android version {api}. It offers: {}.",
            offered.join("; ")
        );
    };
    let licence = catalogue
        .licences
        .get(&image.licence_id)
        .cloned()
        .unwrap_or_default();
    if !catalog::licence_accepted(&ctx.sdk, &image.licence_id, &licence) {
        if !accept_licence {
            let path = aae_core::paths::data_dir()
                .join("licences")
                .join(format!("{}.txt", image.licence_id));
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(&path, &licence)?;
            bail!(
                "{} is under Google's licence \"{}\", which you haven't accepted yet. \
                 Read it in {}, then run this again with --accept-licence to accept it.",
                image.describe(),
                image.licence_id,
                path.display()
            );
        }
        catalog::accept_licence(&ctx.sdk, &image.licence_id, &licence)?;
        println!("Accepted Google's licence {}.", image.licence_id);
    }
    if let (Some(needed), Some(have)) = (&image.min_emulator, ctx.sdk.emulator_version()) {
        if version_less(&have, needed) {
            println!(
                "Warning: {} needs emulator version {needed} or later, and this one is {have}. \
                 Run aae setup --update to update it.",
                image.describe()
            );
        }
    }
    println!(
        "Downloading {}, {}.",
        image.describe(),
        human_size(image.size)
    );
    let sdk = ctx.sdk.clone();
    let installed = tokio::task::spawn_blocking(move || {
        let mut last = 0;
        catalog::install(&sdk, &catalogue, &image, |p| match p {
            InstallProgress::Downloading { .. } => {
                let percent = p.percent().unwrap_or(0);
                if percent >= last + 10 {
                    last = percent - percent % 10;
                    println!("Downloaded {last}%.");
                }
            }
            InstallProgress::Verifying => println!("Checking the download."),
            InstallProgress::Unpacking => println!("Unpacking."),
        })
    })
    .await??;
    Ok(installed)
}

/// True if dotted version `a` is lower than `b`.
fn version_less(a: &str, b: &str) -> bool {
    let parts = |v: &str| -> Vec<u32> { v.split('.').map(|p| p.parse().unwrap_or(0)).collect() };
    parts(a) < parts(b)
}

fn pick_image(sdk: &Sdk, api: Option<&str>, kind: Option<Kind>) -> Result<sdk::SystemImage> {
    let images = sdk.system_images();
    let fits = |i: &&sdk::SystemImage| {
        i.runs_natively()
            && api.is_none_or(|a| i.release.matches(a))
            // Previews only when asked for by name.
            && (api.is_some() || i.release.preview.is_none())
            && kind.is_none_or(|k| k.matches(&i.tag))
    };
    if let Some(image) = images.iter().find(fits) {
        return Ok(image.clone());
    }
    let wanted = match api {
        Some(api) => format!("Android version {api}"),
        None => "a suitable Android version".into(),
    };
    if images.is_empty() {
        bail!("{wanted} is not installed, and no Android versions are installed yet.");
    }
    let list: Vec<String> = images.iter().map(ToString::to_string).collect();
    bail!(
        "{wanted} is not installed. Installed versions are: {}.",
        list.join("; ")
    )
}

/// Starts a device, waits for Android, sets it up on first boot, and turns its
/// services back on if they were turned off.
async fn start(
    ctx: &Ctx,
    device: &mut Device,
    start_options: &StartOptions,
    options: &ProvisionOptions,
) -> Result<()> {
    let name = device.meta.name.clone();
    let (_, adb) =
        lifecycle::start_device(&ctx.sdk, &ctx.store, device, start_options, options, |p| {
            println!("{}", p.describe(&name))
        })
        .await?;
    if device.meta.screen_reader.is_none() && !device.meta.screen_reader_declined {
        offer_screen_reader(ctx, device, &adb).await?;
    }
    Ok(())
}

/// Prompts for a screen reader for a device with none. Without a terminal,
/// prints how to add one.
async fn offer_screen_reader(ctx: &Ctx, device: &mut Device, adb: &Adb) -> Result<()> {
    use std::io::{BufRead, IsTerminal, Write};
    let name = device.meta.name.clone();
    let backtalk = device.meta.api >= aae_core::screenreader::BACKTALK_MIN_API;
    if !std::io::stdin().is_terminal() {
        if backtalk {
            println!(
                "{name} has no screen reader. Add one with: aae screen-reader \"{name}\" backtalk, \
                 or aae screen-reader \"{name}\" followed by the path to an APK."
            );
        } else {
            println!(
                "{name} has no screen reader, and Backtalk needs Android 8 or later. Add one made for \
                 this Android version, such as an older TalkBack, with: aae screen-reader \"{name}\" \
                 followed by the path to its APK."
            );
        }
        return Ok(());
    }
    // The choices, numbered in order.
    let mut choices = Vec::new();
    if backtalk {
        println!("{name} has no screen reader. Choose:");
        choices.push(("Download and install Backtalk.", "backtalk"));
    } else {
        println!(
            "{name} has no screen reader, and Backtalk needs Android 8 or later. \
             You can install one made for this Android version, such as an older TalkBack."
        );
    }
    choices.push(("Install a screen reader APK from this computer.", "apk"));
    choices.push((
        "Continue without a screen reader, and don't ask again.",
        "none",
    ));
    for (i, (text, _)) in choices.iter().enumerate() {
        println!("{}. {text}", i + 1);
    }
    let numbers: Vec<String> = (1..=choices.len()).map(|n| n.to_string()).collect();
    let prompt = format!(
        "Type {} or {}: ",
        numbers[..numbers.len() - 1].join(", "),
        numbers[numbers.len() - 1]
    );
    let stdin = std::io::stdin();
    loop {
        print!("{prompt}");
        std::io::stdout().flush()?;
        let mut answer = String::new();
        if stdin.lock().read_line(&mut answer)? == 0 {
            return Ok(());
        }
        let Some(&(_, choice)) = answer
            .trim()
            .parse::<usize>()
            .ok()
            .and_then(|n| choices.get(n.wrapping_sub(1)))
        else {
            continue;
        };
        let path = match choice {
            "backtalk" => {
                println!("Getting Backtalk's latest development build.");
                provision::download_backtalk(device.meta.api).await?
            }
            "apk" => {
                print!("Type the path to the APK: ");
                std::io::stdout().flush()?;
                let mut path = String::new();
                stdin.lock().read_line(&mut path)?;
                // Finder's Copy as Pathname and dragging into Terminal can add quotes.
                PathBuf::from(path.trim().trim_matches(|c| c == '"' || c == '\''))
            }
            _ => {
                device.meta.screen_reader_declined = true;
                device.save_meta()?;
                println!("{name} has no screen reader. AAE won't ask again.");
                return Ok(());
            }
        };
        let package = provision::add_screen_reader(&ctx.sdk, device, adb, &path, false).await?;
        println!("{package} is installed and on.");
        return Ok(());
    }
}

/// Runs `work` with AAE's helper service on, turning it on just for the work
/// if this device doesn't keep it on.
async fn with_helper<T>(
    ctx: &Ctx,
    device: &Device,
    adb: &Adb,
    work: impl std::future::Future<Output = aae_core::Result<T>>,
) -> Result<T> {
    let helper = provision::HELPER_COMPONENT.to_string();
    let kept = device.meta.keep_enabled.contains(&helper);
    provision::update_helper(&ctx.sdk, adb).await?;
    adb.ensure_services(std::slice::from_ref(&helper), provision::SERVICE_TIMEOUT)
        .await?;
    let result = work.await;
    if !kept {
        adb.disable_service(&helper).await?;
    }
    Ok(result?)
}

async fn keytest(ctx: &Ctx, name: &str) -> Result<()> {
    let (device, controller, adb) = ctx.connect_keyboard(name).await?;
    println!(
        "Testing {}'s keyboard. The keys go to AAE's helper, not to any app.",
        device.meta.name
    );
    // The test runs in the helper's service. Turn it on for the test if this
    // device doesn't keep it on.
    let helper = provision::HELPER_COMPONENT.to_string();
    let kept = device.meta.keep_enabled.contains(&helper);
    // Only install when needed: Android stops sending keys to the helper if
    // it is reinstalled while the device runs.
    provision::update_helper(&ctx.sdk, &adb).await?;
    adb.ensure_services(std::slice::from_ref(&helper), provision::SERVICE_TIMEOUT)
        .await?;
    let results = aae_core::keytest::run(&controller, &adb).await;
    if !kept {
        adb.disable_service(&helper).await?;
    }
    let results = results?;
    let failed = results.iter().filter(|r| !r.passed).count();
    for r in &results {
        if r.passed {
            println!("{}: works.", r.name);
        } else {
            println!("{}: doesn't work. Android received {}.", r.name, r.received);
        }
    }
    if aae_core::keytest::received_nothing(&results) {
        bail!(
            "AAE's helper received no keys at all. Android stops sending keys to it after it is updated \
             while the device runs. Restart Android with aae restart, then test again."
        )
    }
    if failed == 0 {
        println!("All {} keys work.", results.len());
        Ok(())
    } else {
        bail!("{failed} of {} keys don't work.", results.len())
    }
}

async fn latency(ctx: &Ctx, name: &str, names: &str, trials: usize) -> Result<()> {
    let keys = names
        .split(',')
        .map(|k| {
            keys::parse(k.trim())
                .ok_or_else(|| anyhow!("\"{k}\" is not a key name. {}", keys::HELP))
        })
        .collect::<Result<Vec<_>>>()?;
    let (device, controller, _) = ctx.connect_keyboard(name).await?;
    println!(
        "Pressing {names} on {} {trials} times, and timing each response.",
        device.meta.name
    );
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    {
        let stop = stop.clone();
        tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                stop.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        });
    }
    let results = aae_core::audio::measure_key_to_sound(
        &controller,
        &keys,
        trials,
        Duration::from_secs(3),
        Duration::from_secs(60),
        &stop,
    )
    .await?;
    let mut heard: Vec<u128> = results.iter().flatten().map(|d| d.as_millis()).collect();
    let silent = results.len() - heard.len();
    for (i, result) in results.iter().enumerate() {
        match result {
            Some(d) => println!("Trial {}: {} milliseconds.", i + 1, d.as_millis()),
            None => println!("Trial {}: no audio detected.", i + 1),
        }
    }
    if heard.is_empty() {
        bail!("No audio detected. Check the screen reader is on and text-to-speech works.");
    }
    heard.sort_unstable();
    println!(
        "Fastest {}, median {}, slowest {} milliseconds.{}",
        heard[0],
        heard[heard.len() / 2],
        heard[heard.len() - 1],
        if silent > 0 {
            format!(" {silent} presses had no response.")
        } else {
            String::new()
        }
    );
    println!(
        "This is the time until the sound leaves the device. Playback adds about 30 milliseconds plus the Mac's own output delay."
    );
    Ok(())
}

async fn status(ctx: &Ctx, name: &str) -> Result<()> {
    let device = ctx.device(name)?;
    println!("{}.", device.describe());
    let Ok(info) = emulator::running(&device) else {
        println!("It is stopped.");
        return Ok(());
    };
    let controller =
        Controller::connect(info.grpc_port, emulator::grpc_token(&info).as_deref()).await?;
    let adb = Adb::new(ctx.sdk.adb_bin()?, info.serial());
    let booted = adb.boot_completed().await;
    println!(
        "It is running{}.",
        if booted { "" } else { " and still starting" }
    );
    if booted {
        let enabled = adb.enabled_services().await?;
        match &device.meta.screen_reader {
            Some(reader) if enabled.iter().any(|c| same_component(c, reader)) => {
                println!("The screen reader, {reader}, is on.")
            }
            Some(reader) => println!(
                "The screen reader, {reader}, is off. Start the device again, or run aae services to turn it on."
            ),
            None => println!("No screen reader is set up."),
        }
        println!("{}.", controller.orientation().await?.describe());
    }
    println!(
        "adb serial {}, gRPC port {}.",
        info.serial(),
        info.grpc_port
    );
    Ok(())
}

async fn setup(ctx: &Ctx, accept_licence: bool, update: bool, refresh: bool) -> Result<()> {
    use aae_core::setup::{self, Tools, Virtualisation};
    let sdk = &ctx.sdk;
    let own = if setup::is_own_sdk(&sdk.root) {
        "AAE's own"
    } else {
        "the"
    };
    println!("Using {own} Android SDK at {}.", sdk.root.display());
    match setup::virtualisation(sdk) {
        Virtualisation::Available => {
            println!("The emulator can use this computer's hardware virtualisation.")
        }
        Virtualisation::Missing(how) => bail!("{how}"),
        Virtualisation::Unknown => {}
    }
    if let Some(warning) = setup::performance_warning() {
        println!("{warning}");
    }
    let tools = tokio::task::spawn_blocking(move || Tools::load(refresh)).await??;
    let mut wanted: Vec<setup::Tool> = tools.missing(sdk).into_iter().cloned().collect();
    let updates: Vec<setup::Tool> = tools.updates(sdk).into_iter().cloned().collect();
    let elsewhere: Vec<String> = tools
        .managed_elsewhere(sdk)
        .iter()
        .map(|t| t.name.clone())
        .collect();
    if !elsewhere.is_empty() {
        println!("Managed outside AAE: {}.", elsewhere.join(", "));
    }
    if update {
        wanted.extend(updates.iter().cloned());
    } else if !updates.is_empty() {
        let names: Vec<String> = updates
            .iter()
            .map(|t| format!("{} {}", t.name, t.revision))
            .collect();
        println!(
            "Updates are available: {}. Run aae setup --update to install them.",
            names.join(", ")
        );
    }
    if wanted.is_empty() {
        println!("Everything AAE needs is installed.");
        return Ok(());
    }
    if update && !updates.is_empty() {
        let running: Vec<String> = ctx
            .store
            .list()?
            .into_iter()
            .filter(|d| emulator::running(d).is_ok())
            .map(|d| d.meta.name)
            .collect();
        if !running.is_empty() {
            bail!(
                "Stop these devices first, as the emulator and adb are replaced: {}.",
                running.join(", ")
            );
        }
    }
    let refs: Vec<&setup::Tool> = wanted.iter().collect();
    for (id, text) in tools.licences_to_accept(sdk, &refs) {
        if !accept_licence {
            let path = aae_core::paths::data_dir()
                .join("licences")
                .join(format!("{id}.txt"));
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(&path, &text)?;
            bail!(
                "The tools are under Google's licence \"{id}\", which you haven't accepted yet. \
                 Read it in {}, then run this again with --accept-licence to accept it.",
                path.display()
            );
        }
        catalog::accept_licence(sdk, &id, &text)?;
        println!("Accepted Google's licence {id}.");
    }
    let total: u64 = wanted.iter().map(|t| t.size).sum();
    if wanted.len() > 1 {
        println!(
            "Downloading {} tools, {} in all.",
            wanted.len(),
            human_size(total)
        );
    }
    let first_setup = wanted
        .iter()
        .any(|t| !updates.iter().any(|u| u.path == t.path));
    for tool in wanted {
        println!(
            "Downloading {} {}, {}.",
            tool.name,
            tool.revision,
            human_size(tool.size)
        );
        let (sdk, tools) = (sdk.clone(), tools.clone());
        let name = tool.name.clone();
        tokio::task::spawn_blocking(move || {
            let mut last = 0;
            setup::install(&sdk, &tools, &tool, |p| match p {
                InstallProgress::Downloading { .. } => {
                    let percent = p.percent().unwrap_or(0);
                    if percent >= last + 10 {
                        last = percent - percent % 10;
                        println!("Downloaded {last}%.");
                    }
                }
                InstallProgress::Verifying => println!("Checking the download."),
                InstallProgress::Unpacking => println!("Unpacking."),
            })
        })
        .await??;
        println!("{name} is installed.");
    }
    if first_setup {
        println!(
            "AAE is ready. Next, download an Android version with aae available and aae download."
        );
    }
    Ok(())
}

fn doctor(ctx: &Ctx) -> Result<()> {
    println!("Android SDK: {}.", ctx.sdk.root.display());
    if let aae_core::setup::Virtualisation::Missing(how) = aae_core::setup::virtualisation(&ctx.sdk)
    {
        println!("Problem: {how}");
    }
    match ctx.sdk.emulator_bin() {
        Ok(_) => println!(
            "Emulator: version {}.",
            ctx.sdk
                .emulator_version()
                .unwrap_or_else(|| "unknown".into())
        ),
        Err(e) => println!("Problem: {e}"),
    }
    match ctx.sdk.adb_bin() {
        Ok(path) => println!("adb: {}.", path.display()),
        Err(e) => println!("Problem: {e}"),
    }
    match ctx.sdk.aapt2_bin() {
        Some(path) => println!("Build tools: {}.", path.display()),
        None => println!(
            "Problem: no build tools. AAE needs them to read app packages. Run aae setup to install them."
        ),
    }
    let native: Vec<_> = ctx
        .sdk
        .system_images()
        .into_iter()
        .filter(|i| i.runs_natively())
        .collect();
    println!(
        "{} Android versions installed that run at full speed here.",
        native.len()
    );
    match provision::default_screen_reader_apk() {
        Some(path) => println!("Default screen reader: {}.", path.display()),
        None => println!(
            "No default screen reader. Pass --screen-reader when creating a device, or put an APK at {}.",
            aae_core::paths::data_dir()
                .join("screen-readers/default.apk")
                .display()
        ),
    }
    println!("Devices are stored in {}.", ctx.store.root.display());
    Ok(())
}

/// An app's package, from its name or package.
async fn app_package(ctx: &Ctx, adb: &Adb, name: &str) -> Result<String> {
    if name.contains('.') && !name.contains(' ') {
        return Ok(name.to_string());
    }
    provision::update_helper(&ctx.sdk, adb).await?;
    aae_core::apps::list(adb, true)
        .await?
        .into_iter()
        .find(|a| a.label.eq_ignore_ascii_case(name.trim()))
        .map(|a| a.package)
        .ok_or_else(|| anyhow!("No app called \"{name}\" is installed. aae apps lists them."))
}

/// Asks whether to turn on part of an app, in a terminal. Without one, an
/// accessibility service is turned on and anything else left off.
fn ask_part(part: &provision::AppPart, package: &str) -> Result<bool> {
    use std::io::{IsTerminal, Write};
    let default = part.kind == aae_core::apk::ServiceKind::Accessibility;
    if !std::io::stdin().is_terminal() {
        return Ok(default);
    }
    let hint = if default { "Y/n" } else { "y/N" };
    loop {
        print!(
            "{package} has {}, {}. Turn it on? [{hint}] ",
            part.kind.describe(),
            part.name
        );
        std::io::stdout().flush()?;
        let mut answer = String::new();
        if std::io::stdin().read_line(&mut answer)? == 0 {
            return Ok(default);
        }
        match answer.trim().to_lowercase().as_str() {
            "" => return Ok(default),
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => continue,
        }
    }
}

/// Reads a point given as "X,Y".
fn parse_point(text: &str) -> Result<(i32, i32), String> {
    text.split_once(',')
        .and_then(|(x, y)| Some((x.trim().parse().ok()?, y.trim().parse().ok()?)))
        .ok_or_else(|| format!("\"{text}\" is not a point. Give it as X,Y, such as 540,1200."))
}

/// Which devices use an Android version, in words.
fn describe_users(users: &catalog::ImageUsers) -> String {
    let mut parts = Vec::new();
    if !users.devices.is_empty() {
        parts.push(format!("Used by {}", users.devices.join(", ")));
    }
    if !users.others.is_empty() {
        parts.push(format!(
            "Used by other emulator devices: {}",
            users.others.join(", ")
        ));
    }
    if parts.is_empty() {
        "No devices use it.".to_string()
    } else {
        parts.join(". ") + "."
    }
}

fn confirm(question: &str) -> Result<bool> {
    use std::io::Write;
    print!("{question} Type yes to confirm: ");
    std::io::stdout().flush()?;
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    Ok(answer.trim().eq_ignore_ascii_case("yes"))
}

/// Serves devices to AAE's Android app until stopped.
async fn serve(port: u16, discovery: bool, json: bool) -> Result<()> {
    // Lines for people, or JSON for AAE's apps.
    let say = |event: serde_json::Value, text: String| {
        if json {
            println!("{event}");
        } else {
            println!("{text}");
        }
    };
    if let Some(pid) = aae_remote::daemon::running_pid() {
        let text = format!(
            "AAE is already serving this computer's devices (process {pid}), as at login. Use aae pair for a pairing code."
        );
        say(
            serde_json::json!({"event": "error", "text": text}),
            text.clone(),
        );
        bail!("Already serving.");
    }
    let engine = aae_ffi::Engine::new()?;
    let name = aae_remote::discovery::computer_name();
    let server = aae_remote::Server::new(engine, name.clone())?;
    aae_remote::daemon::write_pid();
    let (notices, mut notes) = tokio::sync::mpsc::unbounded_channel();
    server.on_notice(notices);
    let _announcement = if discovery {
        match aae_remote::discovery::announce(&name, port, &server.fingerprint) {
            Ok(a) => Some(a),
            Err(e) => {
                say(
                    serde_json::json!({"event": "notice", "text": format!("Couldn't announce this computer on the network ({e}); phones will need its address.")}),
                    format!(
                        "Couldn't announce this computer on the network ({e}); phones will need its address."
                    ),
                );
                None
            }
        }
    } else {
        None
    };
    let running = tokio::spawn(server.clone().run(port));
    say(
        serde_json::json!({"event": "serving", "name": name, "port": port, "fingerprint": server.fingerprint}),
        format!(
            "Serving {name}'s devices to AAE's Android app, on port {port}.\nIts certificate's fingerprint starts {}.",
            &server.fingerprint[..16]
        ),
    );
    let minutes = aae_remote::security::CODE_LIFETIME.as_secs() / 60;
    let show_code = || {
        let code = server.pairing.new_code();
        say(
            serde_json::json!({"event": "code", "code": code, "minutes": minutes}),
            format!(
                "To pair a phone, enter this code in AAE on it: {code}. It works once, for {minutes} minutes."
            ),
        );
    };
    // No terminal at login to show a code; aae pair makes one.
    use std::io::IsTerminal;
    let attended = json || std::io::stdin().is_terminal();
    if attended {
        show_code();
    }
    if !json && attended {
        println!("Press P and Enter for a new pairing code, or Q and Enter to stop.");
    }
    let mut input = tokio::io::BufReader::new(tokio::io::stdin()).lines();
    use tokio::io::AsyncBufReadExt;
    // With no terminal, as when run as a service, it serves until stopped.
    let mut terminal = true;
    // launchd and systemd stop services with SIGTERM.
    #[cfg(unix)]
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let terminated = async {
        #[cfg(unix)]
        terminate.recv().await;
        #[cfg(not(unix))]
        std::future::pending::<()>().await;
    };
    tokio::pin!(terminated);
    loop {
        tokio::select! {
            line = input.next_line(), if terminal => match line {
                Ok(Some(line)) => match line.trim().to_lowercase().as_str() {
                    "p" => show_code(),
                    "q" => break,
                    _ => {}
                },
                // An app that runs aae serve stops it by closing its input.
                _ if json => break,
                _ => terminal = false,
            },
            Some(note) = notes.recv() => say(serde_json::json!({"event": "notice", "text": note}), note),
            _ = tokio::signal::ctrl_c() => break,
            _ = &mut terminated => break,
        }
        if running.is_finished() {
            break;
        }
    }
    // Stopping a device midway would leave its emulator running.
    let stopping = server.stopping();
    if !stopping.is_empty() {
        let text = format!("Waiting for {} to stop.", stopping.join(" and "));
        say(
            serde_json::json!({"event": "notice", "text": text}),
            text.clone(),
        );
        server.wait_for_stops().await;
    }
    aae_remote::daemon::remove_pid();
    if running.is_finished() {
        if let Err(e) = running.await? {
            let text = format!("Serving stopped: {e}");
            say(
                serde_json::json!({"event": "error", "text": text}),
                text.clone(),
            );
            bail!(e);
        }
    }
    if !json {
        println!("Stopped serving.");
    }
    // Exits now: the thread reading standard input would otherwise keep the
    // process waiting for a line that may never come.
    use std::io::Write;
    let _ = std::io::stdout().flush();
    std::process::exit(0);
}

/// Prints the screen, then again each time it changes and settles, until
/// Control-C: AAE's helper counts the changes.
async fn follow_screen(
    ctx: &Ctx,
    device: &aae_core::device::Device,
    adb: &aae_core::adb::Adb,
    json: bool,
    targets: bool,
) -> Result<()> {
    use aae_core::inspector;
    provision::update_helper(&ctx.sdk, adb).await?;
    let show = |tree: &inspector::Tree| -> Result<String> {
        Ok(if targets {
            inspector::targets(tree)
                .iter()
                .map(|t| {
                    let (x, y) = t.centre();
                    format!("{} (at {x}, {y})\n", t.label)
                })
                .collect()
        } else if json {
            serde_json::to_string_pretty(tree)? + "\n"
        } else {
            inspector::to_text(tree)
        })
    };
    let mut last = String::new();
    let mut seen = None;
    loop {
        let changes = inspector::screen_changes(adb).await?.ok_or_else(|| {
            anyhow::anyhow!(
                "AAE's helper isn't running on the device, so it can't follow the screen."
            )
        })?;
        let settled = changes.1 >= 500;
        if seen != Some(changes.0) && settled {
            seen = Some(changes.0);
            let tree = with_helper(ctx, device, adb, inspector::read_tree(adb)).await?;
            let text = show(&tree)?;
            // Ignore changes that leave the printed tree the same, such as a clock.
            if text != last {
                if !last.is_empty() {
                    println!("\nThe screen changed:");
                }
                print!("{text}");
                last = text;
            }
        }
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_millis(500)) => {}
            _ = tokio::signal::ctrl_c() => return Ok(()),
        }
    }
}
