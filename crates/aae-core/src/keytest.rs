//! Checks that keys reach Android as the keys the user pressed.
//!
//! AAE's helper captures the keys Android delivers while the test runs, logs
//! each one, and swallows it so no app reacts. AAE presses each key in turn and
//! compares what Android received with what it should have.

use std::time::Duration;

use crate::adb::Adb;
use crate::control::Controller;
use crate::error::{Error, Result};
use crate::keys;

const HELPER_RECEIVER: &str = "io.github.aaron_gh.aae.helper/.CommandReceiver";
const KEY_TEST_ACTION: &str = "io.github.aaron_gh.aae.helper.KEY_TEST";

const META_SHIFT_ON: u32 = 0x1;
const META_ALT_ON: u32 = 0x2;
const META_CTRL_ON: u32 = 0x1000;
const META_META_ON: u32 = 0x10000;

/// One key to check.
struct Check {
    /// What is being checked, in words.
    name: &'static str,
    /// The key to press, as [`keys::parse`] reads it.
    press: &'static str,
    /// The Android key code it should arrive as.
    expect: &'static str,
    /// Modifier state Android should report with it.
    meta: u32,
}

const CHECKS: &[Check] = &[
    Check {
        name: "Meta, the screen reader's modifier",
        press: "meta+right",
        expect: "KEYCODE_DPAD_RIGHT",
        meta: META_META_ON,
    },
    Check {
        name: "Alt",
        press: "alt+right",
        expect: "KEYCODE_DPAD_RIGHT",
        meta: META_ALT_ON,
    },
    Check {
        name: "Control",
        press: "ctrl+right",
        expect: "KEYCODE_DPAD_RIGHT",
        meta: META_CTRL_ON,
    },
    Check {
        name: "Shift",
        press: "shift+tab",
        expect: "KEYCODE_TAB",
        meta: META_SHIFT_ON,
    },
    Check {
        name: "Escape",
        press: "escape",
        expect: "KEYCODE_ESCAPE",
        meta: 0,
    },
    Check {
        name: "Home",
        press: "home-key",
        expect: "KEYCODE_MOVE_HOME",
        meta: 0,
    },
    Check {
        name: "End",
        press: "end",
        expect: "KEYCODE_MOVE_END",
        meta: 0,
    },
    Check {
        name: "Page Up",
        press: "page-up",
        expect: "KEYCODE_PAGE_UP",
        meta: 0,
    },
    Check {
        name: "Page Down",
        press: "page-down",
        expect: "KEYCODE_PAGE_DOWN",
        meta: 0,
    },
    Check {
        name: "Insert",
        press: "insert",
        expect: "KEYCODE_INSERT",
        meta: 0,
    },
    Check {
        name: "Forward Delete",
        press: "delete",
        expect: "KEYCODE_FORWARD_DEL",
        meta: 0,
    },
    Check {
        name: "F5",
        press: "f5",
        expect: "KEYCODE_F5",
        meta: 0,
    },
    Check {
        name: "F12",
        press: "f12",
        expect: "KEYCODE_F12",
        meta: 0,
    },
    Check {
        name: "Backtick",
        press: "`",
        expect: "KEYCODE_GRAVE",
        meta: 0,
    },
    Check {
        name: "Letters",
        press: "a",
        expect: "KEYCODE_A",
        meta: 0,
    },
];

/// The result of checking one key.
#[derive(Debug, Clone)]
pub struct KeyResult {
    pub name: &'static str,
    pub passed: bool,
    /// What Android received, such as "KEYCODE_UNKNOWN", or "nothing".
    pub received: String,
}

/// Checks every key in turn. The helper must be installed and its service on.
pub async fn run(controller: &Controller, adb: &Adb) -> Result<Vec<KeyResult>> {
    set_capture(adb, true).await?;
    let results = check_all(controller, adb).await;
    // Always stop capturing, even if a check failed part way.
    let stopped = set_capture(adb, false).await;
    let results = results?;
    stopped?;
    Ok(results)
}

/// True when the helper received no keys at all, which happens after it is
/// updated while the device runs, until Android restarts from scratch.
pub fn received_nothing(results: &[KeyResult]) -> bool {
    results.iter().all(|r| r.received == "nothing")
}

async fn check_all(controller: &Controller, adb: &Adb) -> Result<Vec<KeyResult>> {
    let mut results = Vec::new();
    for (index, check) in CHECKS.iter().enumerate() {
        let key = keys::parse(check.press).expect("the keyboard test's keys all parse");
        // Each check reads only the log lines after a marker it writes, since
        // clearing the log sometimes fails on the emulator.
        let marker = format!("aae-keytest-{index}");
        adb.shell(&format!("log -t AaeKeys -p i {marker}")).await?;
        controller.press(&key).await?;
        tokio::time::sleep(Duration::from_millis(300)).await;
        let log = adb.shell("logcat -d -s AaeKeys:I").await?;
        let after = log.rsplit_once(&marker).map_or("", |(_, after)| after);
        let downs: Vec<(String, u32)> = after.lines().filter_map(parse_line).collect();
        let passed = downs
            .iter()
            .any(|(code, meta)| code == check.expect && meta & check.meta == check.meta);
        // Report the expected key if it arrived without the right modifiers,
        // otherwise whatever arrived last.
        let shown = downs
            .iter()
            .find(|(code, _)| code == check.expect)
            .or(downs.last());
        let received = match shown {
            Some((code, meta)) if *meta != 0 => format!("{code} with modifiers 0x{meta:x}"),
            Some((code, _)) => code.clone(),
            None => "nothing".to_string(),
        };
        results.push(KeyResult {
            name: check.name,
            passed,
            received,
        });
    }
    Ok(results)
}

/// Reads a helper log line, such as "... AaeKeys: KEYCODE_A down meta=0x0",
/// returning the key code and modifier state of key-down events.
fn parse_line(line: &str) -> Option<(String, u32)> {
    let rest = &line[line.find("KEYCODE_")?..];
    let mut words = rest.split_whitespace();
    let code = words.next()?.to_string();
    if words.next()? != "down" {
        return None;
    }
    let meta = u32::from_str_radix(words.next()?.strip_prefix("meta=0x")?, 16).ok()?;
    Some((code, meta))
}

/// Turns the helper's key capture on or off. Waits up to 10 seconds for the
/// helper's service, which Android may still be connecting after a start.
async fn set_capture(adb: &Adb, on: bool) -> Result<()> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let out = adb
            .shell(&format!(
                "am broadcast -n {HELPER_RECEIVER} -a {KEY_TEST_ACTION} --ez on {on}"
            ))
            .await?;
        if out.contains("result=1") {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    {
        Err(Error::Adb(
            "AAE's helper service is not running on this device, so the keyboard can't be tested. \
             Run aae volume, or start the device again, to turn it on."
                .into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_helper_log_lines() {
        let line = "10-06 16:08:39.708  4202  4202 I AaeKeys: KEYCODE_META_LEFT down meta=0x30000";
        assert_eq!(
            parse_line(line),
            Some(("KEYCODE_META_LEFT".into(), 0x30000))
        );
        assert_eq!(parse_line("10-06 I AaeKeys: KEYCODE_A up meta=0x0"), None);
        assert_eq!(parse_line("--------- beginning of main"), None);
    }

    #[test]
    fn every_check_parses() {
        for check in CHECKS {
            assert!(keys::parse(check.press).is_some(), "{}", check.press);
        }
    }
}
