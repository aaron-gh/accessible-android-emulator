//! Starting a device all the way to ready: the emulator, Android, first-boot
//! setup, and the service guard. Shared by every front end.

use std::time::Duration;

use crate::adb::Adb;
use crate::control::Controller;
use crate::device::{Device, DeviceStore};
use crate::emulator::{self, BootStage, StartOptions};
use crate::error::{Error, IoContext, Result};
use crate::provision::{self, ProvisionOptions, Step};
use crate::sdk::Sdk;

/// How long Android may take to boot. A first boot on a slow machine takes minutes.
pub const BOOT_TIMEOUT: Duration = Duration::from_secs(600);

/// Progress while a device starts. Each one reads as a sentence through [`Progress::describe`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    Starting,
    ColdBooting,
    AlreadyRunning,
    WaitingForAndroid {
        first_boot: bool,
    },
    AndroidStarted,
    Setup(Step),
    /// Android had turned off a service that AAE keeps on, and AAE turned it back on.
    ServiceRestored(String),
    /// The device couldn't speak, and AAE repaired it. Holds what was done.
    SpeechRepaired(String),
    /// A screen reader build queued while the device was stopped was
    /// installed (its package), or couldn't be (why).
    QueuedScreenReader(std::result::Result<String, String>),
    /// The emulator couldn't start with the computer's graphics adapter, so
    /// AAE started it again drawing in software.
    SoftwareGraphics,
    /// Android had shut down with the battery empty, so AAE cold boots the
    /// device with the battery charged.
    BatteryEmpty,
    /// The device started with its storage locked, waiting for its screen
    /// lock. The speech check was skipped.
    Locked,
    /// The device is ready. Holds the screen reader's package, if there is one.
    Ready(Option<String>),
}

impl Progress {
    pub fn describe(&self, device: &str) -> String {
        match self {
            Progress::Starting => format!("Starting {device}."),
            Progress::ColdBooting => format!("Cold booting {device}."),
            Progress::AlreadyRunning => format!("{device} is already running."),
            Progress::WaitingForAndroid { first_boot: true } => {
                "The emulator is running. Waiting for Android to start, which takes a few minutes the first time."
                    .to_string()
            }
            Progress::WaitingForAndroid { first_boot: false } => {
                "The emulator is running. Waiting for Android to start.".to_string()
            }
            Progress::AndroidStarted => "Android has started.".to_string(),
            Progress::Setup(step) => format!("{}.", step.describe()),
            Progress::ServiceRestored(component) => {
                format!("Android had turned off {component}. It is back on.")
            }
            Progress::SpeechRepaired(what) => what.clone(),
            Progress::SoftwareGraphics => format!(
                "The emulator couldn't use this computer's graphics adapter, so {device} draws its screen in software, which is slower. Starting it again."
            ),
            Progress::BatteryEmpty => format!(
                "Android on {device} had shut down with the battery at 0% and not charging. Cold booting with the battery charged."
            ),
            Progress::Locked => format!(
                "{device} is locked after a cold boot. Type its PIN or password in device mode and press Enter. Speech is checked at the next start."
            ),
            Progress::QueuedScreenReader(Ok(package)) => {
                format!("Installed the new build of {package} queued for {device}.")
            }
            Progress::QueuedScreenReader(Err(why)) => {
                format!("The screen reader build queued for {device} couldn't be installed: {why}")
            }
            Progress::Ready(Some(reader)) => format!("{device} is ready. {reader} is on."),
            Progress::Ready(None) => format!("{device} is ready. It has no screen reader."),
        }
    }
}

/// Wipes a device back to its first-boot state and sets it up again: its
/// apps, data and snapshots go, while its name, hardware, volume and the
/// user's answers about apps stay. Its screen reader is installed again if
/// AAE can find it: its own copy, the one on the device, or Backtalk afresh.
pub async fn wipe_device(
    sdk: &Sdk,
    store: &DeviceStore,
    device: &mut Device,
    mut report: impl FnMut(Progress),
) -> Result<(Controller, Adb)> {
    let adb = match emulator::running(device) {
        Ok(info) => Some(Adb::new(sdk.adb_bin()?, info.serial())),
        Err(_) => None,
    };
    let screen_reader = provision::screen_reader_for_wipe(device, adb.as_ref()).await;
    if adb.is_some() {
        emulator::stop(sdk, device, Duration::from_secs(60)).await?;
    }
    // Snapshots hold the old data, so they go too, the quick-boot one included.
    let snapshots = device.dir.join("snapshots");
    if snapshots.exists() {
        std::fs::remove_dir_all(&snapshots)
            .context(|| format!("Deleting {}", snapshots.display()))?;
    }
    let declined = device.meta.screen_reader_declined && device.meta.screen_reader.is_none();
    let meta = &mut device.meta;
    meta.provisioned = false;
    meta.screen_reader = None;
    meta.screen_reader_declined = declined;
    meta.keep_enabled.clear();
    meta.speech_log_engine = None;
    meta.speech_log = None;
    meta.relay_verified = None;
    meta.speech_bridge = false;
    meta.pending_screen_reader = None;
    device.save_meta()?;
    let start = StartOptions {
        cold_boot: true,
        extra_args: vec!["-wipe-data".into()],
        ..Default::default()
    };
    let setup = ProvisionOptions {
        screen_reader_apk: screen_reader,
        ..Default::default()
    };
    start_device(sdk, store, device, &start, &setup, &mut report).await
}

/// Boots a device without its quick-boot snapshot, stopping it first if it's
/// running, then checks it as a start does. Its apps and data stay; the
/// snapshot is deleted, and the next stop saves a new one.
pub async fn cold_boot_device(
    sdk: &Sdk,
    store: &DeviceStore,
    device: &mut Device,
    setup: &ProvisionOptions,
    mut report: impl FnMut(Progress),
) -> Result<(Controller, Adb)> {
    tracing::info!("{}", Progress::ColdBooting.describe(&device.meta.name));
    report(Progress::ColdBooting);
    if emulator::running(device).is_ok() {
        emulator::stop(sdk, device, Duration::from_secs(60)).await?;
    }
    let start = StartOptions {
        cold_boot: true,
        ..Default::default()
    };
    start_device(sdk, store, device, &start, setup, report).await
}

/// Starts a device if it isn't running, waits for Android, runs first-boot
/// setup if the device has never been set up, and otherwise turns its kept
/// services back on if Android turned any off.
pub async fn start_device(
    sdk: &Sdk,
    store: &DeviceStore,
    device: &mut Device,
    start: &StartOptions,
    setup: &ProvisionOptions,
    mut report: impl FnMut(Progress),
) -> Result<(Controller, Adb)> {
    // Every step goes to the log as well, so a start that stalls shows where.
    let device_name = device.meta.name.clone();
    let mut progress = |p: Progress| {
        tracing::info!("{}", p.describe(&device_name));
        report(p)
    };
    let first_boot = !device.meta.provisioned;
    let (info, starting) = match emulator::running(device) {
        Ok(info) => {
            progress(Progress::AlreadyRunning);
            (info, false)
        }
        Err(_) => {
            progress(Progress::Starting);
            (emulator::start(sdk, store, device, start)?, true)
        }
    };
    let mut stages = |stage| match stage {
        BootStage::WaitingForEmulator => {}
        BootStage::WaitingForAndroid => progress(Progress::WaitingForAndroid { first_boot }),
        BootStage::Ready => progress(Progress::AndroidStarted),
    };
    let ready = emulator::wait_until_ready(sdk, &info, BOOT_TIMEOUT, &mut stages).await;
    let (controller, adb) = match ready {
        // The quick-boot snapshot was saved after Android shut down: start
        // without it, and charge the battery so Android stays up.
        Err(Error::BatteryEmpty) => {
            drop(stages);
            progress(Progress::BatteryEmpty);
            emulator::force_stop(device, &info).await?;
            let cold = StartOptions {
                cold_boot: true,
                ..start.clone()
            };
            let info = emulator::start(sdk, store, device, &cold)?;
            let mut stages = |stage| match stage {
                BootStage::WaitingForEmulator => {}
                BootStage::WaitingForAndroid => {
                    progress(Progress::WaitingForAndroid { first_boot })
                }
                BootStage::Ready => progress(Progress::AndroidStarted),
            };
            let (controller, adb) =
                emulator::wait_until_ready(sdk, &info, BOOT_TIMEOUT, &mut stages).await?;
            controller.charge_battery().await?;
            (controller, adb)
        }
        // The graphics adapter, or its driver, may not work with the
        // emulator. Then it's started again drawing in software, and kept
        // that way if that works.
        Err(Error::EmulatorExited(why))
            if starting
                && !device.meta.software_graphics
                && std::env::var_os("AAE_GPU").is_none() =>
        {
            tracing::warn!("the emulator stopped while starting with the graphics adapter: {why}");
            drop(stages);
            progress(Progress::SoftwareGraphics);
            device.meta.software_graphics = true;
            let info = emulator::start(sdk, store, device, start)?;
            let mut stages = |stage| match stage {
                BootStage::WaitingForEmulator => {}
                BootStage::WaitingForAndroid => {
                    progress(Progress::WaitingForAndroid { first_boot })
                }
                BootStage::Ready => progress(Progress::AndroidStarted),
            };
            match emulator::wait_until_ready(sdk, &info, BOOT_TIMEOUT, &mut stages).await {
                Ok(ready) => {
                    device.save_meta()?;
                    ready
                }
                // Not the graphics, then: say why it first stopped.
                Err(_) => {
                    device.meta.software_graphics = false;
                    return Err(Error::EmulatorExited(why));
                }
            }
        }
        other => other?,
    };

    // After a cold boot of a device with a screen lock. The helper works, but
    // a speech engine that isn't direct boot aware wouldn't, and failing the
    // speech check would replace it.
    let locked = !first_boot && adb.user_locked().await;
    if first_boot {
        provision::provision(sdk, device, &adb, setup, |step| {
            progress(Progress::Setup(step))
        })
        .await?;
    } else {
        provision::apply_keyboard_layout(
            sdk,
            &adb,
            crate::keyboard_layouts::for_device(&device.meta),
        )
        .await?;
        if device.meta.profile == crate::device::Profile::Foldable {
            let folded = crate::fold::is_folded(&adb).await.unwrap_or(false);
            crate::fold::match_screen(&adb, folded).await?;
        }
        if let Some(result) = provision::install_queued_screen_reader(sdk, device, &adb).await {
            progress(Progress::QueuedScreenReader(result));
        }
        for component in provision::guard_services(device, &adb).await? {
            progress(Progress::ServiceRestored(component));
        }
        if device.meta.audio_speed.is_none() {
            provision::measure_audio(device, &adb).await;
        }
        if locked {
            progress(Progress::Locked);
        } else if let Some(what) = provision::ensure_device_speech(device, &adb)
            .await?
            .describe()
        {
            progress(Progress::SpeechRepaired(what.to_string()));
        }
    }
    let reader = device
        .meta
        .screen_reader
        .as_deref()
        .and_then(|c| c.split('/').next())
        .map(String::from);
    progress(Progress::Ready(reader));
    Ok((controller, adb))
}
