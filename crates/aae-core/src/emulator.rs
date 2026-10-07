//! Starting, watching and stopping emulator processes.
//!
//! AAE runs Google's emulator with no window and talks to it over gRPC and adb.
//! The emulator outlives the process that started it, so the command line can
//! start a device and the Mac app can use it, or the other way round. What a
//! running device needs to be found again (its process and ports) is kept in the
//! device's `running.toml`.

use std::net::TcpListener;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::adb::Adb;
use crate::control::Controller;
use crate::device::{Device, DeviceStore, RuntimeInfo};
use crate::error::{Error, IoContext, Result};
use crate::sdk::Sdk;

const FIRST_CONSOLE_PORT: u16 = 5554;
const LAST_CONSOLE_PORT: u16 = 5682;
const FIRST_GRPC_PORT: u16 = 8554;

/// Options for starting a device.
#[derive(Debug, Clone, Default)]
pub struct StartOptions {
    /// Boot from scratch instead of from the quick-boot snapshot.
    pub cold_boot: bool,
    /// Don't save the quick-boot snapshot on exit.
    pub no_snapshot_save: bool,
    /// Let the emulator play audio itself as well. AAE plays the device's audio
    /// from the gRPC stream, so this is off to avoid hearing everything twice.
    pub emulator_audio: bool,
    /// Extra arguments for the emulator, for troubleshooting.
    pub extra_args: Vec<String>,
}

/// Starts a device's emulator in the background and records its ports. Returns
/// as soon as the process is running; use [`wait_until_ready`] to wait for Android.
pub fn start(
    sdk: &Sdk,
    store: &DeviceStore,
    device: &Device,
    options: &StartOptions,
) -> Result<RuntimeInfo> {
    if let Some(info) = device.runtime() {
        if is_alive(info.pid) {
            return Err(Error::AlreadyRunning(device.meta.name.clone()));
        }
        device.set_runtime(None)?;
    }
    store.repair_pointers()?;

    let used: Vec<RuntimeInfo> = store.list()?.iter().filter_map(Device::runtime).collect();
    let console_port = free_console_port(&used)?;
    let grpc_port = free_grpc_port(&used)?;

    let log = device.dir.join("emulator.log");
    let log_file = std::fs::File::create(&log).context(|| format!("Creating {}", log.display()))?;
    let log_err = log_file
        .try_clone()
        .context(|| format!("Opening {}", log.display()))?;

    let mut command = Command::new(sdk.emulator_bin()?);
    command
        .arg("-avd")
        .arg(&device.id)
        .arg("-no-window")
        .arg("-no-boot-anim")
        .arg("-ports")
        .arg(format!("{},{}", console_port, console_port + 1))
        .arg("-grpc")
        .arg(grpc_port.to_string())
        // Without a token the gRPC port accepts commands from anyone on the network.
        .arg("-grpc-use-token")
        // Fewer cores on a small processor, so the computer isn't starved,
        // unless the user chose how many, up to what the computer has.
        .arg("-cores")
        .arg(
            device
                .meta
                .cores
                .map_or(crate::setup::device_cores(), |c| {
                    (c as usize).clamp(1, crate::setup::threads().max(1))
                })
                .to_string(),
        )
        .env("ANDROID_AVD_HOME", &store.root)
        .env("ANDROID_SDK_ROOT", &sdk.root)
        .env("ANDROID_HOME", &sdk.root)
        .stdin(Stdio::null())
        .stdout(log_file)
        .stderr(log_err);
    // Without a window the emulator picks software graphics by itself, which
    // makes Android slow even on a fast computer, so AAE asks for the
    // graphics adapter, unless it failed for this device before (see
    // lifecycle::start_device). AAE_GPU sets another mode, for troubleshooting.
    match std::env::var("AAE_GPU").ok().filter(|g| !g.is_empty()) {
        Some(mode) => {
            command.arg("-gpu").arg(mode);
        }
        None if !device.meta.software_graphics => {
            command.arg("-gpu").arg("host");
        }
        None => {}
    }
    if options.cold_boot {
        command.arg("-no-snapshot-load");
        // Otherwise a start after an emulator crash, which saves nothing,
        // would restore the old state again.
        let quick_boot = device.dir.join("snapshots").join("default_boot");
        if quick_boot.exists() {
            std::fs::remove_dir_all(&quick_boot)
                .context(|| format!("Deleting {}", quick_boot.display()))?;
        }
    }
    if options.no_snapshot_save {
        command.arg("-no-snapshot-save");
    }
    if !options.emulator_audio {
        // AAE plays the device's sound from the gRPC stream, and sends sound
        // into its microphone the same way, so the emulator uses neither the
        // computer's speakers nor its microphone. "-audio none" keeps the
        // device's sound smooth, but removes its microphone, so what AAE
        // sends into it comes out as silence; the input's "none" driver puts
        // it back. ("none" for the output too, rather than "-audio none",
        // makes the device's sound crackle.)
        command.arg("-audio").arg("none");
        command.env("QEMU_AUDIO_IN_DRV", "none");
    }
    command.args(&options.extra_args);
    detach(&mut command);

    let mut child = command
        .spawn()
        .context(|| "Starting the emulator".to_string())?;
    let info = RuntimeInfo {
        pid: child.id(),
        console_port,
        adb_port: console_port + 1,
        grpc_port,
        log,
    };
    device.set_runtime(Some(&info))?;
    // Reap the process when it exits, so it doesn't linger as a zombie while
    // this process keeps running.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(info)
}

/// How long the emulator may take to answer on gRPC before AAE decides it hung.
const EMULATOR_ANSWER: Duration = Duration::from_secs(60);

/// Whether the emulator's log says its graphics couldn't start, which it
/// doesn't always exit after promptly.
fn graphics_failed(log: &Path) -> bool {
    std::fs::read_to_string(log).is_ok_and(|text| text.contains("OpenGL Core Profile not supported"))
}

/// Progress while a device starts, for announcing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootStage {
    WaitingForEmulator,
    WaitingForAndroid,
    Ready,
}

/// Waits until the emulator answers on gRPC and Android has finished booting.
/// Returns [`Error::BatteryEmpty`] if Android isn't running and the battery is
/// at 0% and not charging, as when a snapshot was saved after Android shut
/// down for that.
pub async fn wait_until_ready(
    sdk: &Sdk,
    info: &RuntimeInfo,
    timeout: Duration,
    mut progress: impl FnMut(BootStage),
) -> Result<(Controller, Adb)> {
    let started = tokio::time::Instant::now();
    let deadline = started + timeout;
    let adb = Adb::new(sdk.adb_bin()?, info.serial());
    let mut stage = BootStage::WaitingForEmulator;
    progress(stage);
    let mut controller = None;

    loop {
        if !is_alive(info.pid) {
            return Err(Error::EmulatorExited(crate::diagnostics::redact(
                &log_tail(&info.log, 15),
            )));
        }
        if tokio::time::Instant::now() > deadline {
            return Err(Error::BootTimeout(timeout.as_secs(), info.log.clone()));
        }
        // With a graphics driver it can't use, the emulator can take minutes
        // to exit, or hang. It answers on gRPC within seconds otherwise.
        if stage == BootStage::WaitingForEmulator {
            let failed = graphics_failed(&info.log);
            if failed || started.elapsed() > EMULATOR_ANSWER {
                terminate(info.pid);
                for _ in 0..40 {
                    if !is_alive(info.pid) {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
                let tail = log_tail(&info.log, 15);
                let why = if failed {
                    tail
                } else {
                    format!(
                        "The emulator didn't answer within {} seconds.\n{tail}",
                        EMULATOR_ANSWER.as_secs()
                    )
                };
                return Err(Error::EmulatorExited(crate::diagnostics::redact(&why)));
            }
        }
        if controller.is_none() {
            // The discovery file, and so the token, appears shortly after the process starts.
            if let Some(token) = grpc_token(info) {
                controller = Controller::connect(info.grpc_port, Some(&token)).await.ok();
            }
        }
        if let Some(c) = &controller {
            if stage == BootStage::WaitingForEmulator && c.status().await.is_ok() {
                stage = BootStage::WaitingForAndroid;
                progress(stage);
            }
            if stage == BootStage::WaitingForAndroid {
                if adb.boot_completed().await {
                    progress(BootStage::Ready);
                    return Ok((controller.unwrap(), adb));
                }
                if c.battery_empty().await.unwrap_or(false) {
                    return Err(Error::BatteryEmpty);
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// Connects to a device that is already running.
pub async fn attach(sdk: &Sdk, device: &Device) -> Result<(RuntimeInfo, Controller, Adb)> {
    let info = running(device)?;
    let controller = Controller::connect(info.grpc_port, grpc_token(&info).as_deref()).await?;
    let adb = Adb::new(sdk.adb_bin()?, info.serial());
    Ok((info, controller, adb))
}

/// Restarts Android inside a running device and waits until it is back. Unlike
/// stopping and cold booting, this keeps everything on the device's disk.
pub async fn reboot(sdk: &Sdk, device: &Device, timeout: Duration) -> Result<()> {
    let info = running(device)?;
    let adb = Adb::new(sdk.adb_bin()?, info.serial());
    let _ = adb.shell("sync").await;
    // The connection drops as Android goes down, so the result doesn't matter.
    let _ = adb.raw(&["reboot"]).await;
    tokio::time::sleep(Duration::from_secs(5)).await;
    if adb.wait_for_boot(timeout).await {
        Ok(())
    } else {
        Err(Error::BootTimeout(timeout.as_secs(), info.log.clone()))
    }
}

/// The device's runtime record, if its emulator is still alive. Clears a stale record.
pub fn running(device: &Device) -> Result<RuntimeInfo> {
    match device.runtime() {
        Some(info) if is_alive(info.pid) => Ok(info),
        Some(_) => {
            device.set_runtime(None)?;
            Err(Error::NotRunning(device.meta.name.clone()))
        }
        None => Err(Error::NotRunning(device.meta.name.clone())),
    }
}

/// Stops a device, saving its quick-boot snapshot. Forces it to stop if it
/// hasn't exited after `timeout`.
pub async fn stop(sdk: &Sdk, device: &Device, timeout: Duration) -> Result<()> {
    let info = running(device)?;
    let adb = Adb::new(sdk.adb_bin()?, info.serial());
    // Flush Android's disk writes first. "emu kill" saves memory in the
    // quick-boot snapshot, and writes still in memory would be missing from
    // the disk if the device were later cold booted: installed apps could be
    // left half there.
    let _ = tokio::time::timeout(Duration::from_secs(15), adb.shell("sync")).await;
    // "emu kill" asks the emulator to save its state and exit.
    let _ = tokio::time::timeout(Duration::from_secs(10), adb.raw(&["emu", "kill"])).await;
    let deadline = tokio::time::Instant::now() + timeout;
    while is_alive(info.pid) && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    if is_alive(info.pid) {
        terminate(info.pid);
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    device.set_runtime(None)
}

/// Ends a device's emulator without waiting for Android, for a device whose
/// Android isn't running. Start it next with `cold_boot`, as any state the
/// emulator saves on the way out is of that Android.
pub async fn force_stop(device: &Device, info: &RuntimeInfo) -> Result<()> {
    terminate(info.pid);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while is_alive(info.pid) && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    #[cfg(unix)]
    if is_alive(info.pid) {
        unsafe {
            libc::kill(info.pid as libc::pid_t, libc::SIGKILL);
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    device.set_runtime(None)
}

/// The token the emulator expects on gRPC calls. With `-grpc-use-token` each
/// run gets its own token, which the emulator writes to its discovery file.
pub fn grpc_token(info: &RuntimeInfo) -> Option<String> {
    find_grpc_token(&discovery_dirs(), info)
}

/// The token from this emulator's discovery file, `pid_<pid>.ini`. On the
/// Mac and Linux that's the process AAE started. On Windows, emulator.exe
/// only launches the emulator, which runs as another process and names the
/// file after itself, so otherwise it's the file with this emulator's ports.
fn find_grpc_token(dirs: &[std::path::PathBuf], info: &RuntimeInfo) -> Option<String> {
    let own = format!("pid_{}.ini", info.pid);
    if let Some(token) = dirs.iter().find_map(|dir| {
        crate::sdk::read_properties(&dir.join(&own))
            .ok()?
            .remove("grpc.token")
    }) {
        return Some(token);
    }
    let (grpc, console) = (info.grpc_port.to_string(), info.console_port.to_string());
    dirs.iter()
        .filter_map(|dir| std::fs::read_dir(dir).ok())
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("pid_") && n.ends_with(".ini"))
        })
        .find_map(|path| {
            let mut properties = crate::sdk::read_properties(&path).ok()?;
            let ours = properties.get("grpc.port") == Some(&grpc)
                && properties.get("port.serial").is_none_or(|p| *p == console);
            if ours {
                properties.remove("grpc.token")
            } else {
                None
            }
        })
}

/// Where the emulator writes a `pid_<pid>.ini` file describing each running instance.
fn discovery_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs = Vec::new();
    if let Some(base) = directories::BaseDirs::new() {
        if cfg!(target_os = "macos") {
            dirs.push(
                base.home_dir()
                    .join("Library/Caches/TemporaryItems/avd/running"),
            );
        } else if cfg!(target_os = "windows") {
            dirs.push(base.data_local_dir().join("Temp/avd/running"));
        } else {
            if let Some(runtime) = base.runtime_dir() {
                dirs.push(runtime.join("avd/running"));
            }
            dirs.push(std::env::temp_dir().join(format!("android-{}/avd/running", whoami())));
        }
    }
    dirs.push(std::env::temp_dir().join("avd/running"));
    dirs
}

#[cfg(unix)]
fn whoami() -> String {
    std::env::var("USER").unwrap_or_default()
}

#[cfg(not(unix))]
fn whoami() -> String {
    std::env::var("USERNAME").unwrap_or_default()
}

/// The last lines of a log file.
pub fn log_tail(path: &Path, lines: usize) -> String {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

fn free_console_port(used: &[RuntimeInfo]) -> Result<u16> {
    (FIRST_CONSOLE_PORT..=LAST_CONSOLE_PORT)
        .step_by(2)
        .find(|&port| {
            !used.iter().any(|r| r.console_port == port)
                && port_is_free(port)
                && port_is_free(port + 1)
        })
        .ok_or(Error::NoFreePorts)
}

fn free_grpc_port(used: &[RuntimeInfo]) -> Result<u16> {
    (FIRST_GRPC_PORT..FIRST_GRPC_PORT + 200)
        .find(|&port| !used.iter().any(|r| r.grpc_port == port) && port_is_free(port))
        .ok_or(Error::NoFreePorts)
}

fn port_is_free(port: u16) -> bool {
    TcpListener::bind(("127.0.0.1", port)).is_ok() && TcpListener::bind(("0.0.0.0", port)).is_ok()
}

/// Runs the emulator in its own process group, so a Control-C in the terminal
/// that started it doesn't stop it.
fn detach(command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
    }
}

/// True if a process with this id is running.
pub fn is_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // Signal 0 checks the process exists without sending anything.
        let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
        result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
        use windows_sys::Win32::System::Threading::{
            GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                return false;
            }
            let mut code = 0u32;
            let ok = GetExitCodeProcess(handle, &mut code) != 0;
            CloseHandle(handle);
            ok && code == STILL_ACTIVE as u32
        }
    }
}

fn terminate(pid: u32) {
    #[cfg(unix)]
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGTERM);
    }
    // On Windows, emulator.exe only launches the emulator, which runs as a
    // process of its own, so the whole tree is ended, or it would carry on.
    #[cfg(windows)]
    {
        use crate::platform::NoConsole;
        let ended = Command::new("taskkill")
            .no_console()
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if !ended {
            use windows_sys::Win32::Foundation::CloseHandle;
            use windows_sys::Win32::System::Threading::{
                OpenProcess, PROCESS_TERMINATE, TerminateProcess,
            };
            unsafe {
                let handle = OpenProcess(PROCESS_TERMINATE, 0, pid);
                if !handle.is_null() {
                    TerminateProcess(handle, 1);
                    CloseHandle(handle);
                }
            }
        }
    }
}

#[cfg(test)]
mod discovery_tests {
    use super::*;

    fn info(pid: u32) -> RuntimeInfo {
        RuntimeInfo {
            pid,
            console_port: 5556,
            adb_port: 5557,
            grpc_port: 8556,
            log: "emulator.log".into(),
        }
    }

    fn write(dir: &std::path::Path, name: &str, grpc: u16, serial: u16, token: &str) {
        std::fs::write(
            dir.join(name),
            format!("port.serial={serial}\ngrpc.port={grpc}\ngrpc.token={token}\n"),
        )
        .unwrap();
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("aae-discovery-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn finds_the_file_named_after_the_process_aae_started() {
        let dir = temp_dir("own");
        write(&dir, "pid_100.ini", 8556, 5556, "own");
        assert_eq!(
            find_grpc_token(std::slice::from_ref(&dir), &info(100)).as_deref(),
            Some("own")
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn finds_windows_files_named_after_the_emulators_own_process_by_port() {
        // emulator.exe (pid 100) launched the emulator as pid 13108.
        let dir = temp_dir("windows");
        write(&dir, "pid_200.ini", 8554, 5554, "another emulator");
        write(&dir, "pid_13108.ini", 8556, 5556, "ours");
        assert_eq!(
            find_grpc_token(std::slice::from_ref(&dir), &info(100)).as_deref(),
            Some("ours")
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn finds_nothing_for_an_emulator_without_a_file() {
        let dir = temp_dir("none");
        write(&dir, "pid_200.ini", 8554, 5554, "another emulator");
        assert_eq!(
            find_grpc_token(std::slice::from_ref(&dir), &info(100)),
            None
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
