//! The attached device's sound and touches, to and from one phone.

use std::sync::Arc;

use aae_core::control::TouchPoint;
use aae_ffi::Session;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::server::{AUDIO_FRAME, AUDIO_RATE, Out};

/// A device attached to a phone.
pub struct Attachment {
    pub session: Arc<Session>,
    touches: mpsc::UnboundedSender<Vec<TouchPoint>>,
    tasks: Vec<JoinHandle<()>>,
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
        let (helper, adb) = (session.clone(), session.adb());
        tasks.push(tokio::spawn(async move {
            if let Err(e) = helper.update_helper().await {
                tracing::warn!("couldn't update AAE's helper: {e}");
            }
            crate::vibration::watch(adb, out).await;
        }));

        Attachment {
            session,
            touches,
            tasks,
        }
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
        for task in self.tasks {
            task.abort();
        }
    }
}
