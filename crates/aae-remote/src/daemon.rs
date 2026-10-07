//! Serving at login, without AAE open: `aae daemon install` sets the
//! system to run `aae serve` whenever this person logs in, and starts it now.
//!
//! - macOS: a LaunchAgent, which launchd starts at login and again if it
//!   stops unexpectedly.
//! - Windows: a Run entry in the registry. It runs the Windows app with
//!   --serve, which runs aae serve with no console window: aae.exe itself
//!   would flash one at every login.
//! - Linux: a systemd user service.
//!
//! The running server writes its process id to `serve.pid` in AAE's data
//! folder, which says whether it's running.

use std::path::PathBuf;
use std::process::Command;

use crate::security::folder;

const LABEL: &str = "io.github.aaron-gh.aae.serve";
#[cfg(windows)]
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
#[cfg(windows)]
const RUN_VALUE: &str = "AAE Serve";

pub struct Status {
    /// Set to start at login.
    pub installed: bool,
    /// A server is running now, at login or not.
    pub running: bool,
}

fn pid_path() -> PathBuf {
    folder().join("serve.pid")
}

/// Notes that this process is serving.
pub fn write_pid() {
    let _ = std::fs::create_dir_all(folder());
    let _ = std::fs::write(pid_path(), std::process::id().to_string());
}

pub fn remove_pid() {
    if running_pid() == Some(std::process::id()) {
        let _ = std::fs::remove_file(pid_path());
    }
}

/// The running server's process id, if one is running.
pub fn running_pid() -> Option<u32> {
    let pid: u32 = std::fs::read_to_string(pid_path())
        .ok()?
        .trim()
        .parse()
        .ok()?;
    aae_core::emulator::is_alive(pid).then_some(pid)
}

fn this_aae() -> std::io::Result<PathBuf> {
    std::env::current_exe()?.canonicalize()
}

fn run(program: &str, args: &[&str]) -> std::io::Result<std::process::Output> {
    let mut command = Command::new(program);
    command.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    command.output()
}

#[cfg(target_os = "macos")]
fn plist_path() -> PathBuf {
    directories::BaseDirs::new()
        .map(|d| d.home_dir().to_path_buf())
        .unwrap_or_default()
        .join("Library/LaunchAgents")
        .join(format!("{LABEL}.plist"))
}

#[cfg(target_os = "macos")]
fn domain() -> String {
    format!("gui/{}", unsafe { libc::getuid() })
}

#[cfg(target_os = "linux")]
fn unit_path() -> PathBuf {
    directories::BaseDirs::new()
        .map(|d| d.config_dir().to_path_buf())
        .unwrap_or_default()
        .join("systemd/user/aae-serve.service")
}

/// Whether the server is set to start at login, and whether it's running.
pub fn status() -> Status {
    #[cfg(target_os = "macos")]
    let installed = plist_path().is_file();
    #[cfg(windows)]
    let installed =
        run("reg", &["query", RUN_KEY, "/v", RUN_VALUE]).is_ok_and(|o| o.status.success());
    #[cfg(target_os = "linux")]
    let installed = unit_path().is_file();
    #[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
    let installed = false;
    Status {
        installed,
        running: running_pid().is_some(),
    }
}

#[cfg(target_os = "macos")]
fn escape_xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Sets `aae serve` to start at login, and starts it now if it isn't running.
pub fn install() -> Result<String, String> {
    let aae = this_aae().map_err(|e| e.to_string())?;
    let log = aae_core::paths::data_dir().join("logs").join("serve.log");
    let _ = std::fs::create_dir_all(log.parent().unwrap());
    #[cfg(target_os = "macos")]
    {
        let plist = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{}</string>
        <string>serve</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>
    <key>ProcessType</key>
    <string>Background</string>
    <key>StandardOutPath</key>
    <string>{}</string>
    <key>StandardErrorPath</key>
    <string>{}</string>
</dict>
</plist>
"#,
            escape_xml(&aae.to_string_lossy()),
            escape_xml(&log.to_string_lossy()),
            escape_xml(&log.to_string_lossy()),
        );
        let path = plist_path();
        std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
        // Replaces one set up before, as by an older AAE elsewhere.
        let _ = run("launchctl", &["bootout", &format!("{}/{LABEL}", domain())]);
        std::fs::write(&path, plist).map_err(|e| e.to_string())?;
        if running_pid().is_none() {
            let out = run(
                "launchctl",
                &["bootstrap", &domain(), &path.to_string_lossy()],
            )
            .map_err(|e| e.to_string())?;
            if !out.status.success() {
                return Err(format!(
                    "launchctl couldn't start it: {}",
                    String::from_utf8_lossy(&out.stderr).trim()
                ));
            }
        }
        Ok(
            "AAE now serves this Mac's devices to phones whenever you log in, and is serving now."
                .into(),
        )
    }
    #[cfg(windows)]
    {
        let app = aae
            .parent()
            .map(|d| d.join("AccessibleAndroidEmulator.exe"))
            .filter(|p| p.is_file())
            .ok_or("Serving at login needs the Windows app, AccessibleAndroidEmulator.exe, next to aae.exe.")?;
        let command = format!("\"{}\" --serve", app.display());
        let out = run(
            "reg",
            &[
                "add", RUN_KEY, "/v", RUN_VALUE, "/t", "REG_SZ", "/d", &command, "/f",
            ],
        )
        .map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
        }
        if running_pid().is_none() {
            Command::new(&app)
                .arg("--serve")
                .spawn()
                .map_err(|e| e.to_string())?;
        }
        let _ = log;
        Ok(
            "AAE now serves this PC's devices to phones whenever you log in, and is serving now."
                .into(),
        )
    }
    #[cfg(target_os = "linux")]
    {
        let unit = format!(
            "[Unit]\nDescription=AAE serving devices to AAE Remote\nAfter=network-online.target\n\n\
             [Service]\nExecStart=\"{}\" serve\nRestart=on-failure\nStandardInput=null\n\
             StandardOutput=append:{}\nStandardError=append:{}\n\n[Install]\nWantedBy=default.target\n",
            aae.display(),
            log.display(),
            log.display(),
        );
        let path = unit_path();
        std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(&path, unit).map_err(|e| e.to_string())?;
        let _ = run("systemctl", &["--user", "daemon-reload"]);
        let out = run(
            "systemctl",
            &["--user", "enable", "--now", "aae-serve.service"],
        )
        .map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
        }
        Ok("AAE now serves this computer's devices to phones whenever you log in, and is serving now.".into())
    }
    #[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
    {
        let _ = (aae, log);
        Err("Serving at login isn't available on this system.".into())
    }
}

/// Stops serving at login, and stops the server running now.
pub fn uninstall() -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        let _ = run("launchctl", &["bootout", &format!("{}/{LABEL}", domain())]);
        let _ = std::fs::remove_file(plist_path());
    }
    #[cfg(windows)]
    {
        let _ = run("reg", &["delete", RUN_KEY, "/v", RUN_VALUE, "/f"]);
    }
    #[cfg(target_os = "linux")]
    {
        let _ = run(
            "systemctl",
            &["--user", "disable", "--now", "aae-serve.service"],
        );
        let _ = std::fs::remove_file(unit_path());
        let _ = run("systemctl", &["--user", "daemon-reload"]);
    }
    // One started some other way stops too.
    if let Some(pid) = running_pid() {
        #[cfg(unix)]
        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGTERM);
        }
        #[cfg(windows)]
        {
            let _ = run("taskkill", &["/PID", &pid.to_string(), "/T", "/F"]);
        }
    }
    let _ = LABEL;
    Ok("AAE no longer serves devices at login, and has stopped serving.".into())
}
