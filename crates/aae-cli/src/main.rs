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
use aae_core::sdk::{self, Sdk, android_name};
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

#[derive(Subcommand)]
enum Command {
    /// Check the Android SDK, emulator and audio, and say what is missing.
    Doctor,
    /// Save a diagnostic report to attach to a bug report: AAE, this
    /// computer, the SDK, your devices, and AAE's log. It never includes
    /// what you typed on a device, and your home folder and account name are
    /// taken out. Read it before sending, if you like: it's plain text.
    Report {
        /// Where to save it. Defaults to a dated file in this folder.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Download the emulator and Android SDK tools AAE needs, so Android
    /// Studio isn't needed. Checks this computer can run the emulator first.
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
    /// Delete an installed Android version, to free its disk space. Refused
    /// while any of AAE's devices use it.
    RemoveImage {
        /// The API level, such as 35.
        #[arg(long)]
        api: u32,
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
    },
    /// Download and install an Android version.
    Download {
        /// The API level, such as 35.
        #[arg(long)]
        api: u32,
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
        /// The API level, such as 35. Defaults to the newest installed.
        #[arg(long)]
        api: Option<u32>,
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
    /// Send this terminal's keyboard and play the device's audio. Control-] returns to the terminal.
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
    /// Check the device's sound reaches AAE: AAE's helper plays a test tone,
    /// which AAE listens for without playing it.
    SoundCheck { device: String },
    /// Measure how fast the device plays audio, so AAE can correct its pitch.
    /// AAE does this by itself on setup; use this to measure again.
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
        /// List only the things that can be touched, in the order gesture
        /// mode's Tab visits them, with their centres.
        #[arg(long)]
        targets: bool,
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
    /// Perform screen reader gestures on the device, in order, such as
    /// swipe-right, swipe-up-then-left, double-tap or two-finger-swipe-down.
    /// With no gestures, lists them all.
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
    /// Install apps. The first time an app has an accessibility service, a
    /// keyboard, a notification listener or a device administrator, AAE asks
    /// whether to turn it on, and remembers the answer for that device.
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
    /// Install a screen reader build and make it the device's screen reader.
    /// Installing a new build of the same screen reader keeps its settings.
    /// Stopped devices get it when they next start.
    #[command(
        after_help = "Use backtalk in place of an APK to download Backtalk's latest development build."
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
    /// List, turn on or turn off accessibility services.
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
    /// Set the battery level, from 0 to 100.
    Battery {
        device: String,
        level: i32,
        #[arg(long)]
        charging: bool,
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
    /// Set the screen reader's volume on the device, from 0 to 100 percent.
    /// Installs AAE's helper on the device if it isn't there.
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
    /// Turn on a service and keep it on, as package/class.
    Enable { component: String },
    /// Turn off a service and stop keeping it on.
    Disable { component: String },
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
enum SnapshotAction {
    /// List the snapshots, newest first, with when each was taken and its notes.
    List,
    /// Save the device as it is now, under a name of your choosing.
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
            println!(
                "Saved the report to {}. It's plain text, if you'd like to read it before sending it.",
                path.display()
            );
            Ok(())
        }
        Command::Setup {
            accept_licence,
            update,
            refresh,
        } => setup(&ctx, accept_licence, update, refresh).await,
        Command::Available { refresh } => {
            let catalogue = tokio::task::spawn_blocking(move || Catalogue::load(refresh)).await??;
            if catalogue.images.is_empty() {
                println!("Google offers no Android versions for this computer's processor.");
            }
            for image in &catalogue.images {
                let state = if image.is_installed(&ctx.sdk) {
                    "installed".to_string()
                } else {
                    format!("{} to download", human_size(image.size))
                };
                println!("{}, {state}.", image.describe());
            }
            Ok(())
        }
        Command::Download {
            api,
            kind,
            accept_licence,
        } => {
            let image = download_image(&ctx, api, kind, accept_licence).await?;
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
                .filter(|i| i.api == api && kind.is_none_or(|k| k.matches(&i.tag)))
                .collect();
            let image = match matching.as_slice() {
                [] => anyhow::bail!(
                    "{} isn't installed. Use the images command to see what is.",
                    aae_core::sdk::android_name(api)
                ),
                [image] => image.clone(),
                several => anyhow::bail!(
                    "More than one kind of {} is installed: {}. Choose one with --kind.",
                    aae_core::sdk::android_name(api),
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
            let image = match (pick_image(&ctx.sdk, api, kind), api) {
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
            targets,
        } => {
            let (device, _, adb) = ctx.connect(&device).await?;
            let tree =
                with_helper(&ctx, &device, &adb, aae_core::inspector::read_tree(&adb)).await?;
            if targets {
                for target in aae_core::inspector::targets(&tree) {
                    let (x, y) = target.centre();
                    println!("{} (at {x}, {y})", target.label);
                }
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
            let reader = device.meta.screen_reader.clone();
            match action {
                SpeechLogAction::On => {
                    if device.meta.speech_log_engine.is_some() {
                        println!("The speech log is already on for {}.", device.meta.name);
                        return Ok(());
                    }
                    provision::update_helper(&ctx.sdk, &adb).await?;
                    let engine = tts::start_speech_log(&adb, reader.as_deref()).await?;
                    device.meta.speech_log_engine = Some(engine.clone());
                    device.save_meta()?;
                    println!("The speech log is on. Speech goes through it to {engine}.");
                }
                SpeechLogAction::Off => {
                    let Some(engine) = device.meta.speech_log_engine.take() else {
                        println!("The speech log is already off for {}.", device.meta.name);
                        return Ok(());
                    };
                    tts::stop_speech_log(&adb, &engine, reader.as_deref()).await?;
                    device.save_meta()?;
                    println!("The speech log is off. Speech goes straight to {engine} again.");
                }
                SpeechLogAction::Show => {
                    let log = tts::speech_log(&adb, 0, false).await?;
                    if log.is_empty() {
                        println!(
                            "Nothing has been said{}.",
                            if device.meta.speech_log_engine.is_none() {
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
                println!("{} can speak, with {}.", device.meta.name, status.engine);
                return Ok(());
            }
            println!(
                "{} can't speak: {}. Repairing it.",
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
            let (_, controller, _) = ctx.connect(&device).await?;
            for name in &names {
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
            let (_, controller, _) = ctx.connect(&device).await?;
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
        Command::ScreenReader {
            device,
            apk,
            replace,
        } => {
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
                        let downloaded =
                            provision::download_backtalk(&ctx.sdk, device.meta.api).await?;
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
            let (mut device, _, adb) = ctx.connect(&device).await?;
            match action {
                None => {
                    let enabled = adb.enabled_services().await?;
                    if enabled.is_empty() {
                        println!("No accessibility services are on.");
                    }
                    for component in enabled {
                        let kept = device.meta.keep_enabled.contains(&component);
                        println!(
                            "{component} is on{}.",
                            if kept { ", and AAE keeps it on" } else { "" }
                        );
                    }
                }
                Some(ServiceAction::Enable { component }) => {
                    device.keep_service_enabled(&component);
                    device.save_meta()?;
                    adb.ensure_services(
                        std::slice::from_ref(&component),
                        provision::SERVICE_TIMEOUT,
                    )
                    .await?;
                    println!("Turned on {component}, and will keep it on.");
                }
                Some(ServiceAction::Disable { component }) => {
                    device.meta.keep_enabled.retain(|c| c != &component);
                    device.save_meta()?;
                    adb.disable_service(&component).await?;
                    println!("Turned off {component}.");
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
        Command::Battery {
            device,
            level,
            charging,
        } => {
            let (_, controller, _) = ctx.connect(&device).await?;
            controller.set_battery(level, charging).await?;
            println!(
                "Battery at {}%, {}.",
                level.clamp(0, 100),
                if charging { "charging" } else { "not charging" }
            );
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
    api: u32,
    kind: Option<Kind>,
    accept_licence: bool,
) -> Result<sdk::SystemImage> {
    let kind = kind.unwrap_or(Kind::Google);
    let catalogue = tokio::task::spawn_blocking(|| Catalogue::load(false)).await??;
    let Some(image) = catalogue.find(api, kind.tag()).cloned() else {
        let offered: Vec<String> = catalogue
            .images
            .iter()
            .filter(|i| i.api == api)
            .map(RemoteImage::describe)
            .collect();
        if offered.is_empty() {
            bail!("Google offers no {} for this computer.", android_name(api));
        }
        bail!(
            "Google doesn't offer that kind of {}. It offers: {}.",
            android_name(api),
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

fn pick_image(sdk: &Sdk, api: Option<u32>, kind: Option<Kind>) -> Result<sdk::SystemImage> {
    let images = sdk.system_images();
    let fits = |i: &&sdk::SystemImage| {
        i.runs_natively()
            && api.is_none_or(|a| i.api == a)
            && kind.is_none_or(|k| k.matches(&i.tag))
    };
    if let Some(image) = images.iter().find(fits) {
        return Ok(image.clone());
    }
    let wanted = match api {
        Some(api) => android_name(api),
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

/// Asks what to do about a device with no screen reader. Without a terminal
/// to ask in, says how to add one.
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
        println!("{name} has no screen reader. What would you like to do?");
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
                provision::download_backtalk(&ctx.sdk, device.meta.api).await?
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
    let (device, controller, adb) = ctx.connect(name).await?;
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
    let (device, controller, _) = ctx.connect(name).await?;
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
            None => println!("Trial {}: nothing heard.", i + 1),
        }
    }
    if heard.is_empty() {
        bail!("Nothing was heard. Check the screen reader is on and speech is working.");
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
    match setup::virtualisation() {
        Virtualisation::Available => println!("This computer can run the emulator at full speed."),
        Virtualisation::Missing(how) => bail!("{how}"),
        Virtualisation::Unknown => {}
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
        let pronoun = if elsewhere.len() == 1 { "it" } else { "them" };
        println!(
            "AAE didn't install the {} here, so it leaves updates to whatever installed {pronoun}, such as Android Studio.",
            elsewhere.join(" or the ")
        );
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
    if let aae_core::setup::Virtualisation::Missing(how) = aae_core::setup::virtualisation() {
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
