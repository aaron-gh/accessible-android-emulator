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
use crate::platform::NoConsole;
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
        match setup::virtualisation(sdk) {
            setup::Virtualisation::Available => "available".to_string(),
            setup::Virtualisation::Missing(why) => format!("missing. {why}"),
            setup::Virtualisation::Unknown => "unknown".to_string(),
        }
    );
    if let Some(free) = crate::platform::free_space(&paths::data_dir()) {
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

/// How a self-test check came out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Passed,
    /// Works, but something isn't as it should be.
    Warning,
    Failed,
}

/// One self-test check.
#[derive(Debug, Clone)]
pub struct Check {
    pub name: String,
    pub outcome: Outcome,
    /// What was found, in words.
    pub detail: String,
}

impl Check {
    fn new(name: &str, outcome: Outcome, detail: impl Into<String>) -> Check {
        Check {
            name: name.to_string(),
            outcome,
            detail: detail.into(),
        }
    }
}

/// Checks what AAE needs: this computer, the SDK, AAE's own parts, and each
/// running device's screen reader and speech. Makes no sound. The Mac app
/// adds its own checks, such as keyboard capture.
pub async fn self_test(sdk: &Sdk, store: &DeviceStore) -> Vec<Check> {
    use Outcome::*;
    let mut checks = Vec::new();
    checks.push(match setup::virtualisation(sdk) {
        setup::Virtualisation::Available => Check::new(
            "Virtualisation",
            Passed,
            "This computer can run the emulator at full speed.",
        ),
        setup::Virtualisation::Missing(why) => Check::new("Virtualisation", Failed, why),
        setup::Virtualisation::Unknown => {
            Check::new("Virtualisation", Warning, "AAE can't tell on this system.")
        }
    });
    checks.push(match sdk.emulator_bin() {
        Ok(_) => Check::new(
            "Emulator",
            Passed,
            format!(
                "Version {}.",
                sdk.emulator_version().unwrap_or_else(|| "unknown".into())
            ),
        ),
        Err(e) => Check::new("Emulator", Failed, e.to_string()),
    });
    checks.push(match sdk.adb_bin() {
        Ok(_) => Check::new("adb", Passed, "Installed."),
        Err(e) => Check::new("adb", Failed, e.to_string()),
    });
    checks.push(
        if sdk.aapt2_bin().is_some() && sdk.apksigner_bin().is_some() {
            Check::new("Build tools", Passed, "Installed.")
        } else {
            Check::new(
                "Build tools",
                Failed,
                "Not installed. AAE needs them to read app packages. Run the setup again.",
            )
        },
    );
    checks.push(match crate::provision::helper_apk() {
        Some(_) => Check::new("AAE's helper app", Passed, "Found."),
        None => Check::new(
            "AAE's helper app",
            Failed,
            "Not found, so new devices can't be set up.",
        ),
    });
    checks.push(match crate::tts::espeak_apk() {
        Ok(_) => Check::new("AAE's eSpeak NG", Passed, "Found."),
        Err(_) => Check::new(
            "AAE's eSpeak NG",
            Warning,
            "Not found, so a device whose speech fails can't be given a working voice.",
        ),
    });
    checks.push(match crate::audio::output_name() {
        Some(name) => Check::new("Sound output", Passed, format!("Playing to {name}.")),
        None => Check::new("Sound output", Failed, "The Mac has no sound output."),
    });
    let images = sdk
        .system_images()
        .into_iter()
        .filter(|i| i.runs_natively())
        .count();
    checks.push(if images == 0 {
        Check::new("Android versions", Warning, "None installed yet.")
    } else {
        Check::new("Android versions", Passed, format!("{images} installed."))
    });
    if let Some(free) = crate::platform::free_space(&paths::data_dir()) {
        let gb = 1024 * 1024 * 1024;
        checks.push(if free < 10 * gb {
            Check::new(
                "Disk space",
                Warning,
                format!(
                    "Only {} free. Devices and Android versions need several gigabytes each.",
                    human_size(free)
                ),
            )
        } else {
            Check::new("Disk space", Passed, format!("{} free.", human_size(free)))
        });
    }
    for device in store.list().unwrap_or_default() {
        let Ok(info) = crate::emulator::running(&device) else {
            continue;
        };
        let name = device.meta.name.clone();
        let Ok(adb_bin) = sdk.adb_bin() else { continue };
        let adb = crate::adb::Adb::new(adb_bin, info.serial());
        if !adb.boot_completed().await {
            checks.push(Check::new(&name, Failed, "Android isn't answering."));
            continue;
        }
        let running = adb.running_services().await.unwrap_or_default();
        let is_on = |component: &str| {
            running
                .iter()
                .any(|r| crate::adb::same_component(r, component))
        };
        checks.push(match &device.meta.screen_reader {
            Some(reader) if is_on(reader) => Check::new(
                &format!("{name}: screen reader"),
                Passed,
                format!("{} is on.", reader.split('/').next().unwrap_or(reader)),
            ),
            Some(reader) => Check::new(
                &format!("{name}: screen reader"),
                Failed,
                format!(
                    "{} is off. Starting the device again turns it back on.",
                    reader.split('/').next().unwrap_or(reader)
                ),
            ),
            None => Check::new(
                &format!("{name}: screen reader"),
                Warning,
                "None is set up.",
            ),
        });
        checks.push(if is_on(crate::provision::HELPER_COMPONENT) {
            Check::new(&format!("{name}: AAE's helper"), Passed, "On.")
        } else {
            Check::new(
                &format!("{name}: AAE's helper"),
                Warning,
                "Off, so the volume, keyboard test and inspector won't work until the device starts again.",
            )
        });
        checks.push(match crate::tts::check(&adb).await {
            Ok(status) if status.ok => Check::new(
                &format!("{name}: speech"),
                Passed,
                format!("Speaking with {}.", status.engine),
            ),
            Ok(status) => Check::new(&format!("{name}: speech"), Failed, status.detail),
            Err(e) => Check::new(
                &format!("{name}: speech"),
                Warning,
                format!("Couldn't check: {e}"),
            ),
        });
    }
    checks
}

fn section(out: &mut String, title: &str) {
    let _ = writeln!(out, "\n== {title} ==");
}

/// The system version, model, processor and memory.
fn system_summary() -> String {
    let run = |cmd: &str, args: &[&str]| -> Option<String> {
        let out = std::process::Command::new(cmd)
            .no_console()
            .args(args)
            .output()
            .ok()?;
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
    } else if cfg!(windows) {
        // "Microsoft Windows [Version 10.0.26100.4652]"
        let version = run("cmd", &["/c", "ver"]).unwrap_or_else(|| "Windows".into());
        let memory = crate::platform::memory()
            .map(|m| format!(", {} of memory", human_size(m)))
            .unwrap_or_default();
        format!("{version}, {}{memory}", std::env::consts::ARCH)
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
        let home = home
            .to_string_lossy()
            .trim_end_matches(['/', '\\'])
            .to_string();
        if !home.is_empty() {
            text = text.replace(&home, "~");
            // Windows paths also turn up with forward slashes, and with
            // doubled backslashes in JSON.
            if home.contains('\\') {
                text = text.replace(&home.replace('\\', "/"), "~");
                text = text.replace(&home.replace('\\', "\\\\"), "~");
            }
        }
    }
    for (name, with) in personal_names() {
        if name.len() >= 3 {
            text = replace_ignoring_case(&text, &name, with);
        }
    }
    text
}

/// The computer's names and, on macOS, the person's full name.
fn personal_names() -> Vec<(String, &'static str)> {
    let run = |cmd: &str, args: &[&str]| -> Option<String> {
        let out = std::process::Command::new(cmd)
            .no_console()
            .args(args)
            .output()
            .ok()?;
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
    } else if cfg!(windows) {
        if let Ok(name) = std::env::var("COMPUTERNAME") {
            names.push((name, "<computer>"));
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
