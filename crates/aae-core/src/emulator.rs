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
    if device.meta.kind == crate::device::DeviceKind::Googlebook {
        return crate::googlebook::start(sdk, store, device);
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
    // graphics adapter, unless the device's hardware chooses software or the
    // adapter failed for it before (see lifecycle::start_device). AAE_GPU
    // sets another mode, for troubleshooting.
    match std::env::var("AAE_GPU").ok().filter(|g| !g.is_empty()) {
        Some(mode) => {
            command.arg("-gpu").arg(mode);
        }
        None if !device.meta.software_graphics => {
            command.arg("-gpu").arg("host");
        }
        None => {
            command.arg("-gpu").arg("swiftshader_indirect");
        }
    }
    if let Some(features) = features(&device.meta) {
        command.arg("-feature").arg(features);
    }
    if let Some(memory_mb) = memory_limit(device) {
        tracing::info!(
            "{}: this Android can use at most {memory_mb} MB of memory on this processor; starting it with that",
            device.meta.name
        );
        command.arg("-memory").arg(memory_mb.to_string());
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
    // On a computer that has never run Android Studio the folder may not
    // exist, and the emulator on Windows then fails to create its lock files
    // there and never answers.
    if let Some(dir) = crate::paths::android_user_dir() {
        std::fs::create_dir_all(&dir).context(|| format!("Creating {}", dir.display()))?;
    }

    let mut child = command
        .spawn()
        .context(|| "Starting the emulator".to_string())?;
    let info = RuntimeInfo {
        pid: child.id(),
        console_port,
        adb_port: console_port + 1,
        grpc_port,
        log,
        vm: None,
    };
    device.set_runtime(Some(&info))?;
    // Reap the process when it exits, so it doesn't linger as a zombie while
    // this process keeps running.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(info)
}

/// The newest Android whose images AAE starts without encrypting their data.
/// The Intel and AMD images of Android 6 ask the emulator to encrypt the
/// data partition at first boot (EncryptUserData in their
/// advancedFeatures.ini); the ARM ones don't. On Windows that first boot
/// never finished in ten minutes, with adb offline throughout, while the ARM
/// image of the same Android, unencrypted, starts in a minute on a Mac.
const LAST_UNENCRYPTED_API: u32 = 23;

/// The emulator features to turn on or off for this device, for `-feature`.
fn features(meta: &crate::device::DeviceMeta) -> Option<String> {
    let mut features = Vec::new();
    // See DeviceMeta::uwb_off_nfc_on.
    if meta.uwb_off_nfc_on {
        features.extend(["-Uwb", "Nfc"]);
    }
    if meta.api <= LAST_UNENCRYPTED_API {
        features.push("-EncryptUserData");
    }
    (!features.is_empty()).then(|| features.join(","))
}

/// The newest Android whose Intel and AMD images run the old 3.10 kernel.
const LAST_OLD_KERNEL_API: u32 = 25;

/// The most memory those images can use. With more than about 3.5 GB the
/// emulator places memory above 4 GB, the kernel reserves its graphics
/// memory there, and the screen can't be set up ("goldfish_fb: probe of
/// GFSH0004:00 failed with error -12"), so Android never finishes starting.
/// Seen with Android 6 and 8 GB on Windows; the ARM images are unaffected.
const OLD_KERNEL_MAX_MEMORY_MB: u32 = 3072;

/// The memory to start this device with instead of its own, when its own
/// is more than its Android can use.
fn memory_limit(device: &Device) -> Option<u32> {
    let meta = &device.meta;
    if meta.api > LAST_OLD_KERNEL_API || !meta.abi.starts_with("x86") {
        return None;
    }
    let memory_mb = crate::hardware::read(device).ok()?.memory_mb;
    (memory_mb > OLD_KERNEL_MAX_MEMORY_MB).then_some(OLD_KERNEL_MAX_MEMORY_MB)
}

/// How long the emulator may take to answer on gRPC before AAE decides it hung.
const EMULATOR_ANSWER: Duration = Duration::from_secs(60);

/// Whether the emulator's log says its graphics couldn't start, which it
/// doesn't always exit after promptly.
fn graphics_failed(log: &Path) -> bool {
    std::fs::read_to_string(log).is_ok_and(|text| text.contains("OpenGL Core Profile not supported"))
}

/// Whether the emulator's log says it couldn't connect NFC, as with
/// `-feature Nfc` on an image without a virtio-serial bus, such as Android 6.
/// It exits at once then.
pub fn nfc_unsupported(log: &Path) -> bool {
    std::fs::read_to_string(log).is_ok_and(|text| {
        text.contains("chardev=nfc") && text.contains("No 'virtio-serial-bus' bus found")
    })
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
    if info.vm.is_some() {
        return crate::googlebook::wait_until_ready(sdk, info, timeout, progress).await;
    }
    let started = tokio::time::Instant::now();
    let deadline = started + timeout;
    let adb = Adb::new(sdk.adb_bin()?, info.serial());
    let mut stage = BootStage::WaitingForEmulator;
    progress(stage);
    let mut controller = None;
    // Why the control service hasn't answered yet, for the error if it never does.
    let mut unanswered = "its access token wasn't found".to_string();

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
                if !failed && adb.boot_completed().await {
                    // Not the graphics, then, so not for starting again in software.
                    return Err(Error::Message(crate::diagnostics::redact(&format!(
                        "Android started, but the emulator's control service on port {} didn't answer within {} seconds: {unanswered}.\n{tail}",
                        info.grpc_port,
                        EMULATOR_ANSWER.as_secs()
                    ))));
                }
                let why = if failed {
                    tail
                } else {
                    format!(
                        "The emulator didn't answer on port {} within {} seconds: {unanswered}.\n{tail}",
                        info.grpc_port,
                        EMULATOR_ANSWER.as_secs()
                    )
                };
                return Err(Error::EmulatorExited(crate::diagnostics::redact(&why)));
            }
        }
        if controller.is_none() {
            // The discovery file, and so the token, appears shortly after the process starts.
            if let Some(token) = grpc_token(info) {
                match Controller::connect(info.grpc_port, Some(&token)).await {
                    Ok(c) => controller = Some(c),
                    Err(e) => unanswered = format!("connecting failed: {e}"),
                }
            }
        }
        let mut refused = false;
        if let Some(c) = &controller {
            if stage == BootStage::WaitingForEmulator {
                match c.status().await {
                    Ok(_) => {
                        stage = BootStage::WaitingForAndroid;
                        progress(stage);
                    }
                    Err(e) => {
                        unanswered = format!("it refused a request: {e}");
                        refused = true;
                    }
                }
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
        if refused {
            // The token may have been an old emulator's, read before this
            // one wrote its own: read it again.
            controller = None;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// Connects to a device that is already running.
pub async fn attach(sdk: &Sdk, device: &Device) -> Result<(RuntimeInfo, Controller, Adb)> {
    let info = running(device)?;
    let adb = adb_for(sdk, &info).await?;
    let controller = Controller::for_runtime(&info, adb.clone()).await?;
    Ok((info, controller, adb))
}

/// adb for a running device. For a Googlebook device, adb is connected to it first.
pub async fn adb_for(sdk: &Sdk, info: &RuntimeInfo) -> Result<Adb> {
    let adb = Adb::new(sdk.adb_bin()?, info.serial());
    if info.vm.is_some() {
        crate::googlebook::connect_adb(&adb).await;
    }
    Ok(adb)
}

/// Restarts Android inside a running device and waits until it is back. Unlike
/// stopping and cold booting, this keeps everything on the device's disk.
pub async fn reboot(sdk: &Sdk, device: &Device, timeout: Duration) -> Result<()> {
    let info = running(device)?;
    if info.vm.is_some() {
        // QEMU exits when Android restarts, so the VM is started again.
        let adb = Adb::new(sdk.adb_bin()?, info.serial());
        crate::googlebook::stop(device, &info, Some(&adb), timeout).await?;
        let store = crate::device::DeviceStore::open(
            device
                .dir
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_default(),
        )?;
        let info = crate::googlebook::start(sdk, &store, device)?;
        crate::googlebook::wait_until_ready(sdk, &info, timeout, |_| {}).await?;
        return Ok(());
    }
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
    if info.vm.is_some() {
        // A VM has no quick-boot snapshot: Android shuts down.
        let adb = Adb::new(sdk.adb_bin()?, info.serial());
        return crate::googlebook::stop(device, &info, Some(&adb), timeout).await;
    }
    let adb = Adb::new(sdk.adb_bin()?, info.serial());
    // Flush Android's disk writes first. "emu kill" saves memory in the
    // quick-boot snapshot, and writes still in memory would be missing from
    // the disk if the device were later cold booted: installed apps could be
    // left half there.
    let _ = tokio::time::timeout(Duration::from_secs(15), adb.shell("sync")).await;
    // "emu kill" asks the emulator to save its state and exit.
    let _ = tokio::time::timeout(Duration::from_secs(10), adb.raw(&["emu", "kill"])).await;
    // The snapshot holds all the device's memory. On Windows it is written
    // out in full, which for 8 GB can take more than a minute on a slow
    // disk, and ending the emulator part way leaves a broken snapshot.
    let timeout = timeout.max(snapshot_save_time(device));
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

/// How long saving the quick-boot snapshot may take: half a minute, and 15
/// seconds for each gigabyte of the device's memory.
fn snapshot_save_time(device: &Device) -> Duration {
    let memory_mb = crate::hardware::read(device).map_or(2048, |h| h.memory_mb);
    Duration::from_secs(30 + 15 * u64::from(memory_mb).div_ceil(1024))
}

/// Ends a device's emulator without waiting for Android, for a device whose
/// Android isn't running. Start it next with `cold_boot`, as any state the
/// emulator saves on the way out is of that Android.
pub async fn force_stop(device: &Device, info: &RuntimeInfo) -> Result<()> {
    if info.vm.is_some() {
        return crate::googlebook::stop(device, info, None, Duration::from_secs(1)).await;
    }
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
///
/// An emulator that was ended by force, as AAE does when a start fails,
/// leaves its file behind with the same ports, as AAE takes the first free
/// ones again. Its token is refused, so files of processes that have ended
/// are skipped, and of the rest the newest is taken.
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
                .and_then(|n| n.strip_prefix("pid_")?.strip_suffix(".ini"))
                .and_then(|pid| pid.parse::<u32>().ok())
                .is_some_and(is_alive)
        })
        .filter_map(|path| {
            let mut properties = crate::sdk::read_properties(&path).ok()?;
            let ours = properties.get("grpc.port") == Some(&grpc)
                && properties.get("port.serial").is_none_or(|p| *p == console);
            if !ours {
                return None;
            }
            let written = std::fs::metadata(&path)
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            Some((written, properties.remove("grpc.token")?))
        })
        .max_by_key(|(written, _)| *written)
        .map(|(_, token)| token)
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
    let all = crate::diagnostics::collapse_repeats(text.lines());
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
            vm: None,
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

    /// A process id that is running: this test's own.
    fn alive() -> u32 {
        std::process::id()
    }

    /// A process id no system hands out.
    const ENDED: u32 = 0x3FFF_FFF0;

    #[test]
    fn finds_windows_files_named_after_the_emulators_own_process_by_port() {
        // emulator.exe (pid 100) launched the emulator as another process.
        let dir = temp_dir("windows");
        write(&dir, &format!("pid_{}.ini", alive()), 8556, 5556, "ours");
        write(&dir, "pid_200.ini", 8554, 5554, "another emulator");
        assert_eq!(
            find_grpc_token(std::slice::from_ref(&dir), &info(100)).as_deref(),
            Some("ours")
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    fn meta(api: u32) -> crate::device::DeviceMeta {
        let dir = temp_dir(&format!("features-{api}"));
        let store = crate::device::DeviceStore::open(dir.clone()).unwrap();
        let level = api.to_string();
        let image = crate::sdk::SystemImage {
            api,
            release: crate::sdk::Release::read(&level, &format!("android-{api}"), false, false)
                .unwrap(),
            tag: "google_apis".into(),
            abi: "x86_64".into(),
            sysdir: format!("system-images/android-{api}/google_apis/x86_64/"),
            path: std::path::PathBuf::from("/nowhere"),
        };
        let meta = store
            .create("Phone", &image, crate::device::Profile::Phone)
            .unwrap()
            .meta;
        let _ = std::fs::remove_dir_all(dir);
        meta
    }

    fn device(api: u32, abi: &str, memory_mb: u32) -> (Device, std::path::PathBuf) {
        let dir = temp_dir(&format!("memory-{api}-{abi}-{memory_mb}"));
        let store = crate::device::DeviceStore::open(dir.clone()).unwrap();
        let level = api.to_string();
        let image = crate::sdk::SystemImage {
            api,
            release: crate::sdk::Release::read(&level, &format!("android-{api}"), false, false)
                .unwrap(),
            tag: "google_apis".into(),
            abi: abi.into(),
            sysdir: format!("system-images/android-{api}/google_apis/{abi}/"),
            path: std::path::PathBuf::from("/nowhere"),
        };
        let mut device = store
            .create("Phone", &image, crate::device::Profile::Phone)
            .unwrap();
        let mut hardware = crate::hardware::read(&device).unwrap();
        hardware.memory_mb = memory_mb;
        crate::hardware::write(&mut device, &hardware).unwrap();
        (device, dir)
    }

    #[test]
    fn old_android_on_intel_gets_at_most_3_gb() {
        for (api, abi, memory, limit) in [
            (23, "x86_64", 8192, Some(3072)),
            (23, "x86_64", 2048, None),
            (23, "arm64-v8a", 8192, None),
            (36, "x86_64", 8192, None),
        ] {
            let (device, dir) = device(api, abi, memory);
            assert_eq!(
                memory_limit(&device),
                limit,
                "Android API {api}, {abi}, {memory} MB"
            );
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn android_6_starts_without_encrypting_its_data() {
        let mut marshmallow = meta(23);
        assert_eq!(
            features(&marshmallow).as_deref(),
            Some("-Uwb,Nfc,-EncryptUserData")
        );
        marshmallow.uwb_off_nfc_on = false;
        assert_eq!(features(&marshmallow).as_deref(), Some("-EncryptUserData"));
        let mut recent = meta(36);
        assert_eq!(features(&recent).as_deref(), Some("-Uwb,Nfc"));
        recent.uwb_off_nfc_on = false;
        assert_eq!(features(&recent), None);
    }

    #[test]
    fn tells_when_this_android_cant_have_nfc() {
        let dir = temp_dir("nfc");
        let log = dir.join("emulator.log");
        // As the emulator says with an Android 6 image on Windows.
        std::fs::write(
            &log,
            "WARNING      | QEMU main loop exits abnormally with code 1\n\
             pc_memory_init: above 4g size: 40000000\n\
             qemu-system-x86_64-headless.exe: -device virtserialport,chardev=nfc,name=nfc: \
             No 'virtio-serial-bus' bus found for device 'virtserialport'\n",
        )
        .unwrap();
        assert!(nfc_unsupported(&log));
        std::fs::write(&log, "OpenGL Core Profile not supported\n").unwrap();
        assert!(!nfc_unsupported(&log));
        assert!(!nfc_unsupported(&dir.join("missing.log")));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn skips_the_file_an_emulator_ended_by_force_left_on_the_same_ports() {
        let dir = temp_dir("stale");
        write(&dir, &format!("pid_{ENDED}.ini"), 8556, 5556, "old");
        assert_eq!(
            find_grpc_token(std::slice::from_ref(&dir), &info(100)),
            None
        );
        write(&dir, &format!("pid_{}.ini", alive()), 8556, 5556, "ours");
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
