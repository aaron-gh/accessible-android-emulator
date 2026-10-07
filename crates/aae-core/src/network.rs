//! The device's network: airplane mode, Wi-Fi and mobile data on or off,
//! and how fast and slow its connection is, for testing how apps behave
//! offline or on a poor connection.
//!
//! Airplane mode, Wi-Fi and data are Android's own settings, changed with its
//! shell commands. Speed and delay are the emulator's, through its console.

use crate::adb::Adb;
use crate::error::{Error, Result};

/// A connection speed, with the delay that goes with it, as the emulator's
/// console names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Speed {
    Full,
    Lte,
    Hsdpa,
    Umts,
    Edge,
    Gprs,
}

impl Speed {
    pub const ALL: [Speed; 6] = [
        Speed::Full,
        Speed::Lte,
        Speed::Hsdpa,
        Speed::Umts,
        Speed::Edge,
        Speed::Gprs,
    ];

    /// The name people use, and for `aae network`.
    pub fn name(self) -> &'static str {
        match self {
            Speed::Full => "full",
            Speed::Lte => "lte",
            Speed::Hsdpa => "3g",
            Speed::Umts => "slow-3g",
            Speed::Edge => "edge",
            Speed::Gprs => "gprs",
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Speed::Full => "full speed",
            Speed::Lte => "4G LTE",
            Speed::Hsdpa => "3G",
            Speed::Umts => "slow 3G",
            Speed::Edge => "EDGE, 2G",
            Speed::Gprs => "GPRS, the slowest",
        }
    }

    pub fn parse(name: &str) -> Option<Speed> {
        let name = name.trim().to_lowercase().replace([' ', '_'], "-");
        Speed::ALL
            .into_iter()
            .find(|s| s.name() == name)
            .or(match name.as_str() {
                "4g" | "4g-lte" => Some(Speed::Lte),
                "hsdpa" => Some(Speed::Hsdpa),
                "umts" => Some(Speed::Umts),
                "2g" => Some(Speed::Edge),
                "none" | "fast" => Some(Speed::Full),
                _ => None,
            })
    }

    /// The console's speed and delay presets.
    fn console(self) -> (&'static str, &'static str) {
        match self {
            Speed::Full => ("full", "none"),
            Speed::Lte => ("lte", "none"),
            Speed::Hsdpa => ("hsdpa", "umts"),
            Speed::Umts => ("umts", "umts"),
            Speed::Edge => ("edge", "edge"),
            Speed::Gprs => ("gprs", "gprs"),
        }
    }

    /// The speed whose download rate the console reports, in bits a second.
    fn from_rate(bits: u64) -> Option<Speed> {
        Some(match bits {
            0 => Speed::Full,
            173_000_000 => Speed::Lte,
            13_980_000 => Speed::Hsdpa,
            384_000 => Speed::Umts,
            473_600 => Speed::Edge,
            57_600 => Speed::Gprs,
            _ => return None,
        })
    }
}

/// How the device's network is.
#[derive(Debug, Clone)]
pub struct Status {
    pub airplane: bool,
    pub wifi: bool,
    pub data: bool,
    /// None when the console reports a speed AAE didn't set.
    pub speed: Option<Speed>,
}

impl Status {
    pub fn describe(&self) -> String {
        if self.airplane {
            return "Airplane mode is on: the device has no network.".into();
        }
        let on = |b: bool| if b { "on" } else { "off" };
        format!(
            "Wi-Fi is {}, mobile data is {}, and the connection is {}.",
            on(self.wifi),
            on(self.data),
            self.speed
                .map_or("at a speed set elsewhere", Speed::describe)
        )
    }
}

fn setting(value: Option<String>) -> bool {
    value.is_some_and(|v| v.trim() == "1")
}

pub async fn status(adb: &Adb) -> Result<Status> {
    let console = adb
        .raw(&["emu", "network", "status"])
        .await
        .unwrap_or_default();
    let rate = console
        .lines()
        .find(|l| l.contains("download speed"))
        .and_then(|l| l.split(':').nth(1))
        .and_then(|v| v.split_whitespace().next())
        .and_then(|v| v.parse().ok());
    Ok(Status {
        airplane: setting(adb.setting("global", "airplane_mode_on").await?),
        wifi: setting(adb.setting("global", "wifi_on").await?),
        data: setting(adb.setting("global", "mobile_data").await?),
        speed: rate.and_then(Speed::from_rate),
    })
}

pub async fn set_airplane(adb: &Adb, on: bool) -> Result<()> {
    let state = if on { "enable" } else { "disable" };
    let out = adb
        .shell(&format!("cmd connectivity airplane-mode {state}"))
        .await?;
    if out.contains("Unknown") || out.contains("No shell command") {
        // Before Android 11's command: the setting, then tell the system.
        adb.put_setting("global", "airplane_mode_on", if on { "1" } else { "0" })
            .await?;
        adb.shell(&format!(
            "am broadcast -a android.intent.action.AIRPLANE_MODE --ez state {on}"
        ))
        .await?;
    }
    Ok(())
}

pub async fn set_wifi(adb: &Adb, on: bool) -> Result<()> {
    adb.shell(&format!(
        "svc wifi {}",
        if on { "enable" } else { "disable" }
    ))
    .await?;
    Ok(())
}

pub async fn set_data(adb: &Adb, on: bool) -> Result<()> {
    adb.shell(&format!(
        "svc data {}",
        if on { "enable" } else { "disable" }
    ))
    .await?;
    Ok(())
}

pub async fn set_speed(adb: &Adb, speed: Speed) -> Result<()> {
    let (rate, delay) = speed.console();
    for args in [
        ["emu", "network", "speed", rate],
        ["emu", "network", "delay", delay],
    ] {
        let out = adb.raw(&args).await?;
        if out.contains("KO") {
            return Err(Error::Adb(format!(
                "The emulator wouldn't set the network to {}: {}",
                speed.describe(),
                out.trim()
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speeds_read_back_and_parse() {
        for speed in Speed::ALL {
            assert_eq!(Speed::parse(speed.name()), Some(speed));
        }
        assert_eq!(Speed::from_rate(473_600), Some(Speed::Edge));
        assert_eq!(Speed::parse("4G"), Some(Speed::Lte));
        assert_eq!(Speed::parse("warp"), None);
    }
}
