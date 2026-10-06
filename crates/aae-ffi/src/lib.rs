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
use aae_core::audio::AudioPlayer;
use aae_core::control::Controller;
use aae_core::device::{Device, DeviceStore, Profile, human_size};
use aae_core::emulator::{self, StartOptions};
use aae_core::provision::{self, ProvisionOptions};
use aae_core::sdk::{Sdk, android_name, image_kind};
use aae_core::speech::{Announcer, Route};
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
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_env("AAE_LOG"))
            .with_writer(std::io::stderr)
            .try_init();
        Ok(Arc::new(Engine {
            sdk: Sdk::locate()?,
            store: DeviceStore::open_default()?,
            announcer: Announcer::new(Route::Best),
        }))
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
    pub async fn start_audio(&self) -> Result<(), AaeError> {
        if self.audio.lock().unwrap().is_some() {
            return Ok(());
        }
        let controller = self.controller.clone();
        let player = on_runtime(async move { Ok(AudioPlayer::start(&controller).await?) }).await?;
        *self.audio.lock().unwrap() = Some(player);
        Ok(())
    }

    pub fn stop_audio(&self) {
        if let Some(mut player) = self.audio.lock().unwrap().take() {
            player.stop();
        }
    }

    /// Sets AAE's playback volume, from 0 to 1.
    pub fn set_audio_volume(&self, volume: f32) {
        if let Some(player) = self.audio.lock().unwrap().as_ref() {
            player.set_volume(volume);
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
    pub async fn install_apk(&self, path: String) -> Result<String, AaeError> {
        let mut device = self.device.lock().unwrap().clone();
        let (sdk, adb) = (self.sdk.clone(), self.adb.clone());
        let (message, device) = on_runtime(async move {
            let (info, enabled) =
                provision::install_app(&sdk, &mut device, &adb, std::path::Path::new(&path), true)
                    .await?;
            let mut message = format!("Installed {}.", info.package);
            for service in enabled {
                message.push_str(&format!(" Turned on {service}."));
            }
            Ok((message, device))
        })
        .await?;
        *self.device.lock().unwrap() = device;
        Ok(message)
    }
}
