//! Controlling a running emulator over its gRPC interface.

use tonic::metadata::MetadataValue;
use tonic::service::Interceptor;
use tonic::service::interceptor::InterceptedService;
use tonic::transport::{Channel, Endpoint};
use tonic::{Request, Status};

use std::pin::Pin;
use std::sync::Arc;

use tokio_stream::{Stream, StreamExt};

use crate::adb::Adb;
use crate::device::RuntimeInfo;
use crate::error::{Error, Result};
use crate::keys::Key;
use crate::proto::android::emulation::control as pb;
use crate::qmp::Qmp;
use pb::emulator_controller_client::EmulatorControllerClient;
use pb::snapshot_service_client::SnapshotServiceClient;

/// Adds the emulator's access token to every request.
#[derive(Clone)]
pub struct Auth {
    header: Option<MetadataValue<tonic::metadata::Ascii>>,
}

impl Interceptor for Auth {
    fn call(&mut self, mut request: Request<()>) -> std::result::Result<Request<()>, Status> {
        if let Some(header) = &self.header {
            request
                .metadata_mut()
                .insert("authorization", header.clone());
        }
        Ok(request)
    }
}

type Svc = InterceptedService<Channel, Auth>;

/// The emulator's own snapshot, which it saves on stop and starts from.
const QUICK_BOOT: &str = "default_boot";

/// A saved snapshot of a device.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub id: String,
    /// The name people gave it.
    pub name: String,
    pub notes: String,
    /// When it was taken, in seconds since 1970.
    pub created: Option<i64>,
    pub size: u64,
    /// The device started from it, or it was the last one restored.
    pub loaded: bool,
    /// False when the emulator won't restore it, for example after an
    /// emulator update.
    pub compatible: bool,
}

impl Snapshot {
    /// When it was taken, in local time, such as "6 Oct 2026, 20:41".
    pub fn taken(&self) -> Option<String> {
        let secs = self.created?;
        if let Some(t) = crate::platform::local_time(u64::try_from(secs).ok()?) {
            const MONTHS: [&str; 12] = [
                "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
            ];
            return Some(format!(
                "{} {} {}, {:02}:{:02}",
                t.day,
                MONTHS[(t.month.clamp(1, 12) - 1) as usize],
                t.year,
                t.hour,
                t.minute
            ));
        }
        Some(format!("{secs} seconds after 1970"))
    }
}

/// One finger on the touchscreen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TouchPoint {
    pub id: i32,
    pub x: i32,
    pub y: i32,
    /// False lifts the finger.
    pub down: bool,
}

/// Device orientation, in the terms a user would use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    Portrait,
    LandscapeLeft,
    UpsideDown,
    LandscapeRight,
}

impl Orientation {
    pub fn describe(self) -> &'static str {
        match self {
            Orientation::Portrait => "Portrait",
            Orientation::LandscapeLeft => "Landscape, turned left",
            Orientation::UpsideDown => "Upside down",
            Orientation::LandscapeRight => "Landscape, turned right",
        }
    }

    /// The rotation around the Z axis, in degrees, as the emulator's physical model expects.
    fn degrees(self) -> f32 {
        match self {
            Orientation::Portrait => 0.0,
            Orientation::LandscapeLeft => 90.0,
            Orientation::UpsideDown => 180.0,
            Orientation::LandscapeRight => -90.0,
        }
    }

    pub fn turned_left(self) -> Self {
        match self {
            Orientation::Portrait => Orientation::LandscapeLeft,
            Orientation::LandscapeLeft => Orientation::UpsideDown,
            Orientation::UpsideDown => Orientation::LandscapeRight,
            Orientation::LandscapeRight => Orientation::Portrait,
        }
    }

    pub fn turned_right(self) -> Self {
        self.turned_left().turned_left().turned_left()
    }

    fn from_degrees(z: f32) -> Self {
        match ((z.round() as i32).rem_euclid(360) + 45) / 90 % 4 {
            1 => Orientation::LandscapeLeft,
            2 => Orientation::UpsideDown,
            3 => Orientation::LandscapeRight,
            _ => Orientation::Portrait,
        }
    }
}

/// A connection to one running device: Google's emulator over gRPC, or a
/// Googlebook device's virtual machine over QMP, SPICE and adb.
#[derive(Clone)]
pub struct Controller {
    emu: Option<EmulatorControllerClient<Svc>>,
    snapshots: Option<SnapshotServiceClient<Svc>>,
    vm: Option<Arc<Vm>>,
}

/// A Googlebook device's virtual machine.
struct Vm {
    qmp: Qmp,
    adb: Adb,
    run_dir: std::path::PathBuf,
}

/// The device's sound, as it arrives.
pub type AudioStream =
    Pin<Box<dyn Stream<Item = std::result::Result<pb::AudioPacket, Status>> + Send>>;

impl Controller {
    /// Connects to the emulator's gRPC port on this computer.
    pub async fn connect(port: u16, token: Option<&str>) -> Result<Self> {
        let channel = Endpoint::from_shared(format!("http://127.0.0.1:{port}"))?
            .connect_timeout(std::time::Duration::from_secs(5))
            .tcp_nodelay(true)
            .connect()
            .await?;
        let header = token
            .map(|t| format!("Bearer {}", t.trim()))
            .and_then(|value| value.parse().ok());
        let auth = Auth { header };
        Ok(Controller {
            emu: Some(
                EmulatorControllerClient::with_interceptor(channel.clone(), auth.clone())
                    .max_decoding_message_size(64 * 1024 * 1024),
            ),
            snapshots: Some(SnapshotServiceClient::with_interceptor(channel, auth)),
            vm: None,
        })
    }

    /// Connects to whatever runs this device.
    pub async fn for_runtime(info: &RuntimeInfo, adb: Adb) -> Result<Self> {
        if info.vm.is_some() {
            Self::connect_vm(info, adb).await
        } else {
            Self::connect(info.grpc_port, crate::emulator::grpc_token(info).as_deref()).await
        }
    }

    /// Connects to a Googlebook device's virtual machine.
    pub async fn connect_vm(info: &RuntimeInfo, adb: Adb) -> Result<Self> {
        let vm = info
            .vm
            .as_ref()
            .ok_or_else(|| Error::Vm("not a Googlebook device".into()))?;
        let qmp = Qmp::connect(&vm.run_dir.join("qmp.sock"), (vm.width, vm.height)).await?;
        Ok(Controller {
            emu: None,
            snapshots: None,
            vm: Some(Arc::new(Vm {
                qmp,
                adb,
                run_dir: vm.run_dir.clone(),
            })),
        })
    }

    /// True for a Googlebook device.
    pub fn is_vm(&self) -> bool {
        self.vm.is_some()
    }

    /// The emulator's gRPC client, or an error naming `what` for a Googlebook device.
    fn emu(&self, what: &'static str) -> Result<EmulatorControllerClient<Svc>> {
        self.emu.clone().ok_or(Error::NotOnGooglebook(what))
    }

    fn snapshot_client(&self) -> Result<SnapshotServiceClient<Svc>> {
        self.snapshots
            .clone()
            .ok_or(Error::NotOnGooglebook("Snapshots"))
    }

    pub async fn status(&self) -> Result<pb::EmulatorStatus> {
        Ok(self
            .emu("The emulator status")?
            .get_status(())
            .await?
            .into_inner())
    }

    pub async fn is_booted(&self) -> Result<bool> {
        if let Some(vm) = &self.vm {
            return Ok(vm.adb.boot_completed().await);
        }
        Ok(self.status().await?.booted)
    }

    // Keyboard.

    /// Presses and releases a key, with its modifiers held around it.
    ///
    /// Keyboard keys go as Linux evdev codes, which the emulator passes to
    /// Android exactly; it ignores modifier keys sent by name. Android's own
    /// buttons, such as Back and Home, have no keyboard code and go by name.
    pub async fn press(&self, key: &Key) -> Result<()> {
        let modifiers: Option<Vec<i32>> = key
            .modifiers
            .iter()
            .map(|m| crate::keys::modifier_code(m))
            .collect();
        if let (Some(mut modifiers), Some((code, shift))) =
            (modifiers, crate::keys::evdev_code(&key.name))
        {
            if shift && !modifiers.contains(&42) {
                modifiers.push(42);
            }
            for &m in &modifiers {
                self.evdev_key(m, true).await?;
            }
            self.evdev_key(code, true).await?;
            self.evdev_key(code, false).await?;
            for &m in modifiers.iter().rev() {
                self.evdev_key(m, false).await?;
            }
            return Ok(());
        }
        for modifier in &key.modifiers {
            self.key_event(modifier, pb::keyboard_event::KeyEventType::Keydown)
                .await?;
        }
        self.key_event(&key.name, pb::keyboard_event::KeyEventType::Keypress)
            .await?;
        for modifier in key.modifiers.iter().rev() {
            self.key_event(modifier, pb::keyboard_event::KeyEventType::Keyup)
                .await?;
        }
        Ok(())
    }

    /// Sends one key down or up by its W3C key name, such as "Enter" or "a".
    pub async fn key_event(&self, key: &str, kind: pb::keyboard_event::KeyEventType) -> Result<()> {
        if let Some(vm) = &self.vm {
            // Keys without a keyboard code are Android's own buttons, which
            // Android takes as key events by name.
            if kind == pb::keyboard_event::KeyEventType::Keyup {
                return Ok(());
            }
            let Some(code) = android_keycode(key) else {
                return Err(Error::Message(format!(
                    "The key {key} can't be sent to a Googlebook device."
                )));
            };
            vm.adb.shell(&format!("input keyevent {code}")).await?;
            return Ok(());
        }
        let event = pb::KeyboardEvent {
            event_type: kind as i32,
            key: key.to_string(),
            ..Default::default()
        };
        self.emu("Keys")?.send_key(event).await?;
        Ok(())
    }

    /// Sends a physical key by its macOS virtual key code. This is how the Mac
    /// app forwards the keyboard: the emulator maps the physical key itself, so
    /// every key and modifier arrives as it was pressed.
    pub async fn mac_key(&self, keycode: u16, down: bool) -> Result<()> {
        if self.vm.is_some() {
            return match crate::keys::mac_to_evdev(keycode) {
                Some(code) => self.evdev_key(code, down).await,
                None => Ok(()),
            };
        }
        self.raw_key(pb::keyboard_event::KeyCodeType::Mac, keycode as i32, down)
            .await
    }

    /// Sends a physical key by its Linux evdev code.
    pub async fn evdev_key(&self, code: i32, down: bool) -> Result<()> {
        if let Some(vm) = &self.vm {
            if !vm.qmp.evdev_key(code, down).await? && down {
                // Android's own buttons, such as Back, which a USB keyboard doesn't have.
                if let Some(key) = android_key_for_evdev(code) {
                    vm.adb.shell(&format!("input keyevent {key}")).await?;
                }
            }
            return Ok(());
        }
        self.raw_key(pb::keyboard_event::KeyCodeType::Evdev, code, down)
            .await
    }

    async fn raw_key(
        &self,
        code_type: pb::keyboard_event::KeyCodeType,
        code: i32,
        down: bool,
    ) -> Result<()> {
        let kind = if down {
            pb::keyboard_event::KeyEventType::Keydown
        } else {
            pb::keyboard_event::KeyEventType::Keyup
        };
        let event = pb::KeyboardEvent {
            code_type: code_type as i32,
            event_type: kind as i32,
            key_code: code,
            ..Default::default()
        };
        self.emu("Keys")?.send_key(event).await?;
        Ok(())
    }

    /// Types text as key presses.
    pub async fn type_text(&self, text: &str) -> Result<()> {
        if self.vm.is_some() {
            return self.type_on_keyboard(text).await;
        }
        let event = pb::KeyboardEvent {
            text: text.to_string(),
            ..Default::default()
        };
        self.emu("Typing")?.send_key(event).await?;
        Ok(())
    }

    // Touch.

    /// Puts fingers down at the given points (one per finger), or lifts them all
    /// when `points` is empty. Coordinates are in device pixels.
    pub async fn touch(&self, points: &[(i32, i32)], down: bool) -> Result<()> {
        if let Some(vm) = &self.vm {
            let points: Vec<TouchPoint> = if down {
                points
                    .iter()
                    .enumerate()
                    .map(|(i, &(x, y))| TouchPoint {
                        id: i as i32,
                        x,
                        y,
                        down: true,
                    })
                    .collect()
            } else {
                (0..10)
                    .map(|id| TouchPoint {
                        id,
                        x: 0,
                        y: 0,
                        down: false,
                    })
                    .collect()
            };
            return vm.qmp.touch_points(&points).await;
        }
        let touches = points
            .iter()
            .enumerate()
            .map(|(i, &(x, y))| pb::Touch {
                x,
                y,
                identifier: i as i32,
                pressure: if down { 1 } else { 0 },
                touch_major: if down { 8 } else { 0 },
                ..Default::default()
            })
            .collect();
        self.emu("Touch")?
            .send_touch(pb::TouchEvent {
                touches,
                display: 0,
            })
            .await?;
        Ok(())
    }

    /// Moves, puts down or lifts fingers, each by its id, in one event.
    /// Coordinates are touchscreen pixels, unrotated.
    pub async fn touch_points(&self, points: &[TouchPoint]) -> Result<()> {
        if let Some(vm) = &self.vm {
            return vm.qmp.touch_points(points).await;
        }
        let touches = points
            .iter()
            .map(|p| pb::Touch {
                x: p.x,
                y: p.y,
                identifier: p.id,
                pressure: if p.down { 1 } else { 0 },
                touch_major: if p.down { 8 } else { 0 },
                ..Default::default()
            })
            .collect();
        self.emu("Touch")?
            .send_touch(pb::TouchEvent {
                touches,
                display: 0,
            })
            .await?;
        Ok(())
    }

    // Device state.

    pub async fn orientation(&self) -> Result<Orientation> {
        if self.vm.is_some() {
            return Ok(Orientation::Portrait);
        }
        let value = self
            .emu("Rotation")?
            .get_physical_model(pb::PhysicalModelValue {
                target: pb::physical_model_value::PhysicalType::Rotation as i32,
                ..Default::default()
            })
            .await?
            .into_inner();
        let z = value
            .value
            .and_then(|v| v.data.get(2).copied())
            .unwrap_or(0.0);
        Ok(Orientation::from_degrees(z))
    }

    pub async fn set_orientation(&self, orientation: Orientation) -> Result<()> {
        self.emu("Rotation")?
            .set_physical_model(pb::PhysicalModelValue {
                target: pb::physical_model_value::PhysicalType::Rotation as i32,
                value: Some(pb::ParameterValue {
                    data: vec![0.0, 0.0, orientation.degrees()],
                }),
                // Turn at once. A smooth turn takes a moment, and reading the
                // orientation during it gives the old one, so a quick second
                // rotation started from the wrong place.
                interpolation: pb::physical_model_value::Interpolation::Step as i32,
                ..Default::default()
            })
            .await?;
        Ok(())
    }

    pub async fn set_battery(&self, level: i32, charging: bool) -> Result<()> {
        use pb::battery_state::{BatteryCharger, BatteryStatus};
        // Keeps the health as it was set.
        let health = self
            .emu("Battery")?
            .get_battery(())
            .await
            .map_or(0, |state| state.into_inner().health);
        let state = pb::BatteryState {
            has_battery: true,
            is_present: true,
            charger: if charging {
                BatteryCharger::Ac
            } else {
                BatteryCharger::None
            } as i32,
            charge_level: level.clamp(0, 100),
            health,
            status: if charging {
                BatteryStatus::Charging
            } else {
                BatteryStatus::Discharging
            } as i32,
        };
        self.emu("Battery")?.set_battery(state).await?;
        Ok(())
    }

    /// Sets the battery's health, keeping its level and charging.
    pub async fn set_battery_health(&self, health: BatteryHealth) -> Result<()> {
        let mut state = self.emu("Battery")?.get_battery(()).await?.into_inner();
        state.health = health.proto() as i32;
        self.emu("Battery")?.set_battery(state).await?;
        Ok(())
    }

    /// The battery's health.
    pub async fn battery_health(&self) -> Result<BatteryHealth> {
        let state = self.emu("Battery")?.get_battery(()).await?.into_inner();
        Ok(BatteryHealth::from_proto(state.health))
    }

    /// Whether the battery is at 0% and not charging, where Android shuts down.
    pub async fn battery_empty(&self) -> Result<bool> {
        use pb::battery_state::BatteryCharger;
        let state = self.emu("Battery")?.get_battery(()).await?.into_inner();
        Ok(state.charge_level == 0 && state.charger == BatteryCharger::None as i32)
    }

    /// Sets the battery to 100%, charging, with good health.
    pub async fn charge_battery(&self) -> Result<()> {
        self.set_battery(100, true).await?;
        self.set_battery_health(BatteryHealth::Good).await
    }

    /// Touches the fingerprint sensor with a finger, as Android's settings
    /// enrolled it, then lifts it. Finger numbers are the emulator's own: a
    /// finger enrolled while AAE touched with finger 1 is finger 1 after.
    pub async fn touch_fingerprint(&self, finger: i32) -> Result<()> {
        let mut emu = self.emu("Fingerprint")?;
        emu.send_fingerprint(pb::Fingerprint {
            is_touching: true,
            touch_id: finger,
        })
        .await?;
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        emu.send_fingerprint(pb::Fingerprint {
            is_touching: false,
            touch_id: finger,
        })
        .await?;
        Ok(())
    }

    /// Shakes the device, as a hand would: side to side, quickly, for a
    /// second, so the accelerometer feels it.
    pub async fn shake(&self) -> Result<()> {
        use pb::physical_model_value::{Interpolation, PhysicalType};
        let mut emu = self.emu("Shake")?;
        let position = |x: f32| pb::PhysicalModelValue {
            target: PhysicalType::Position as i32,
            value: Some(pb::ParameterValue {
                data: vec![x, 0.0, 0.0],
            }),
            interpolation: Interpolation::Smooth as i32,
            ..Default::default()
        };
        for step in 0..12 {
            let x = if step % 2 == 0 { 0.5 } else { -0.5 };
            emu.set_physical_model(position(x)).await?;
            tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        }
        emu.set_physical_model(position(0.0)).await?;
        Ok(())
    }

    /// Sets where the device is, moving: with its altitude in metres, speed
    /// in metres a second, and heading in degrees from north.
    pub async fn set_moving_location(
        &self,
        latitude: f64,
        longitude: f64,
        altitude: f64,
        speed: f64,
        bearing: f64,
    ) -> Result<()> {
        let state = pb::GpsState {
            passive_update: false,
            latitude,
            longitude,
            altitude,
            speed,
            bearing,
            satellites: 8,
        };
        self.emu("Location")?.set_gps(state).await?;
        Ok(())
    }

    pub async fn set_location(&self, latitude: f64, longitude: f64) -> Result<()> {
        let state = pb::GpsState {
            passive_update: false,
            latitude,
            longitude,
            altitude: 10.0,
            satellites: 8,
            ..Default::default()
        };
        self.emu("Location")?.set_gps(state).await?;
        Ok(())
    }

    pub async fn send_sms(&self, from: &str, text: &str) -> Result<()> {
        self.emu("Text messages")?
            .send_sms(pb::SmsMessage {
                src_address: from.to_string(),
                text: text.to_string(),
            })
            .await?;
        Ok(())
    }

    pub async fn phone(&self, operation: pb::phone_call::Operation, number: &str) -> Result<()> {
        self.emu("Phone calls")?
            .send_phone(pb::PhoneCall {
                operation: operation as i32,
                number: number.to_string(),
            })
            .await?;
        Ok(())
    }

    pub async fn clipboard(&self) -> Result<String> {
        Ok(self
            .emu("Clipboard")?
            .get_clipboard(())
            .await?
            .into_inner()
            .text)
    }

    pub async fn set_clipboard(&self, text: &str) -> Result<()> {
        self.emu("Clipboard")?
            .set_clipboard(pb::ClipData {
                text: text.to_string(),
            })
            .await?;
        Ok(())
    }

    /// A screenshot as PNG bytes.
    pub async fn screenshot_png(&self) -> Result<Vec<u8>> {
        self.screenshot_png_scaled(0).await
    }

    /// A screenshot scaled down to at most `width` pixels across, keeping its
    /// shape; 0 keeps the screen's own size.
    pub async fn screenshot_png_scaled(&self, width: u32) -> Result<Vec<u8>> {
        if let Some(vm) = &self.vm {
            // Full size: the VM's display isn't scaled.
            let _ = width;
            return vm.adb.screenshot_png().await;
        }
        let format = pb::ImageFormat {
            format: pb::image_format::ImgFormat::Png as i32,
            width,
            ..Default::default()
        };
        Ok(self
            .emu("Screenshots")?
            .get_screenshot(format)
            .await?
            .into_inner()
            .image)
    }

    /// Asks Android to shut down cleanly.
    pub async fn shutdown(&self) -> Result<()> {
        if let Some(vm) = &self.vm {
            return crate::googlebook::power_off(&vm.adb).await;
        }
        self.set_vm_state(pb::vm_run_state::RunState::Shutdown)
            .await
    }

    pub async fn set_vm_state(&self, state: pb::vm_run_state::RunState) -> Result<()> {
        self.emu("Pausing")?
            .set_vm_state(pb::VmRunState {
                state: state as i32,
            })
            .await?;
        Ok(())
    }

    // Audio.

    /// Lets sound into the device's microphone. The emulator zeroes it
    /// otherwise.
    pub async fn allow_microphone(&self, allowed: bool) -> Result<()> {
        self.emu("Microphone")?
            .set_microphone_state(pb::MicrophoneState {
                real_audio_enabled: allowed,
            })
            .await?;
        Ok(())
    }

    /// Sends audio into the device's microphone until `packets` ends.
    /// Only one source can feed the microphone at a time.
    pub async fn inject_audio(
        &self,
        packets: impl tokio_stream::Stream<Item = pb::AudioPacket> + Send + 'static,
    ) -> Result<()> {
        self.emu("Microphone")?.inject_audio(packets).await?;
        Ok(())
    }

    /// Starts streaming the device's audio output as 16-bit signed samples.
    pub async fn stream_audio(&self, sample_rate: u32, stereo: bool) -> Result<AudioStream> {
        use pb::audio_format::{Channels, SampleFormat};
        let format = pb::AudioFormat {
            sampling_rate: sample_rate as u64,
            channels: if stereo {
                Channels::Stereo
            } else {
                Channels::Mono
            } as i32,
            format: SampleFormat::AudFmtS16 as i32,
            mode: 0,
        };
        if let Some(vm) = &self.vm {
            // The VM's sound is always stereo.
            let blocks =
                crate::spice::playback(&crate::spice::socket(&vm.run_dir), sample_rate).await?;
            let stream = tokio_stream::wrappers::ReceiverStream::new(blocks).map(move |block| {
                let mut audio = Vec::with_capacity(block.samples.len() * 2);
                for sample in block.samples {
                    audio.extend_from_slice(&sample.to_le_bytes());
                }
                Ok(pb::AudioPacket {
                    format: Some(format),
                    timestamp: block.timestamp,
                    audio,
                })
            });
            return Ok(Box::pin(stream));
        }
        let stream = self.emu("Sound")?.stream_audio(format).await?.into_inner();
        Ok(Box::pin(stream))
    }

    // Snapshots.

    pub async fn list_snapshots(&self) -> Result<Vec<pb::SnapshotDetails>> {
        let filter = pb::SnapshotFilter {
            status_filter: pb::snapshot_filter::LoadStatus::All as i32,
        };
        Ok(self
            .snapshot_client()?
            .list_snapshots(filter)
            .await?
            .into_inner()
            .snapshots)
    }

    /// The snapshots people have saved, newest first. The emulator's own
    /// quick-boot state is left out.
    pub async fn snapshots(&self) -> Result<Vec<Snapshot>> {
        let mut list: Vec<Snapshot> = self
            .list_snapshots()
            .await?
            .into_iter()
            .filter(|s| s.snapshot_id != QUICK_BOOT)
            .map(|s| {
                let details = s.details.unwrap_or_default();
                Snapshot {
                    name: details
                        .logical_name
                        .clone()
                        .filter(|n| !n.is_empty())
                        .unwrap_or_else(|| s.snapshot_id.clone()),
                    notes: details.description.clone().unwrap_or_default(),
                    created: details.creation_time,
                    size: s.size,
                    loaded: s.status == pb::snapshot_details::LoadStatus::Loaded as i32,
                    compatible: s.status != pb::snapshot_details::LoadStatus::Incompatible as i32,
                    id: s.snapshot_id,
                }
            })
            .collect();
        list.sort_by_key(|s| std::cmp::Reverse(s.created));
        Ok(list)
    }

    /// Saves a snapshot under a name people choose, with notes. Returns its id.
    pub async fn save_named_snapshot(&self, name: &str, notes: &str) -> Result<String> {
        let slug: String = name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect::<String>()
            .split('-')
            .filter(|p| !p.is_empty())
            .collect::<Vec<_>>()
            .join("-");
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let id = if slug.is_empty() {
            format!("snapshot-{stamp}")
        } else {
            format!("{slug}-{stamp}")
        };
        self.save_snapshot(&id).await?;
        self.update_snapshot(&id, name, notes).await?;
        Ok(id)
    }

    /// Changes a snapshot's name and notes. Its id stays the same.
    pub async fn update_snapshot(&self, id: &str, name: &str, notes: &str) -> Result<()> {
        self.snapshot_client()?
            .update_snapshot(pb::SnapshotUpdateDescription {
                snapshot_id: id.to_string(),
                logical_name: Some(name.trim().to_string()),
                description: Some(notes.trim().to_string()),
            })
            .await?;
        Ok(())
    }

    pub async fn save_snapshot(&self, name: &str) -> Result<()> {
        self.snapshot_call(name, SnapshotOp::Save).await
    }

    pub async fn load_snapshot(&self, name: &str) -> Result<()> {
        self.snapshot_call(name, SnapshotOp::Load).await
    }

    pub async fn delete_snapshot(&self, name: &str) -> Result<()> {
        self.snapshot_call(name, SnapshotOp::Delete).await
    }

    async fn snapshot_call(&self, name: &str, op: SnapshotOp) -> Result<()> {
        let package = pb::SnapshotPackage {
            snapshot_id: name.to_string(),
            ..Default::default()
        };
        let mut client = self.snapshot_client()?;
        let reply = match op {
            SnapshotOp::Save => client.save_snapshot(package).await?,
            SnapshotOp::Load => client.load_snapshot(package).await?,
            SnapshotOp::Delete => client.delete_snapshot(package).await?,
        }
        .into_inner();
        if reply.success {
            Ok(())
        } else {
            let reason = String::from_utf8_lossy(&reply.err).trim().to_string();
            Err(Status::failed_precondition(if reason.is_empty() {
                format!("The snapshot \"{name}\" could not be {}.", op.past_tense())
            } else {
                reason
            })
            .into())
        }
    }
}

impl Controller {
    /// Types text on a Googlebook device's keyboard, key by key, as AAE's
    /// keyboard layout maps them. Characters it has no key for go through
    /// Android's input command.
    async fn type_on_keyboard(&self, text: &str) -> Result<()> {
        let vm = self.vm.as_ref().expect("only for Googlebook devices");
        let mut others = String::new();
        for c in text.chars() {
            let name = match c {
                '\n' => "Enter".to_string(),
                '\t' => "Tab".to_string(),
                c => c.to_string(),
            };
            match crate::keys::evdev_code(&name) {
                Some((code, shift)) if others.is_empty() => {
                    if shift {
                        vm.qmp.evdev_key(42, true).await?;
                    }
                    vm.qmp.evdev_key(code, true).await?;
                    vm.qmp.evdev_key(code, false).await?;
                    if shift {
                        vm.qmp.evdev_key(42, false).await?;
                    }
                }
                _ => others.push(c),
            }
        }
        if !others.is_empty() {
            let quoted = others.replace('\'', "'\\''");
            vm.adb.shell(&format!("input text '{quoted}'")).await?;
        }
        Ok(())
    }
}

/// Android's key code for one of its own buttons, by the name AAE gives it.
fn android_keycode(name: &str) -> Option<&'static str> {
    Some(match name {
        "GoBack" => "KEYCODE_BACK",
        "GoHome" => "KEYCODE_HOME",
        "AppSwitch" => "KEYCODE_APP_SWITCH",
        "Power" => "KEYCODE_POWER",
        "AudioVolumeUp" => "KEYCODE_VOLUME_UP",
        "AudioVolumeDown" => "KEYCODE_VOLUME_DOWN",
        "AudioVolumeMute" => "KEYCODE_VOLUME_MUTE",
        "Assist" | "Assistant" => "KEYCODE_ASSIST",
        "Notification" => "KEYCODE_NOTIFICATION",
        _ => return None,
    })
}

/// Android's key code for an evdev code a USB keyboard doesn't have.
fn android_key_for_evdev(code: i32) -> Option<&'static str> {
    Some(match code {
        116 => "KEYCODE_POWER",
        158 => "KEYCODE_BACK",
        163 => "KEYCODE_MEDIA_NEXT",
        164 => "KEYCODE_MEDIA_PLAY_PAUSE",
        165 => "KEYCODE_MEDIA_PREVIOUS",
        166 => "KEYCODE_MEDIA_STOP",
        172 => "KEYCODE_HOME",
        580 => "KEYCODE_APP_SWITCH",
        _ => return None,
    })
}

#[derive(Clone, Copy)]
enum SnapshotOp {
    Save,
    Load,
    Delete,
}

impl SnapshotOp {
    fn past_tense(self) -> &'static str {
        match self {
            SnapshotOp::Save => "saved",
            SnapshotOp::Load => "restored",
            SnapshotOp::Delete => "deleted",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orientation_round_trips() {
        for o in [
            Orientation::Portrait,
            Orientation::LandscapeLeft,
            Orientation::UpsideDown,
            Orientation::LandscapeRight,
        ] {
            assert_eq!(Orientation::from_degrees(o.degrees()), o);
            assert_eq!(o.turned_left().turned_right(), o);
        }
    }
}

/// A battery's health, as Android reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BatteryHealth {
    Good,
    Failed,
    Dead,
    Overvoltage,
    Overheated,
}

impl BatteryHealth {
    pub const ALL: [BatteryHealth; 5] = [
        BatteryHealth::Good,
        BatteryHealth::Failed,
        BatteryHealth::Dead,
        BatteryHealth::Overvoltage,
        BatteryHealth::Overheated,
    ];

    /// Its name for commands, such as "overheated".
    pub fn name(self) -> &'static str {
        match self {
            BatteryHealth::Good => "good",
            BatteryHealth::Failed => "failed",
            BatteryHealth::Dead => "dead",
            BatteryHealth::Overvoltage => "overvoltage",
            BatteryHealth::Overheated => "overheated",
        }
    }

    /// How to say it, such as "Overheated".
    pub fn label(self) -> &'static str {
        match self {
            BatteryHealth::Good => "Good",
            BatteryHealth::Failed => "Failed",
            BatteryHealth::Dead => "Dead",
            BatteryHealth::Overvoltage => "Over voltage",
            BatteryHealth::Overheated => "Overheated",
        }
    }

    pub fn from_name(name: &str) -> Option<BatteryHealth> {
        let name = name.trim().to_lowercase().replace([' ', '-'], "");
        Self::ALL.into_iter().find(|h| h.name() == name)
    }

    fn proto(self) -> pb::battery_state::BatteryHealth {
        use pb::battery_state::BatteryHealth as P;
        match self {
            BatteryHealth::Good => P::Good,
            BatteryHealth::Failed => P::Failed,
            BatteryHealth::Dead => P::Dead,
            BatteryHealth::Overvoltage => P::Overvoltage,
            BatteryHealth::Overheated => P::Overheated,
        }
    }

    fn from_proto(value: i32) -> BatteryHealth {
        Self::ALL
            .into_iter()
            .find(|h| h.proto() as i32 == value)
            .unwrap_or(BatteryHealth::Good)
    }
}
