//! Controlling a running emulator over its gRPC interface.

use tonic::metadata::MetadataValue;
use tonic::service::Interceptor;
use tonic::service::interceptor::InterceptedService;
use tonic::transport::{Channel, Endpoint};
use tonic::{Request, Status, Streaming};

use crate::error::Result;
use crate::keys::Key;
use crate::proto::android::emulation::control as pb;
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

/// A connection to one running emulator.
#[derive(Clone)]
pub struct Controller {
    emu: EmulatorControllerClient<Svc>,
    snapshots: SnapshotServiceClient<Svc>,
}

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
            emu: EmulatorControllerClient::with_interceptor(channel.clone(), auth.clone())
                .max_decoding_message_size(64 * 1024 * 1024),
            snapshots: SnapshotServiceClient::with_interceptor(channel, auth),
        })
    }

    pub async fn status(&self) -> Result<pb::EmulatorStatus> {
        Ok(self.emu.clone().get_status(()).await?.into_inner())
    }

    pub async fn is_booted(&self) -> Result<bool> {
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
        let event = pb::KeyboardEvent {
            event_type: kind as i32,
            key: key.to_string(),
            ..Default::default()
        };
        self.emu.clone().send_key(event).await?;
        Ok(())
    }

    /// Sends a physical key by its macOS virtual key code. This is how the Mac
    /// app forwards the keyboard: the emulator maps the physical key itself, so
    /// every key and modifier arrives as it was pressed.
    pub async fn mac_key(&self, keycode: u16, down: bool) -> Result<()> {
        self.raw_key(pb::keyboard_event::KeyCodeType::Mac, keycode as i32, down)
            .await
    }

    /// Sends a physical key by its Linux evdev code.
    pub async fn evdev_key(&self, code: i32, down: bool) -> Result<()> {
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
        self.emu.clone().send_key(event).await?;
        Ok(())
    }

    /// Types text as key presses.
    pub async fn type_text(&self, text: &str) -> Result<()> {
        let event = pb::KeyboardEvent {
            text: text.to_string(),
            ..Default::default()
        };
        self.emu.clone().send_key(event).await?;
        Ok(())
    }

    // Touch.

    /// Puts fingers down at the given points (one per finger), or lifts them all
    /// when `points` is empty. Coordinates are in device pixels.
    pub async fn touch(&self, points: &[(i32, i32)], down: bool) -> Result<()> {
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
        self.emu
            .clone()
            .send_touch(pb::TouchEvent {
                touches,
                display: 0,
            })
            .await?;
        Ok(())
    }

    // Device state.

    pub async fn orientation(&self) -> Result<Orientation> {
        let value = self
            .emu
            .clone()
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
        self.emu
            .clone()
            .set_physical_model(pb::PhysicalModelValue {
                target: pb::physical_model_value::PhysicalType::Rotation as i32,
                value: Some(pb::ParameterValue {
                    data: vec![0.0, 0.0, orientation.degrees()],
                }),
                ..Default::default()
            })
            .await?;
        Ok(())
    }

    pub async fn set_battery(&self, level: i32, charging: bool) -> Result<()> {
        use pb::battery_state::{BatteryCharger, BatteryHealth, BatteryStatus};
        let state = pb::BatteryState {
            has_battery: true,
            is_present: true,
            charger: if charging {
                BatteryCharger::Ac
            } else {
                BatteryCharger::None
            } as i32,
            charge_level: level.clamp(0, 100),
            health: BatteryHealth::Good as i32,
            status: if charging {
                BatteryStatus::Charging
            } else {
                BatteryStatus::Discharging
            } as i32,
        };
        self.emu.clone().set_battery(state).await?;
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
        self.emu.clone().set_gps(state).await?;
        Ok(())
    }

    pub async fn send_sms(&self, from: &str, text: &str) -> Result<()> {
        self.emu
            .clone()
            .send_sms(pb::SmsMessage {
                src_address: from.to_string(),
                text: text.to_string(),
            })
            .await?;
        Ok(())
    }

    pub async fn phone(&self, operation: pb::phone_call::Operation, number: &str) -> Result<()> {
        self.emu
            .clone()
            .send_phone(pb::PhoneCall {
                operation: operation as i32,
                number: number.to_string(),
            })
            .await?;
        Ok(())
    }

    pub async fn clipboard(&self) -> Result<String> {
        Ok(self.emu.clone().get_clipboard(()).await?.into_inner().text)
    }

    pub async fn set_clipboard(&self, text: &str) -> Result<()> {
        self.emu
            .clone()
            .set_clipboard(pb::ClipData {
                text: text.to_string(),
            })
            .await?;
        Ok(())
    }

    /// A screenshot as PNG bytes.
    pub async fn screenshot_png(&self) -> Result<Vec<u8>> {
        let format = pb::ImageFormat {
            format: pb::image_format::ImgFormat::Png as i32,
            ..Default::default()
        };
        Ok(self
            .emu
            .clone()
            .get_screenshot(format)
            .await?
            .into_inner()
            .image)
    }

    /// Asks Android to shut down cleanly.
    pub async fn shutdown(&self) -> Result<()> {
        self.set_vm_state(pb::vm_run_state::RunState::Shutdown)
            .await
    }

    pub async fn set_vm_state(&self, state: pb::vm_run_state::RunState) -> Result<()> {
        self.emu
            .clone()
            .set_vm_state(pb::VmRunState {
                state: state as i32,
            })
            .await?;
        Ok(())
    }

    // Audio.

    /// Starts streaming the device's audio output as 16-bit signed samples.
    pub async fn stream_audio(
        &self,
        sample_rate: u32,
        stereo: bool,
    ) -> Result<Streaming<pb::AudioPacket>> {
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
        Ok(self.emu.clone().stream_audio(format).await?.into_inner())
    }

    // Snapshots.

    pub async fn list_snapshots(&self) -> Result<Vec<pb::SnapshotDetails>> {
        let filter = pb::SnapshotFilter {
            status_filter: pb::snapshot_filter::LoadStatus::All as i32,
        };
        Ok(self
            .snapshots
            .clone()
            .list_snapshots(filter)
            .await?
            .into_inner()
            .snapshots)
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
        let mut client = self.snapshots.clone();
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
