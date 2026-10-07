//! The attached device's sound and touches, to and from one phone.

use std::sync::Arc;

use aae_core::control::TouchPoint;
use aae_ffi::Session;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::server::{AUDIO_FRAME, AUDIO_RATE, Out};

/// The phone's microphone sound: 48 kHz mono.
const MICROPHONE_RATE: u32 = 48_000;

/// Speaks for the device's screen reader on the phone, for the speech
/// bridge: `{"type":"event","event":"speak","id":…,"text":…,"language":…}`
/// and `{"type":"event","event":"speech_stop"}`. The phone answers each
/// utterance with `{"type":"speech_done","id":…}` when it has finished or
/// stopped.
struct PhoneSpeaker {
    out: Out,
}

impl aae_ffi::SpeechBridgeListener for PhoneSpeaker {
    fn speak(&self, id: u64, text: String, language: String, rate: u32, pitch: u32) {
        self.out.json(json!({
            "type": "event",
            "event": "speak",
            "id": id,
            "text": text,
            "language": language,
            "rate": rate,
            "pitch": pitch,
        }));
    }

    fn stop(&self) {
        self.out
            .json(json!({"type": "event", "event": "speech_stop"}));
    }
}

/// A device attached to a phone.
pub struct Attachment {
    pub session: Arc<Session>,
    touches: mpsc::UnboundedSender<Vec<TouchPoint>>,
    tasks: Vec<JoinHandle<()>>,
    out: Out,
    /// The phone's microphone stream, while on.
    microphone: Option<(mpsc::Sender<Vec<i16>>, JoinHandle<()>)>,
}

impl Attachment {
    pub async fn start(session: Arc<Session>, out: Out) -> Attachment {
        let mut tasks = Vec::new();

        // The device's sound, as it comes. Nothing plays on the computer.
        let controller = session.controller();
        let speed = session.audio_speed();
        let sound = out.clone();
        tasks.push(tokio::spawn(async move {
            let Ok(mut samples) = aae_core::audio::pcm_stream(&controller, AUDIO_RATE, speed).await
            else {
                return;
            };
            while let Some(samples) = samples.recv().await {
                let mut frame = Vec::with_capacity(1 + samples.len() * 2);
                frame.push(AUDIO_FRAME);
                for sample in samples {
                    frame.extend_from_slice(&sample.to_le_bytes());
                }
                if !sound.binary(frame) {
                    break;
                }
            }
        }));

        // Touches go through one queue, so they reach the device in order.
        let (touches, mut queue) = mpsc::unbounded_channel::<Vec<TouchPoint>>();
        let controller = session.controller();
        tasks.push(tokio::spawn(async move {
            while let Some(points) = queue.recv().await {
                if let Err(e) = controller.touch_points(&points).await {
                    tracing::debug!("touch: {e}");
                }
            }
        }));

        // The helper watches the vibrator; an older one can't say what played.
        let (helper, adb, vibrations) = (session.clone(), session.adb(), out.clone());
        tasks.push(tokio::spawn(async move {
            if let Err(e) = helper.update_helper().await {
                tracing::warn!("couldn't update AAE's helper: {e}");
            }
            crate::vibration::watch(adb, vibrations).await;
        }));

        let attachment = Attachment {
            session,
            touches,
            tasks,
            out,
            microphone: None,
        };
        if attachment.session.speech_bridge() {
            attachment.speak_on_phone(true);
        }
        attachment
    }

    /// Has the device's screen reader speak on the phone, for the speech
    /// bridge, taking it over from a desktop app; or stops.
    pub fn speak_on_phone(&self, on: bool) {
        if on {
            self.session
                .start_speech_bridge_taking_over(Arc::new(PhoneSpeaker {
                    out: self.out.clone(),
                }));
        } else {
            self.session.stop_speech_bridge();
        }
    }

    /// Sound from the phone's microphone, as 16-bit little-endian samples:
    /// the first starts sending it into the device's.
    pub fn microphone(&mut self, bytes: &[u8]) {
        if self
            .microphone
            .as_ref()
            .is_none_or(|(_, task)| task.is_finished())
        {
            let (feed, task) = aae_core::microphone::feed(
                &self.session.controller(),
                &self.session.adb(),
                MICROPHONE_RATE,
            );
            // Says why, if the emulator won't take it, such as when the
            // computer's microphone is already going in.
            let out = self.out.clone();
            let task = tokio::spawn(async move {
                if let Ok(Err(e)) = task.await {
                    out.json(json!({
                        "type": "event",
                        "event": "microphone",
                        "on": false,
                        "message": format!("Microphone input to the device failed: {e}"),
                    }));
                }
            });
            self.microphone = Some((feed, task));
        }
        let samples = bytes
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect();
        if let Some((feed, _)) = &self.microphone {
            // Sound that can't keep up is dropped, not queued.
            let _ = feed.try_send(samples);
        }
    }

    /// Stops the phone's microphone stream.
    pub fn microphone_off(&mut self) {
        // Ending the stream ends the emulator's injection.
        self.microphone = None;
    }

    /// Touches from the phone: `[{"id":0,"x":…,"y":…,"down":true}]`.
    pub fn touch(&self, points: &Value) {
        let Some(points) = points.as_array() else {
            return;
        };
        let points = points
            .iter()
            .filter_map(|p| {
                Some(TouchPoint {
                    id: p["id"].as_i64()? as i32,
                    x: p["x"].as_i64()? as i32,
                    y: p["y"].as_i64()? as i32,
                    down: p["down"].as_bool().unwrap_or(false),
                })
            })
            .collect::<Vec<_>>();
        if !points.is_empty() {
            let _ = self.touches.send(points);
        }
    }

    pub fn stop(self) {
        // The device's own engine, or a desktop app's bridge, takes over.
        self.session.stop_speech_bridge();
        if let Some((_, task)) = self.microphone {
            task.abort();
        }
        for task in self.tasks {
            task.abort();
        }
    }
}
