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
use tokio::task::JoinHandle;

use crate::server::Out;

const WATCH: &str = "CLASSPATH=$(pm path io.github.aaron_gh.aae.helper | cut -d: -f2) \
     app_process / io.github.aaron_gh.aae.helper.ShellTool vibration-watch";

/// Sends `{"type":"event","event":"vibration","on":true}` when the device
/// starts vibrating, and `"on":false` with `"ms"`, how long it lasted, when
/// it stops. Ends when the task is aborted.
pub fn watch(adb: Adb, out: Out) -> JoinHandle<()> {
    tokio::spawn(async move {
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
            let mut parts = line.split_whitespace();
            let (Some(state), Some(time)) = (parts.next(), parts.next()) else {
                continue;
            };
            let Ok(time) = time.parse::<u64>() else {
                continue;
            };
            match state {
                "on" => {
                    started = Some(time);
                    out.json(json!({"type": "event", "event": "vibration", "on": true}));
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
    })
}
