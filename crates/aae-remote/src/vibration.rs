//! The attached device's vibrations, for the phone to play.
//!
//! AAE's helper watches the device's vibrator as the shell user (see its
//! VibrationWatch), printing "on" or "off" and the time each time it changes.
//! The phone vibrates while it's on.

use std::process::Stdio;

use aae_core::adb::Adb;
use serde_json::json;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use crate::server::Out;

const WATCH: &str = "CLASSPATH=$(pm path io.github.aaron_gh.aae.helper | cut -d: -f2) \
     app_process / io.github.aaron_gh.aae.helper.ShellTool vibration-watch";

/// Sends `{"type":"event","event":"vibration","on":true,"effect":…}` when
/// the device starts vibrating, with what it's playing as Android describes
/// it, such as its haptic primitives and their strengths, so the phone can
/// play the same; and `"on":false` with `"ms"`, how long it lasted, when it
/// stops. Ends when the task is aborted.
pub async fn watch(adb: Adb, out: Out) {
    {
        let child = Command::new(&adb.bin)
            .args(["-s", &adb.serial, "shell", WATCH])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn();
        let mut child = match child {
            Ok(child) => child,
            Err(e) => {
                tracing::warn!("couldn't watch the device's vibrator: {e}");
                return;
            }
        };
        let Some(stdout) = child.stdout.take() else {
            return;
        };
        let mut lines = BufReader::new(stdout).lines();
        let mut started: Option<u64> = None;
        while let Ok(Some(line)) = lines.next_line().await {
            // "on 6963878 Mono{mEffect=Composed{segments=[Primitive{…}]}}":
            // the state, the time, and what's playing, if known.
            let mut parts = line.splitn(3, ' ');
            let (Some(state), Some(time)) = (parts.next(), parts.next()) else {
                continue;
            };
            let effect = parts.next().unwrap_or("").trim().to_string();
            let Ok(time) = time.parse::<u64>() else {
                continue;
            };
            match state {
                "on" => {
                    started = Some(time);
                    out.json(json!({"type": "event", "event": "vibration", "on": true, "effect": effect}));
                }
                // The first "off" only says how it was when watching began.
                "off" => {
                    if let Some(start) = started.take() {
                        out.json(json!({
                            "type": "event",
                            "event": "vibration",
                            "on": false,
                            "ms": time.saturating_sub(start),
                        }));
                    }
                }
                _ => {}
            }
        }
        if let Ok(output) = child.wait_with_output().await
            && !output.stderr.is_empty()
        {
            tracing::warn!(
                "the device's vibrator can't be watched: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
    }
}
