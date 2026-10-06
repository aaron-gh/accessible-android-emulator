//! AAE's own log, and the diagnostic report people attach to bug reports.
//!
//! What's kept is about AAE, never about what people do on their devices:
//! the log records AAE's steps and errors at the info level, while keys,
//! typed text, clipboards, shell output and announcements are only ever
//! logged at the debug level, which the file never keeps. The device's own
//! log is left out of the report too, as apps can log what's typed into
//! them. And the report replaces the home folder with `~`, the computer's
//! names with `<computer>` and the person's full name with `<name>`.

use std::fmt::Write as _;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::device::{DeviceStore, human_size};
use crate::paths;
use crate::sdk::Sdk;
use crate::setup;

/// The log's size before it's moved aside to a second file, which replaces
/// the one before it, so the log never takes more than twice this.
const LOG_LIMIT: u64 = 2 * 1024 * 1024;

/// Where AAE's log is kept.
pub fn log_path() -> PathBuf {
    paths::data_dir().join("logs").join("aae.log")
}

/// The log file, for `tracing_subscriber`'s `with_writer`: each event is
/// appended, and the file moves aside when it's full.
pub struct LogFile(Option<File>);

impl LogFile {
    pub fn open() -> LogFile {
        let path = log_path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if std::fs::metadata(&path).is_ok_and(|m| m.len() > LOG_LIMIT) {
            let _ = std::fs::rename(&path, path.with_extension("1.log"));
        }
        LogFile(OpenOptions::new().create(true).append(true).open(path).ok())
    }
}

impl Write for LogFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match &mut self.0 {
            Some(file) => file.write(buf),
            // A log that can't be written mustn't stop AAE.
            None => Ok(buf.len()),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match &mut self.0 {
            Some(file) => file.flush(),
            None => Ok(()),
        }
    }
}

/// What goes in AAE's log file: its own crates at the info level, anything
/// else only for warnings. Debug, where keys and typed text are, never.
pub const LOG_FILTER: &str = "warn,aae=info,aae_core=info,aae_cli=info,aae_ffi=info";

/// A report on AAE, this computer and its devices, for attaching to a bug
/// report, as plain text with personal details taken out. `version` is the
/// version of the app or command making it.
pub fn report(sdk: &Sdk, store: &DeviceStore, version: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "AAE diagnostic report");
    let _ = writeln!(out, "AAE {version}");
    section(&mut out, "Computer");
    let _ = writeln!(out, "{}", system_summary());
    let _ = writeln!(
        out,
        "Virtualisation: {}",
        match setup::virtualisation() {
            setup::Virtualisation::Available => "available".to_string(),
            setup::Virtualisation::Missing(why) => format!("missing. {why}"),
            setup::Virtualisation::Unknown => "unknown".to_string(),
        }
    );
    if let Some(free) = crate::catalog::free_space(&paths::data_dir()) {
        let _ = writeln!(out, "Free disk space: {}", human_size(free));
    }

    section(&mut out, "Android SDK");
    let _ = writeln!(
        out,
        "Folder: {}{}",
        sdk.root.display(),
        if setup::is_own_sdk(&sdk.root) {
            ", AAE's own"
        } else {
            ""
        }
    );
    let _ = writeln!(
        out,
        "Emulator: {}",
        match sdk.emulator_bin() {
            Ok(_) => sdk
                .emulator_version()
                .unwrap_or_else(|| "unknown version".into()),
            Err(_) => "not installed".into(),
        }
    );
    let _ = writeln!(
        out,
        "adb: {}",
        if sdk.adb_bin().is_ok() {
            "installed"
        } else {
            "not installed"
        }
    );
    let _ = writeln!(
        out,
        "Build tools: {}",
        sdk.aapt2_bin()
            .map_or("not installed".to_string(), |p| p.display().to_string())
    );
    let _ = writeln!(out, "Android versions:");
    let images = sdk.system_images();
    if images.is_empty() {
        let _ = writeln!(out, "  none");
    }
    for image in images {
        let _ = writeln!(out, "  {image} ({})", image.sysdir);
    }

    section(&mut out, "Devices");
    match store.list() {
        Ok(devices) if devices.is_empty() => {
            let _ = writeln!(out, "none");
        }
        Ok(devices) => {
            for device in devices {
                let m = &device.meta;
                let running = crate::emulator::running(&device).is_ok();
                let _ = writeln!(out, "{}", device.describe());
                let _ = writeln!(
                    out,
                    "  {}, set up: {}, screen reader: {}{}",
                    if running { "running" } else { "stopped" },
                    if m.provisioned { "yes" } else { "no" },
                    m.screen_reader.as_deref().unwrap_or("none"),
                    if m.screen_reader_declined {
                        " (declined)"
                    } else {
                        ""
                    }
                );
                let _ = writeln!(
                    out,
                    "  Kept on: {}",
                    if m.keep_enabled.is_empty() {
                        "nothing".to_string()
                    } else {
                        m.keep_enabled.join(", ")
                    }
                );
                if let Some(speed) = m.audio_speed {
                    let _ = writeln!(out, "  Audio speed: {speed}");
                }
                if let Some(engine) = &m.speech_log_engine {
                    let _ = writeln!(out, "  Speech log on, relaying to {engine}");
                }
                if m.pending_screen_reader.is_some() {
                    let _ = writeln!(out, "  A screen reader build is queued for its next start.");
                }
                let log = tail(&device.dir.join("emulator.log"), 80);
                if !log.is_empty() {
                    let _ = writeln!(out, "  Last lines of its emulator log:");
                    for line in log.lines() {
                        let _ = writeln!(out, "    {line}");
                    }
                }
            }
        }
        Err(e) => {
            let _ = writeln!(out, "couldn't be read: {e}");
        }
    }

    section(&mut out, "AAE's log");
    let older = tail(&log_path().with_extension("1.log"), 400);
    let newer = tail(&log_path(), 1500);
    if older.is_empty() && newer.is_empty() {
        let _ = writeln!(out, "empty");
    }
    out.push_str(&older);
    out.push_str(&newer);
    redact(&out)
}

fn section(out: &mut String, title: &str) {
    let _ = writeln!(out, "\n== {title} ==");
}

/// The macOS version, model, processor and memory.
fn system_summary() -> String {
    let run = |cmd: &str, args: &[&str]| -> Option<String> {
        let out = std::process::Command::new(cmd).args(args).output().ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    };
    if cfg!(target_os = "macos") {
        let version = run("sw_vers", &["-productVersion"]).unwrap_or_default();
        let model = run("sysctl", &["-n", "hw.model"]).unwrap_or_default();
        let memory = run("sysctl", &["-n", "hw.memsize"])
            .and_then(|m| m.parse::<u64>().ok())
            .map(human_size)
            .unwrap_or_default();
        format!(
            "macOS {version}, {model}, {}, {memory} of memory",
            std::env::consts::ARCH
        )
    } else {
        format!("{}, {}", std::env::consts::OS, std::env::consts::ARCH)
    }
}

/// The last lines of a text file, or nothing.
fn tail(path: &Path, lines: usize) -> String {
    let Ok(text) = std::fs::read(path) else {
        return String::new();
    };
    let text = String::from_utf8_lossy(&text);
    let all: Vec<&str> = text.lines().collect();
    let mut out = all[all.len().saturating_sub(lines)..].join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

/// Takes personal details out: the home folder, which holds the account
/// name, becomes `~`; the computer's names become `<computer>`; and the
/// person's full name becomes `<name>`. Package names and the like are left
/// alone, even when they contain the account name.
pub fn redact(text: &str) -> String {
    let mut text = text.to_string();
    if let Some(home) = directories::BaseDirs::new().map(|d| d.home_dir().to_path_buf()) {
        let home = home.to_string_lossy().trim_end_matches('/').to_string();
        if !home.is_empty() {
            text = text.replace(&home, "~");
        }
    }
    for (name, with) in personal_names() {
        if name.len() >= 3 {
            text = replace_ignoring_case(&text, &name, with);
        }
    }
    text
}

/// The computer's names and the person's full name, as macOS has them.
fn personal_names() -> Vec<(String, &'static str)> {
    let run = |cmd: &str, args: &[&str]| -> Option<String> {
        let out = std::process::Command::new(cmd).args(args).output().ok()?;
        let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
        (out.status.success() && !text.is_empty()).then_some(text)
    };
    let mut names = Vec::new();
    if cfg!(target_os = "macos") {
        for key in ["ComputerName", "LocalHostName", "HostName"] {
            if let Some(name) = run("scutil", &["--get", key]) {
                names.push((name, "<computer>"));
            }
        }
        if let Some(name) = run("id", &["-F"]) {
            names.push((name, "<name>"));
        }
    }
    // Longest first, so "Aaron's MacBook Air" goes before "Aaron".
    names.sort_by_key(|(n, _)| std::cmp::Reverse(n.len()));
    names
}

fn replace_ignoring_case(text: &str, find: &str, with: &str) -> String {
    let lower = text.to_lowercase();
    let find = find.to_lowercase();
    // Lowercasing can change byte lengths outside ASCII; then don't risk it.
    if lower.len() != text.len() {
        return text.replace(&find, with);
    }
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (start, _) in lower.match_indices(&find) {
        out.push_str(&text[last..start]);
        out.push_str(with);
        last = start + find.len();
    }
    out.push_str(&text[last..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_names_in_any_case() {
        assert_eq!(
            replace_ignoring_case("Aarons-MacBook and /x/aaron/y", "aaron", "<user>"),
            "<user>s-MacBook and /x/<user>/y"
        );
    }

    #[test]
    fn keeps_the_end_of_a_log() {
        let path = std::env::temp_dir().join(format!("aae-test-tail-{}", std::process::id()));
        std::fs::write(&path, "one\ntwo\nthree\n").unwrap();
        assert_eq!(tail(&path, 2), "two\nthree\n");
        let _ = std::fs::remove_file(&path);
    }
}
