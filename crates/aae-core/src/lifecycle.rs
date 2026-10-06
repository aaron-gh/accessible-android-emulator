//! Starting a device all the way to ready: the emulator, Android, first-boot
//! setup, and the service guard. Shared by every front end.

use std::time::Duration;

use crate::adb::Adb;
use crate::control::Controller;
use crate::device::{Device, DeviceStore};
use crate::emulator::{self, BootStage, StartOptions};
use crate::error::Result;
use crate::provision::{self, ProvisionOptions, Step};
use crate::sdk::Sdk;

/// How long Android may take to boot. A first boot on a slow machine takes minutes.
pub const BOOT_TIMEOUT: Duration = Duration::from_secs(600);

/// Progress while a device starts. Each one reads as a sentence through [`Progress::describe`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    Starting,
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
    /// The device is ready. Holds the screen reader's package, if there is one.
    Ready(Option<String>),
}

impl Progress {
    pub fn describe(&self, device: &str) -> String {
        match self {
            Progress::Starting => format!("Starting {device}."),
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
            Progress::Ready(Some(reader)) => format!("{device} is ready. {reader} is on."),
            Progress::Ready(None) => format!("{device} is ready. It has no screen reader."),
        }
    }
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
    let info = match emulator::running(device) {
        Ok(info) => {
            progress(Progress::AlreadyRunning);
            info
        }
        Err(_) => {
            progress(Progress::Starting);
            emulator::start(sdk, store, device, start)?
        }
    };
    let (controller, adb) =
        emulator::wait_until_ready(sdk, &info, BOOT_TIMEOUT, |stage| match stage {
            BootStage::WaitingForEmulator => {}
            BootStage::WaitingForAndroid => progress(Progress::WaitingForAndroid { first_boot }),
            BootStage::Ready => progress(Progress::AndroidStarted),
        })
        .await?;

    if first_boot {
        provision::provision(sdk, device, &adb, setup, |step| {
            progress(Progress::Setup(step))
        })
        .await?;
    } else {
        provision::apply_keyboard_layout(sdk, &adb).await?;
        for component in provision::guard_services(device, &adb).await? {
            progress(Progress::ServiceRestored(component));
        }
        if device.meta.audio_speed.is_none() {
            provision::measure_audio(device, &adb).await;
        }
        if let Some(what) = provision::ensure_device_speech(device, &adb)
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
