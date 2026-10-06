//! `aae`: create, run and test accessible Android virtual devices from the terminal.
//!
//! Every message is a full sentence, printed on its own line, so a screen reader
//! reads it as it arrives. Errors say what went wrong and what to do.

mod attach;

use std::path::PathBuf;
use std::time::Duration;

use aae_core::adb::{Adb, same_component};
use aae_core::control::Orientation;
use aae_core::device::{Device, DeviceStore, Profile, human_size};
use aae_core::emulator::{self, BootStage, StartOptions};
use aae_core::proto::android::emulation::control::phone_call::Operation;
use aae_core::provision::{self, ProvisionOptions};
use aae_core::sdk::{self, Sdk, android_name};
use aae_core::{control::Controller, keys};
use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, Subcommand, ValueEnum};

const BOOT_TIMEOUT: Duration = Duration::from_secs(600);

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
    /// List the Android versions installed on this computer.
    Images,
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
    /// Stop a running device, saving its state for a quick start next time.
    Stop { device: String },
    /// Send this terminal's keyboard and play the device's audio. Control-] returns to the terminal.
    Attach {
        device: String,
        /// Send Option as Alt. By default it is sent as Meta, the screen reader's modifier.
        #[arg(long)]
        keep_alt: bool,
    },
    /// Play the device's audio until Control-C.
    Listen { device: String },
    /// Say what a device is doing.
    Status { device: String },
    /// Measure the time from a key press to hearing the device respond.
    Latency {
        device: String,
        /// The keys to press, in turn, separated by commas. The default moves
        /// TalkBack to the next item and back, so it never runs off the end.
        #[arg(long, default_value = "meta+right,meta+left")]
        keys: String,
        #[arg(long, default_value_t = 10)]
        trials: usize,
    },
    /// Press keys on the device, in order.
    #[command(after_help = keys::HELP)]
    Key { device: String, keys: Vec<String> },
    /// Type text on the device.
    Type { device: String, text: String },
    /// Install apps, turning on any accessibility services they contain.
    Install {
        device: String,
        apks: Vec<PathBuf>,
        /// Don't turn on accessibility services.
        #[arg(long)]
        no_services: bool,
    },
    /// Install a screen reader build and make it the device's screen reader.
    ScreenReader { device: String, apk: PathBuf },
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

#[derive(Subcommand)]
enum SnapshotAction {
    List,
    Save { name: String },
    Load { name: String },
    Delete { name: String },
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_env("AAE_LOG"))
        .with_writer(std::io::stderr)
        .init();
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
            sdk: Sdk::locate()?,
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
        Command::Images => {
            let images = ctx.sdk.system_images();
            if images.is_empty() {
                println!("No Android versions are installed yet.");
            }
            for image in images {
                println!("{image}");
            }
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
            no_animations,
            no_volume_boost,
            no_start,
            attach,
        } => {
            let image = pick_image(&ctx.sdk, api, kind)?;
            let mut device = ctx.store.create(&name, &image, profile.into())?;
            println!("Created {}.", device.describe());
            if no_start {
                return Ok(());
            }
            let options = ProvisionOptions {
                screen_reader_apk: screen_reader,
                disable_animations: no_animations,
                full_volume: !no_volume_boost,
                ..Default::default()
            };
            start(&ctx, &mut device, &StartOptions::default(), &options).await?;
            if attach {
                attach::run(&ctx.sdk, &device, false).await?;
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
            no_volume_boost,
            emulator_args,
            emulator_audio,
            attach,
        } => {
            let mut device = ctx.device(&device)?;
            let options = ProvisionOptions {
                screen_reader_apk: screen_reader,
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
                attach::run(&ctx.sdk, &device, false).await?;
            }
            Ok(())
        }
        Command::Stop { device } => {
            let device = ctx.device(&device)?;
            println!("Stopping {}.", device.meta.name);
            emulator::stop(&ctx.sdk, &device, Duration::from_secs(60)).await?;
            println!("{} is stopped.", device.meta.name);
            Ok(())
        }
        Command::Attach { device, keep_alt } => {
            attach::run(&ctx.sdk, &ctx.device(&device)?, keep_alt).await
        }
        Command::Listen { device } => attach::listen(&ctx.sdk, &ctx.device(&device)?).await,
        Command::Status { device } => status(&ctx, &device).await,
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
        Command::Type { device, text } => {
            let (_, controller, _) = ctx.connect(&device).await?;
            controller.type_text(&text).await?;
            Ok(())
        }
        Command::Install {
            device,
            apks,
            no_services,
        } => {
            let (mut device, _, adb) = ctx.connect(&device).await?;
            for apk in apks {
                println!("Installing {}.", apk.display());
                let (info, enabled) =
                    provision::install_app(&ctx.sdk, &mut device, &adb, &apk, !no_services).await?;
                println!("Installed {}.", info.package);
                for service in &info.services {
                    println!("It has {}: {}.", service.kind.describe(), service.class);
                }
                for component in enabled {
                    println!("Turned on {component}, and will keep it on.");
                }
            }
            Ok(())
        }
        Command::ScreenReader { device, apk } => {
            let (mut device, _, adb) = ctx.connect(&device).await?;
            let info = provision::read_apk(&ctx.sdk, &apk)?;
            let component = info
                .accessibility_components()
                .into_iter()
                .next()
                .ok_or_else(|| {
                    anyhow!(
                        "{} has no accessibility service, so it can't be a screen reader.",
                        apk.display()
                    )
                })?;
            adb.install(&apk).await?;
            if let Some(old) = device.meta.screen_reader.take() {
                if !same_component(&old, &component) {
                    device.meta.keep_enabled.retain(|c| c != &old);
                    adb.disable_service(&old).await?;
                    println!("Turned off the previous screen reader, {old}.");
                }
            }
            device.meta.screen_reader = Some(component.clone());
            device.keep_service_enabled(&component);
            device.save_meta()?;
            adb.ensure_services(std::slice::from_ref(&component), provision::SERVICE_TIMEOUT)
                .await?;
            println!("{} is installed and on.", info.package);
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
                    adb.ensure_services(std::slice::from_ref(&component), provision::SERVICE_TIMEOUT)
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
            match action {
                SnapshotAction::List => {
                    let snapshots = controller.list_snapshots().await?;
                    if snapshots.is_empty() {
                        println!("{} has no snapshots.", device.meta.name);
                    }
                    for s in snapshots {
                        println!("{}, {}.", s.snapshot_id, human_size(s.size));
                    }
                }
                SnapshotAction::Save { name } => {
                    println!("Saving snapshot {name}.");
                    controller.save_snapshot(&name).await?;
                    println!("Saved snapshot {name}.");
                }
                SnapshotAction::Load { name } => {
                    controller.load_snapshot(&name).await?;
                    println!("Restored snapshot {name}.");
                }
                SnapshotAction::Delete { name } => {
                    controller.delete_snapshot(&name).await?;
                    println!("Deleted snapshot {name}.");
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
    let first_boot = !device.meta.provisioned;
    let info = match emulator::running(device) {
        Ok(info) => {
            println!("{} is already running.", device.meta.name);
            info
        }
        Err(_) => {
            println!("Starting {}.", device.meta.name);
            emulator::start(&ctx.sdk, &ctx.store, device, start_options)?
        }
    };
    let (_controller, adb) =
        emulator::wait_until_ready(&ctx.sdk, &info, BOOT_TIMEOUT, |stage| match stage {
            BootStage::WaitingForEmulator => {}
            BootStage::WaitingForAndroid => println!(
                "The emulator is running. Waiting for Android to start{}.",
                if first_boot {
                    ", which takes a few minutes the first time"
                } else {
                    ""
                }
            ),
            BootStage::Ready => println!("Android has started."),
        })
        .await?;

    if first_boot {
        provision::provision(&ctx.sdk, device, &adb, options, |step| {
            println!("{}.", step.describe())
        })
        .await?;
    } else {
        for component in provision::guard_services(device, &adb).await? {
            println!("Android had turned off {component}. It is back on.");
        }
    }
    let reader = device
        .meta
        .screen_reader
        .as_deref()
        .and_then(|c| c.split('/').next())
        .unwrap_or("No screen reader");
    println!("{} is ready. {reader} is on.", device.meta.name);
    Ok(())
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
    let results =
        aae_core::audio::measure_key_to_sound(&controller, &keys, trials, Duration::from_secs(3))
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

fn doctor(ctx: &Ctx) -> Result<()> {
    println!("Android SDK: {}.", ctx.sdk.root.display());
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
        None => println!("Problem: no build tools. AAE needs them to read app packages."),
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

fn confirm(question: &str) -> Result<bool> {
    use std::io::Write;
    print!("{question} Type yes to confirm: ");
    std::io::stdout().flush()?;
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    Ok(answer.trim().eq_ignore_ascii_case("yes"))
}
