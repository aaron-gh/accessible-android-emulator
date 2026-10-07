//! A device's advanced hardware: memory, processor cores, storage, and the
//! screen's size and density, kept in the emulator's `config.ini`. Changed
//! only while the device is stopped; it takes effect when it next starts.

use crate::device::Device;
use crate::error::{Error, Result};

/// A device's hardware.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hardware {
    /// Memory, in megabytes.
    pub memory_mb: u32,
    /// Processor cores.
    pub cores: u32,
    /// Storage for apps and data, in megabytes.
    pub storage_mb: u32,
    /// The screen, in pixels, as held upright.
    pub width: u32,
    pub height: u32,
    /// Pixels per inch, which sets how big things are drawn.
    pub density: u32,
}

/// Limits, so a typing slip can't make a device that won't start.
pub const MEMORY_MB: (u32, u32) = (1024, 16384);
pub const CORES: (u32, u32) = (1, 16);
pub const STORAGE_MB: (u32, u32) = (2048, 131_072);
pub const SIDE: (u32, u32) = (320, 4096);
pub const DENSITY: (u32, u32) = (120, 640);

fn config(device: &Device) -> Result<String> {
    let path = device.dir.join("config.ini");
    std::fs::read_to_string(&path)
        .map_err(|e| Error::Message(format!("{} can't be read: {e}", path.display())))
}

fn value<'a>(config: &'a str, key: &str) -> Option<&'a str> {
    config.lines().find_map(|line| {
        let (k, v) = line.split_once('=')?;
        (k.trim() == key).then_some(v.trim())
    })
}

/// A size such as "6G", "2048M", "512MB" or "6442450944", in megabytes.
pub fn parse_size_mb(text: &str) -> Option<u32> {
    let text = text.trim().to_uppercase();
    let text = text.trim_end_matches('B');
    let (number, unit) = match text.chars().last()? {
        'G' => (&text[..text.len() - 1], 1024.0),
        'M' => (&text[..text.len() - 1], 1.0),
        'K' => (&text[..text.len() - 1], 1.0 / 1024.0),
        _ => (text, 1.0 / 1024.0 / 1024.0),
    };
    let mb = number.trim().parse::<f64>().ok()? * unit;
    (mb > 0.0).then_some(mb.round() as u32)
}

/// The device's hardware, from its `config.ini`.
pub fn read(device: &Device) -> Result<Hardware> {
    let config = config(device)?;
    let number = |key: &str, default: u32| {
        value(&config, key)
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    };
    Ok(Hardware {
        memory_mb: number("hw.ramSize", 2048),
        cores: device
            .meta
            .cores
            .unwrap_or_else(|| crate::setup::device_cores() as u32),
        storage_mb: value(&config, "disk.dataPartition.size")
            .and_then(parse_size_mb)
            .unwrap_or(6144),
        width: number("hw.lcd.width", 1080),
        height: number("hw.lcd.height", 2400),
        density: number("hw.lcd.density", 420),
    })
}

/// Checks hardware is within AAE's limits. Says what isn't.
pub fn check(hardware: &Hardware) -> Result<()> {
    let within = |name: &str, value: u32, (low, high): (u32, u32), unit: &str| {
        if (low..=high).contains(&value) {
            Ok(())
        } else {
            Err(Error::Message(format!(
                "{name} must be from {low} to {high}{unit}."
            )))
        }
    };
    within("Memory", hardware.memory_mb, MEMORY_MB, " megabytes")?;
    within("Processor cores", hardware.cores, CORES, "")?;
    within("Storage", hardware.storage_mb, STORAGE_MB, " megabytes")?;
    within("The screen's width", hardware.width, SIDE, " pixels")?;
    within("The screen's height", hardware.height, SIDE, " pixels")?;
    within(
        "The screen's density",
        hardware.density,
        DENSITY,
        " dots per inch",
    )?;
    Ok(())
}

/// Writes the device's hardware into its `config.ini`, keeping everything
/// else. The device must be stopped. Returns what to say, including that
/// smaller storage needs a wipe, which the emulator can't shrink otherwise.
pub fn write(device: &mut Device, hardware: &Hardware) -> Result<String> {
    if device.runtime().is_some() {
        return Err(Error::MustStop(device.meta.name.clone(), "changed"));
    }
    check(hardware)?;
    let before = read(device)?;
    let config = config(device)?;
    let changes = [
        ("hw.ramSize", hardware.memory_mb.to_string()),
        ("hw.cpu.ncore", hardware.cores.to_string()),
        (
            "disk.dataPartition.size",
            format!("{}M", hardware.storage_mb),
        ),
        ("hw.lcd.width", hardware.width.to_string()),
        ("hw.lcd.height", hardware.height.to_string()),
        ("hw.lcd.density", hardware.density.to_string()),
    ];
    let mut lines: Vec<String> = config.lines().map(str::to_string).collect();
    for (key, new) in &changes {
        match lines
            .iter_mut()
            .find(|l| l.split_once('=').is_some_and(|(k, _)| k.trim() == *key))
        {
            Some(line) => *line = format!("{key}={new}"),
            None => lines.push(format!("{key}={new}")),
        }
    }
    let path = device.dir.join("config.ini");
    std::fs::write(&path, lines.join("\n") + "\n")
        .map_err(|e| Error::Message(format!("{} can't be written: {e}", path.display())))?;
    // AAE starts devices with as many cores as suit the computer, unless
    // they were chosen.
    if hardware.cores != before.cores || device.meta.cores.is_some() {
        device.meta.cores = Some(hardware.cores);
        device.save_meta()?;
    }
    let mut said = format!(
        "{}: {} MB memory, {} cores, {} MB storage, {}x{} at {} dpi, from next start.",
        device.meta.name,
        hardware.memory_mb,
        hardware.cores,
        hardware.storage_mb,
        hardware.width,
        hardware.height,
        hardware.density
    );
    if hardware.storage_mb < before.storage_mb {
        said.push_str(" Reducing storage needs a wipe.");
    }
    if hardware != &before {
        said.push_str(" The next start is a cold boot.");
    }
    Ok(said)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_as_the_emulator_writes_them() {
        assert_eq!(parse_size_mb("6G"), Some(6144));
        assert_eq!(parse_size_mb("2048M"), Some(2048));
        assert_eq!(parse_size_mb("512MB"), Some(512));
        assert_eq!(parse_size_mb("6442450944"), Some(6144));
        assert_eq!(parse_size_mb("lots"), None);
    }

    #[test]
    fn hardware_outside_the_limits_is_refused() {
        let good = Hardware {
            memory_mb: 4096,
            cores: 4,
            storage_mb: 8192,
            width: 1080,
            height: 2400,
            density: 420,
        };
        assert!(check(&good).is_ok());
        assert!(
            check(&Hardware {
                memory_mb: 64,
                ..good.clone()
            })
            .is_err()
        );
        assert!(
            check(&Hardware {
                density: 2000,
                ..good
            })
            .is_err()
        );
    }
}
