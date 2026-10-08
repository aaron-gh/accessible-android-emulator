//! Googlebook devices: Googlebook OS (Android 17, desktop) in a QEMU virtual
//! machine. Apple silicon Macs only.
//!
//! AAE installs what they need itself (see [`install`]): its component
//! bundle, UTM's QEMU, and a disk assembled from Google's recovery image for
//! the Dell Googlebook ([`image`], [`vendor`]). It runs QEMU with no window:
//! a USB keyboard and a virtio touchscreen take keys and gestures over QMP,
//! sound goes to AAE over SPICE, and adb reaches Android through a forwarded
//! port, authorised with AAE's own adb key.
//!
//! The approach follows gbos-vm by Skylar Taylor-Barrick (MIT licence, see
//! googlebook/NOTICE.md).

pub mod cuttlefish;
pub mod image;
pub mod install;
pub mod vendor;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::adb::Adb;
use crate::control::Controller;
use crate::device::{Device, DeviceStore, RuntimeInfo, VmRuntime};
use crate::emulator::BootStage;
use crate::error::{Error, IoContext, Result};
use crate::sdk::Sdk;

/// The tag Googlebook devices have in place of a system image's.
pub const TAG: &str = "googlebook";
pub const API: u32 = 37;
pub const RELEASE: &str = "Googlebook OS (Android 17)";

const DISK: &str = "googlebook.raw";
const KERNEL: &str = "kernel.Image";
const INITRD: &str = "initrd.img";
const LOG: &str = "vm.log";

/// The display. Googlebook OS is a desktop; 1920 by 1200 at 240 dpi is the
/// size gbos-vm starts with.
pub const WIDTH: u32 = 1920;
pub const HEIGHT: u32 = 1200;
pub const DENSITY: u32 = 240;
const MEMORY_MIB: u32 = 4096;

const FIRST_ADB_PORT: u16 = 6520;

/// The kernel command line for the pinned recovery image (mica-user
/// 16471258), as gbos-vm boots it. The vbmeta values describe that image's
/// original vbmeta partition.
const CMDLINE: &str = "console=ttyAMA0,115200 earlycon=pl011,0x9000000 panic=0 root=/dev/ram0 \
androidboot.hardware=android-desktop androidboot.hardware.platform=android-desktop \
androidboot.slot_suffix=_a androidboot.boot_devices=3f000000.pcie androidboot.vbmeta.size=7680 \
androidboot.vbmeta.hash_alg=sha256 \
androidboot.vbmeta.digest=9118d58c024a0b43fef17a1dcdf6b999b5ea9cbc053fd4377d5d78c445b15692 \
androidboot.vbmeta.device_state=unlocked androidboot.verifiedbootstate=orange \
androidboot.veritymode=enforcing printk.devkmsg=on \
androidboot.vendor.apex.com.android.hardware.keymint.strongbox.desktop=none \
androidboot.vendor.apex.com.android.hardware.audio.desktop=none loglevel=3";

/// Whether this computer can run Googlebook devices at all.
pub fn supported() -> bool {
    cfg!(all(target_os = "macos", target_arch = "aarch64"))
}

/// Copies the installed disk, kernel and ramdisk into a device's folder, as
/// copy-on-write clones, replacing any there (as wiping does).
pub(crate) fn copy_image(dir: &Path) -> Result<()> {
    if !install::installed() {
        return Err(Error::Message(
            "Googlebook OS isn't installed. Create a Googlebook device to install it.".into(),
        ));
    }
    for name in [DISK, KERNEL, INITRD] {
        let from = install::image().join(name);
        let to = dir.join(name);
        match std::fs::remove_file(&to) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                return Err(e).context(|| format!("Deleting {}", to.display()));
            }
            _ => {}
        }
        let status = Command::new("/bin/cp")
            .arg("-c")
            .arg(&from)
            .arg(&to)
            .status()
            .context(|| format!("Copying {}", from.display()))?;
        if !status.success() {
            return Err(Error::Message(format!(
                "Couldn't copy {} into the device. Googlebook devices need an APFS volume, \
                 for copy-on-write clones.",
                from.display()
            )));
        }
    }
    Ok(())
}

/// Starts a Googlebook device's virtual machine in the background and records
/// how to reach it. Returns once QEMU is running; use [`wait_until_ready`].
pub fn start(sdk: &Sdk, store: &DeviceStore, device: &Device) -> Result<RuntimeInfo> {
    if !supported() {
        return Err(Error::Message(
            "Googlebook devices need a Mac with Apple silicon.".into(),
        ));
    }
    if !install::installed() {
        return Err(Error::Message(
            "Googlebook OS isn't installed, or this version of AAE needs it set up again. \
             Create a Googlebook device to install it."
                .into(),
        ));
    }
    let key = adb_public_key(sdk)?;
    let used: Vec<RuntimeInfo> = store.list()?.iter().filter_map(Device::runtime).collect();
    let adb_port = (FIRST_ADB_PORT..FIRST_ADB_PORT + 200)
        .find(|&p| !used.iter().any(|r| r.adb_port == p) && port_is_free(p))
        .ok_or(Error::NoFreePorts)?;

    // Socket paths are limited to about 100 characters, so they go in a
    // short folder of their own rather than the device's.
    let run_dir = std::env::temp_dir().join(format!("aae-vm-{}", device.id));
    let _ = std::fs::remove_dir_all(&run_dir);
    std::fs::create_dir_all(&run_dir).context(|| format!("Creating {}", run_dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&run_dir, std::fs::Permissions::from_mode(0o700));
    }

    let cores = device
        .meta
        .cores
        .map_or(6.min(crate::setup::threads()), |c| c as usize)
        .clamp(1, crate::setup::threads().max(1));
    let disk = device.dir.join(DISK);
    let (host, utm) = (install::components().join("host"), install::utm());
    let mut command = Command::new(host.join("aae-vm"));
    command
        .arg("-L")
        .arg(utm.join("Resources/qemu"))
        .args(["-nodefaults", "-vga", "none", "-nic", "none"])
        .args(["-device", "virtio-gpu-gl-pci,hostmem=8G,blob=true,venus=true"])
        .args(["-global", &format!("virtio-gpu-gl-pci.xres={WIDTH}")])
        .args(["-global", &format!("virtio-gpu-gl-pci.yres={HEIGHT}")])
        .args(["-cpu", "host"])
        .args(["-smp", &format!("cpus={cores},sockets=1,cores={cores},threads=1")])
        .args(["-machine", "virt,gic-version=3,highmem=on,highmem-ecam=off"])
        .args(["-accel", "hvf,ipa-granule-size=0x1000"])
        .args(["-m", &MEMORY_MIB.to_string()])
        .arg("-kernel")
        .arg(device.dir.join(KERNEL))
        .arg("-initrd")
        .arg(device.dir.join(INITRD))
        .arg("-append")
        .arg(format!("{CMDLINE} androidboot.vm.adb_key={key}"))
        .arg("-drive")
        .arg(format!(
            "if=none,media=disk,id=disk0,format=raw,file={}",
            disk.display()
        ))
        .args(["-device", "virtio-blk-pci,drive=disk0"])
        .args(["-device", "virtio-serial", "-no-reboot"])
        .args(["-device", "qemu-xhci,id=xhci,addr=0x5"])
        // The only input devices, so QMP's keys and touches go to them.
        // Googlebook OS disables a virtio keyboard, as a built-in one, so
        // the keyboard is USB.
        .args(["-device", "usb-kbd,bus=xhci.0,id=aaekeyboard"])
        .args(["-device", "virtio-multitouch-pci,id=aaetouch"])
        .args(["-display", "none"])
        .args([
            "-spice",
            "unix=on,addr=spice.sock,disable-ticketing=on,disable-copy-paste=on,disable-agent-file-xfer=on,gl=es",
        ])
        .args([
            "-chardev",
            "socket,id=serial0,path=serial.sock,server=on,wait=off,logfile=serial.log",
        ])
        .args(["-serial", "chardev:serial0"])
        .args(["-qmp", "unix:qmp.sock,server=on,wait=off"])
        .args([
            "-netdev",
            &format!("user,id=net0,ipv6=off,hostfwd=tcp:127.0.0.1:{adb_port}-:5555"),
        ])
        .args([
            "-device",
            "usb-net,id=ethernet,netdev=net0,bus=xhci.0,mac=52:54:00:12:34:56",
        ])
        .args(["-audiodev", "spice,id=audio0"])
        .args(["-device", "usb-audio,audiodev=audio0,bus=xhci.0"])
        .current_dir(&run_dir)
        .env(
            "AAE_QEMU_LIBRARY",
            utm.join("Frameworks/qemu-aarch64-softmmu.framework/Versions/A/qemu-aarch64-softmmu"),
        )
        // AAE's virglrenderer, with the patch for sharing buffers on macOS,
        // is found ahead of UTM's.
        .env(
            "DYLD_FRAMEWORK_PATH",
            format!("{}:{}", host.join("Frameworks").display(), utm.join("Frameworks").display()),
        )
        .env("RENDER_SERVER_EXEC_PATH", host.join("virgl_render_server"))
        .env(
            "VK_DRIVER_FILES",
            utm.join("Resources/vulkan/icd.d/MoltenVK_icd.json"),
        )
        .env("ANGLE_DEFAULT_PLATFORM", "metal")
        .env("XDG_RUNTIME_DIR", &run_dir)
        .env("TMPDIR", &run_dir)
        .env_remove("APP_SANDBOX_GROUP_ID")
        .stdin(Stdio::null());
    let log = device.dir.join(LOG);
    let log_file = std::fs::File::create(&log).context(|| format!("Creating {}", log.display()))?;
    let log_err = log_file
        .try_clone()
        .context(|| format!("Opening {}", log.display()))?;
    command.stdout(log_file).stderr(log_err);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command
        .spawn()
        .context(|| "Starting the Googlebook virtual machine".to_string())?;
    let info = RuntimeInfo {
        pid: child.id(),
        console_port: 0,
        adb_port,
        grpc_port: 0,
        log,
        vm: Some(VmRuntime {
            run_dir,
            width: WIDTH,
            height: HEIGHT,
        }),
    };
    device.set_runtime(Some(&info))?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(info)
}

/// AAE's adb public key, without its comment, for the guest's adb_keys. adb
/// makes the key pair the first time its server starts.
fn adb_public_key(sdk: &Sdk) -> Result<String> {
    let dir = match std::env::var_os("ANDROID_USER_HOME") {
        Some(dir) => PathBuf::from(dir),
        None => directories::BaseDirs::new()
            .map(|b| b.home_dir().join(".android"))
            .unwrap_or_else(|| PathBuf::from(".android")),
    };
    let path = dir.join("adbkey.pub");
    if !path.exists() {
        let _ = Command::new(sdk.adb_bin()?)
            .arg("start-server")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let text = std::fs::read_to_string(&path).context(|| format!("Reading {}", path.display()))?;
    let key = text.split_whitespace().next().unwrap_or_default();
    if key.is_empty()
        || !key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "+/=".contains(c))
    {
        return Err(Error::Message(format!(
            "adb's key at {} can't be read.",
            path.display()
        )));
    }
    Ok(key.to_string())
}

fn port_is_free(port: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()
}

/// Waits until QEMU answers and Android has finished booting with adb on.
pub async fn wait_until_ready(
    sdk: &Sdk,
    info: &RuntimeInfo,
    timeout: Duration,
    mut progress: impl FnMut(BootStage),
) -> Result<(Controller, Adb)> {
    let deadline = tokio::time::Instant::now() + timeout;
    let adb = Adb::new(sdk.adb_bin()?, info.serial());
    let mut stage = BootStage::WaitingForEmulator;
    progress(stage);
    loop {
        if !crate::emulator::is_alive(info.pid) {
            return Err(Error::EmulatorExited(crate::diagnostics::redact(
                &crate::emulator::log_tail(&info.log, 15),
            )));
        }
        if tokio::time::Instant::now() > deadline {
            return Err(Error::BootTimeout(timeout.as_secs(), info.log.clone()));
        }
        if stage == BootStage::WaitingForEmulator
            && Controller::connect_vm(info, adb.clone()).await.is_ok()
        {
            stage = BootStage::WaitingForAndroid;
            progress(stage);
        }
        if stage == BootStage::WaitingForAndroid {
            connect_adb(&adb).await;
            if adb.boot_completed().await {
                prime_audio(&adb).await;
                enter_desktop_user(&adb).await?;
                match_time_zone(&adb).await;
                let controller = Controller::connect_vm(info, adb.clone()).await?;
                progress(BootStage::Ready);
                return Ok((controller, adb));
            }
        }
        tokio::time::sleep(Duration::from_millis(1000)).await;
    }
}

/// Plays silence and waits for Android's output to go to standby. After a
/// boot, the first output stream to the VM's USB audio plays nothing until
/// it goes to standby, which would lose the screen reader's first speech.
async fn prime_audio(adb: &Adb) {
    let played = adb
        .shell(
            "am broadcast -n io.github.aaron_gh.aae.helper/.CommandReceiver \
             -a io.github.aaron_gh.aae.helper.PLAY_TONE --ei hz 0",
        )
        .await;
    if !played.is_ok_and(|out| out.contains("result=") && !out.contains("result=0")) {
        return;
    }
    // The silence lasts a second and a half; standby follows a few seconds later.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    while tokio::time::Instant::now() < deadline {
        let dump = adb
            .shell("dumpsys media.audio_flinger")
            .await
            .unwrap_or_default();
        if !dump.lines().any(|l| l.starts_with("  Standby: no")) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    tracing::debug!("Android's audio output didn't go to standby");
}

/// Switches to the desktop's user. Googlebook OS runs its system user
/// headless, and after the first boot it starts on a user picker with the
/// system user in front, where AAE's settings would go to the wrong user.
/// With no screen lock, the switch also unlocks the user.
async fn enter_desktop_user(adb: &Adb) -> Result<()> {
    let list = adb.shell("cmd user list -v").await?;
    // Such as "1: id=10, name=User, type=full.SECONDARY, flags=ADMIN|FULL|..."
    let Some(user) = list.lines().find_map(|line| {
        let id = line
            .split("id=")
            .nth(1)?
            .split(',')
            .next()?
            .trim()
            .to_string();
        (line.contains("type=full.") && id != "0").then_some(id)
    }) else {
        return Ok(());
    };
    let current = || async { adb.shell("am get-current-user").await.unwrap_or_default() };
    if current().await.trim() == user {
        return Ok(());
    }
    adb.shell(&format!("am switch-user {user}")).await?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
    while tokio::time::Instant::now() < deadline {
        if current().await.trim() == user
            && adb
                .shell(&format!("am get-started-user-state {user}"))
                .await
                .is_ok_and(|s| s.contains("RUNNING_UNLOCKED"))
        {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Err(Error::Vm(format!(
        "Googlebook OS didn't switch to its desktop user ({user}) within 90 seconds."
    )))
}

/// Sets Android's time zone to the computer's. The VM's clock is the
/// computer's, but Googlebook OS starts in GMT.
async fn match_time_zone(adb: &Adb) {
    let Some(zone) = std::fs::read_link("/etc/localtime").ok().and_then(|link| {
        let link = link.to_string_lossy().into_owned();
        let zone = link.split("zoneinfo/").nth(1)?.to_string();
        zone.chars()
            .all(|c| c.is_ascii_alphanumeric() || "/_+-".contains(c))
            .then_some(zone)
    }) else {
        return;
    };
    if let Err(e) = adb.shell(&format!("cmd alarm set-timezone {zone}")).await {
        tracing::warn!("couldn't set the time zone to {zone}: {e}");
    }
}

/// Connects adb to a Googlebook device's forwarded port. adb forgets the
/// connection when its server restarts, so this is done before each use.
pub async fn connect_adb(adb: &Adb) {
    let adb_command = |args: &[&str]| {
        let run = tokio::process::Command::new(&adb.bin)
            .args(args)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .output();
        async move {
            tokio::time::timeout(Duration::from_secs(10), run)
                .await
                .ok()
                .and_then(|r| r.ok())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_default()
        }
    };
    let state = adb_command(&["-s", &adb.serial, "get-state"]).await;
    if state == "device" {
        return;
    }
    // A connection made before adbd restarted stays "offline" until it's
    // made again.
    if !state.is_empty() {
        adb_command(&["disconnect", &adb.serial]).await;
    }
    adb_command(&["connect", &adb.serial]).await;
}

/// Asks Android to shut down; QEMU exits when it has.
pub async fn power_off(adb: &Adb) -> Result<()> {
    adb.shell("sync; setprop sys.powerctl shutdown")
        .await
        .map(|_| ())
}

/// Shuts Android down through `adb` and waits for QEMU to exit; ends QEMU if
/// it's still running after `timeout`. Without adb, QEMU is ended at once.
pub async fn stop(
    device: &Device,
    info: &RuntimeInfo,
    adb: Option<&Adb>,
    timeout: Duration,
) -> Result<()> {
    if let Some(adb) = adb {
        connect_adb(adb).await;
        let _ = tokio::time::timeout(Duration::from_secs(15), power_off(adb)).await;
    }
    let deadline = tokio::time::Instant::now() + timeout;
    while crate::emulator::is_alive(info.pid) && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    if crate::emulator::is_alive(info.pid) {
        if let Some(vm) = &info.vm {
            if let Ok(qmp) =
                crate::qmp::Qmp::connect(&vm.run_dir.join("qmp.sock"), (vm.width, vm.height)).await
            {
                let _ = qmp.quit().await;
            }
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    #[cfg(unix)]
    if crate::emulator::is_alive(info.pid) {
        unsafe {
            libc::kill(info.pid as libc::pid_t, libc::SIGKILL);
        }
    }
    if let Some(vm) = &info.vm {
        let _ = std::fs::remove_dir_all(&vm.run_dir);
    }
    device.set_runtime(None)
}
