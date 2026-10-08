//! What AAE lays over the Googlebook vendor partition, with each file's
//! SELinux label and mode.
//!
//! The VM has none of the laptop's hardware (TPM, Trusty, Qualcomm DSPs, its
//! GPU), so software and virtual-device versions from Cuttlefish, and Mesa
//! for the virtual GPU, take their place; services for hardware that isn't
//! there are kept from waiting for it. AAE adds its adb setup. The choices
//! follow gbos-vm's (MIT licence, see googlebook/NOTICE.md).

use std::path::Path;

use crate::error::{IoContext, Result};

/// The time given to every added file, as in gbos-vm: 1 January 2009.
pub const MTIME: u64 = 1_230_768_000;

/// Directories the overlay has files in, which keep their original metadata.
pub const DIRS: [&str; 13] = [
    "apex",
    "bin",
    "bin/hw",
    "etc",
    "etc/init",
    "etc/init/hw",
    "etc/selinux",
    "etc/vintf",
    "etc/vintf/manifest",
    "lib64",
    "lib64/egl",
    "lib64/hw",
    "lib64/vm_keymint",
];

/// SELinux rules added to the vendor policy, as source:target:class:perms.
/// The first three let graphics clients share memfd-backed buffers (each was
/// a denial seen in the guest); the others let vendor init write AAE's adb key.
pub const POLICY_RULES: [&str; 5] = [
    "platform_app:hal_graphics_allocator_default:memfd_file:read,write,map,getattr",
    "priv_app:hal_graphics_allocator_default:memfd_file:read,write,map,getattr",
    "priv_app_36:hal_graphics_allocator_default:memfd_file:read,write,map,getattr",
    "vendor_init:adb_keys_file:dir:search,getattr,setattr,read,open,write,add_name",
    "vendor_init:adb_keys_file:file:create,getattr,setattr,open,write",
];

/// One file in the overlay.
pub struct File {
    /// Path in the vendor partition, such as "etc/init/vm-aae.rc".
    pub path: String,
    pub data: Vec<u8>,
    pub label: String,
    pub mode: u32,
}

/// Files from the component bundle (Mesa) and from Cuttlefish, and their labels.
const PREBUILT: [(&str, &str, u32); 33] = [
    (
        "apex/com.android.hardware.audio.apex",
        "vendor_apex_file",
        0o644,
    ),
    (
        "apex/com.android.hardware.gatekeeper.nonsecure.apex",
        "vendor_apex_file",
        0o644,
    ),
    (
        "bin/hw/android.hardware.boot-service.android-desktop",
        "hal_bootctl_default_exec",
        0o755,
    ),
    (
        "bin/hw/android.hardware.composer.hwc3-service.drm",
        "hal_graphics_composer_default_exec",
        0o755,
    ),
    (
        "bin/hw/android.hardware.graphics.allocator-service.minigbm",
        "hal_graphics_allocator_default_exec",
        0o755,
    ),
    (
        "bin/hw/android.hardware.security.keymint-service",
        "hal_keymint_default_exec",
        0o755,
    ),
    ("etc/audio_effects.xml", "vendor_configs_file", 0o644),
    ("etc/audio_effects_config.xml", "vendor_configs_file", 0o644),
    (
        "etc/audio_policy_configuration.xml",
        "vendor_configs_file",
        0o644,
    ),
    ("etc/audio_policy_volumes.xml", "vendor_configs_file", 0o644),
    (
        "etc/bluetooth_with_le_audio_policy_configuration_7_0.xml",
        "vendor_configs_file",
        0o644,
    ),
    (
        "etc/default_volume_tables.xml",
        "vendor_configs_file",
        0o644,
    ),
    (
        "etc/primary_audio_policy_configuration.xml",
        "vendor_configs_file",
        0o644,
    ),
    (
        "etc/r_submix_audio_policy_configuration.xml",
        "vendor_configs_file",
        0o644,
    ),
    (
        "etc/surround_sound_configuration_5_0.xml",
        "vendor_configs_file",
        0o644,
    ),
    (
        "etc/usb_audio_policy_configuration.xml",
        "vendor_configs_file",
        0o644,
    ),
    (
        "etc/vintf/manifest/vm-android.hardware.security.keymint-service.xml",
        "vendor_configs_file",
        0o644,
    ),
    (
        "etc/vintf/manifest/vm-android.hardware.security.secureclock-service.xml",
        "vendor_configs_file",
        0o644,
    ),
    (
        "etc/vintf/manifest/vm-android.hardware.security.sharedsecret-service.xml",
        "vendor_configs_file",
        0o644,
    ),
    (
        "lib64/android.hardware.graphics.allocator-V3-ndk.so",
        "same_process_hal_file",
        0o644,
    ),
    (
        "lib64/android.hardware.graphics.common-V7-ndk.so",
        "same_process_hal_file",
        0o644,
    ),
    (
        "lib64/drm_hwcomposer_atom_reporter.so",
        "same_process_hal_file",
        0o644,
    ),
    ("lib64/egl/libEGL_virgl.so", "same_process_hal_file", 0o644),
    (
        "lib64/egl/libGLESv1_CM_virgl.so",
        "same_process_hal_file",
        0o644,
    ),
    (
        "lib64/egl/libGLESv2_virgl.so",
        "same_process_hal_file",
        0o644,
    ),
    (
        "lib64/hw/gralloc.default.so",
        "same_process_hal_file",
        0o644,
    ),
    ("lib64/hw/mapper.minigbm.so", "same_process_hal_file", 0o644),
    ("lib64/hw/vulkan.virtio.so", "same_process_hal_file", 0o644),
    ("lib64/libdrm.so", "same_process_hal_file", 0o644),
    ("lib64/libgallium_dri.so", "same_process_hal_file", 0o644),
    (
        "lib64/libminigbm_gralloc.so",
        "same_process_hal_file",
        0o644,
    ),
    (
        "lib64/libminigbm_gralloc4_utils.so",
        "same_process_hal_file",
        0o644,
    ),
    ("lib64/vm_keymint/libcrypto.so", "vendor_file", 0o644),
];

/// Build properties for Mesa's virtual GPU drivers and the virtual buffer
/// allocator, replacing the image's values.
const PROPERTIES: [(&str, &str); 12] = [
    ("ro.hardware.egl", "virgl"),
    ("ro.hardware.vulkan", "virtio"),
    ("ro.hwui.use_vulkan", "true"),
    ("debug.renderengine.backend", "skiavkthreaded"),
    ("debug.renderengine.vulkan", "true"),
    ("debug.renderengine.graphite_desktop_optin", "false"),
    ("debug.hwui.renderer", "skiavk"),
    ("ro.gfx.angle.supported", "false"),
    ("ro.vendor.hwcomposer.mode", "client"),
    ("ro.surface_flinger.has_wide_color_display", "false"),
    ("ro.surface_flinger.has_HDR_display", "false"),
    ("ro.vendor.hwc.drop_drm_master", "0"),
];

const EMPTY_MANIFEST: &[u8] = b"<manifest version=\"1.0\" type=\"device\"/>\n";
const EMPTY_MANIFEST_9: &[u8] = b"<manifest version=\"9.0\" type=\"device\"/>\n";

/// The files AAE writes into the vendor partition. `original` reads a file
/// from the image's vendor partition, by its absolute path; `sources` are
/// folders holding [`PREBUILT`]'s files; `policy` adds [`POLICY_RULES`] to
/// a compiled policy.
pub fn overlay(
    original: &dyn Fn(&str) -> Result<Vec<u8>>,
    sources: &[&Path],
    policy: &dyn Fn(&[u8]) -> Result<Vec<u8>>,
) -> Result<Vec<File>> {
    let mut files: Vec<File> = Vec::new();
    let mut add = |path: &str, data: Vec<u8>, label: &str, mode: u32| {
        files.retain(|f| f.path != path);
        files.push(File {
            path: path.to_string(),
            data,
            label: label.to_string(),
            mode,
        });
    };
    for (path, label, mode) in PREBUILT {
        let source = sources
            .iter()
            .map(|dir| dir.join(path))
            .find(|p| p.exists())
            .ok_or_else(|| {
                crate::Error::Vm(format!("{path} is missing from AAE's Googlebook files"))
            })?;
        let data = std::fs::read(&source).context(|| format!("Reading {}", source.display()))?;
        add(path, data, label, mode);
    }

    let text = |b: Vec<u8>| String::from_utf8_lossy(&b).into_owned();
    let mut build = text(original("/build.prop")?)
        .lines()
        .filter(|l| {
            !PROPERTIES
                .iter()
                .any(|(k, _)| l.split('=').next() == Some(*k))
        })
        .map(|l| format!("{l}\n"))
        .collect::<String>();
    build.push_str("# Mesa's virtual GPU drivers and the virtual buffer allocator.\n");
    for (k, v) in PROPERTIES {
        build.push_str(&format!("{k}={v}\n"));
    }
    add("build.prop", build.into_bytes(), "vendor_file", 0o600);

    // The Qualcomm coprocessors don't exist in a VM.
    let kernel_rc = text(original("/etc/init/hw/init.qti.kernel.rc")?);
    let old = "on post-fs-data\n    # Late attach SOCCP and start ADSP and CDSP\n    wait_for_prop vendor.all.modules.ready 1\n    restart start-subsys";
    if kernel_rc.matches(old).count() != 1 {
        return Err(crate::Error::Vm(
            "The Googlebook image's init.qti.kernel.rc isn't the expected one.".into(),
        ));
    }
    let kernel_rc = kernel_rc.replace(
        old,
        "# The Qualcomm coprocessors don't exist in a VM.\non post-fs-data && property:ro.boot.vm.qti_subsystems=1\n    wait_for_prop vendor.all.modules.ready 1\n    restart start-subsys",
    );
    add(
        "etc/init/hw/init.qti.kernel.rc",
        kernel_rc.into_bytes(),
        "vendor_configs_file",
        0o644,
    );

    // The serial console, and only the virtual GPU's sysfs tree, labelled as
    // Google's GPU type.
    let mut contexts = original("/etc/selinux/vendor_file_contexts")?;
    contexts.extend_from_slice(b"\n/dev/ttyAMA0 u:object_r:console_device:s0\n/sys/devices/platform/3f000000\\.pcie/pci0000:00/0000:00:01\\.0(/.*)? u:object_r:sysfs_gpu:s0\n");
    add(
        "etc/selinux/vendor_file_contexts",
        contexts,
        "vendor_configs_file",
        0o644,
    );
    let precompiled = original("/etc/selinux/precompiled_sepolicy")?;
    add(
        "etc/selinux/precompiled_sepolicy",
        policy(&precompiled)?,
        "vendor_configs_file",
        0o644,
    );

    let configs = "vendor_configs_file";
    add("etc/init/vm-software-keymint.rc", b"# AOSP software KeyMint: not hardware-backed.\nservice vendor.keymint-default /vendor/bin/hw/android.hardware.security.keymint-service\n    class early_hal\n    user nobody\n    setenv LD_LIBRARY_PATH /vendor/lib64/vm_keymint:/vendor/lib64\n".to_vec(), configs, 0o644);

    // Graphics: AOSP's minigbm allocator in place of Qualcomm's, and the
    // virtual GPU's sysfs labelled.
    add("etc/init/vendor.qti.hardware.display.allocator-service.rc", b"# AOSP minigbm replaces the Qualcomm allocator.\nservice vendor.graphics.allocator /vendor/bin/hw/android.hardware.graphics.allocator-service.minigbm\n    class hal animation\n    user system\n    group graphics drmrpc\n    capabilities SYS_NICE\n    onrestart restart surfaceflinger\n    task_profiles ServiceCapacityLow\n".to_vec(), configs, 0o644);
    add("etc/vintf/manifest/vendor.qti.hardware.display.allocator-service.xml", b"<manifest version=\"9.0\" type=\"device\"><hal format=\"aidl\"><name>android.hardware.graphics.allocator</name><version>3</version><fqname>IAllocator/default</fqname></hal></manifest>\n".to_vec(), configs, 0o644);
    add("etc/vintf/manifest/mapper.qti.xml", b"<manifest version=\"9.0\" type=\"device\"><hal format=\"native\"><name>mapper</name><fqname>@5.0/minigbm</fqname></hal></manifest>\n".to_vec(), configs, 0o644);
    add("etc/init/vm-graphics.rc", b"# This path is the QEMU virtual GPU only.\non init\n    restorecon_recursive /sys/devices/platform/3f000000.pcie/pci0000:00/0000:00:01.0\n".to_vec(), configs, 0o644);

    // Audio: Cuttlefish's APEX declares Bluetooth audio itself; there's no
    // hotword DSP, and an advertised but absent sound-trigger service makes
    // system_server wait forever.
    add(
        "etc/vintf/manifest/bluetooth_audio.xml",
        EMPTY_MANIFEST.to_vec(),
        configs,
        0o644,
    );
    add(
        "etc/vintf/manifest/soundtrigger.qti.xml",
        EMPTY_MANIFEST.to_vec(),
        configs,
        0o644,
    );

    // Lock settings: AOSP's software Gatekeeper, and no Weaver hardware.
    add(
        "etc/init/android.hardware.gatekeeper-service.trusty.rc",
        b"# AOSP's software Gatekeeper APEX supplies this service in a VM.\n".to_vec(),
        configs,
        0o644,
    );
    add(
        "etc/vintf/manifest/android.hardware.gatekeeper-service.trusty.xml",
        EMPTY_MANIFEST_9.to_vec(),
        configs,
        0o644,
    );
    add(
        "etc/vintf/manifest/android.hardware.weaver-service.android-desktop.xml",
        EMPTY_MANIFEST_9.to_vec(),
        configs,
        0o644,
    );
    add(
        "etc/init/android.hardware.weaver-service.android-desktop.rc",
        b"# No Weaver hardware in a VM.\n".to_vec(),
        configs,
        0o644,
    );

    // No TPM or Trusty: stop these after their first failure rather than
    // restarting them every 5 seconds.
    add("etc/init/vm-absent-hardware.rc", b"on property:init.svc.android.system.desktop.security.gscd=restarting\n    stop android.system.desktop.security.gscd\n\non property:init.svc.vendor.secretkeeper.trusty=restarting\n    stop vendor.secretkeeper.trusty\n".to_vec(), configs, 0o644);

    // First boot goes straight to the desktop, without the setup wizard.
    add(
        "etc/init/vm-first-boot.rc",
        FIRST_BOOT_RC.to_vec(),
        configs,
        0o644,
    );
    add(
        "bin/vm-first-boot.sh",
        FIRST_BOOT_SH.to_vec(),
        "vendor_shell_exec",
        0o755,
    );

    // adb for AAE.
    add("etc/init/vm-aae.rc", AAE_RC.to_vec(), configs, 0o644);
    add("bin/vm-aae.sh", AAE_SH.to_vec(), "vendor_shell_exec", 0o755);
    Ok(files)
}

const FIRST_BOOT_RC: &[u8] = b"service vm-first-boot /system/bin/sh /vendor/bin/vm-first-boot.sh
    disabled
    oneshot
    user shell
    group shell log readproc graphics
    seclabel u:r:shell:s0

on property:sys.boot_completed=1
    start vm-first-boot
";

/// The provisioning flags AOSP's Provision app sets, for fresh userdata,
/// before any account exists. Accounts, networking and FRP are left alone.
const FIRST_BOOT_SH: &[u8] = b"#!/system/bin/sh
[ \"$(settings get global device_provisioned)\" = 1 ] && exit 0
for attempt in 1 2 3 4 5 6 7 8 9 10 11 12; do
    user=$(am get-current-user)
    case \"$user\" in
        ''|*[!0-9]*|0) sleep 5 ;;
        *) break ;;
    esac
done
case \"$user\" in ''|*[!0-9]*|0) exit 1 ;; esac
settings put global device_provisioned 1
settings --user 0 put secure user_setup_complete 1
settings --user \"$user\" put secure user_setup_complete 1
pm disable-user --user \"$user\" com.google.android.setupwizard
pm disable-user --user \"$user\" com.google.android.desktop.setupwizard
am start --user \"$user\" -a android.intent.action.MAIN -c android.intent.category.HOME -p com.google.android.apps.nexuslauncher
";

/// AAE's adb setup. AAE passes its adb public key on the kernel command
/// line; it becomes the only authorised key. The image lets adbd listen on
/// localhost only unless firmware allows more (init.adbd-killer.rc), which
/// QEMU's forwarded port can't reach, so adbd is opened on port 5555, which
/// only the host reaches. That waits until USB debugging is on, which
/// restarts adbd, so the host's first connection isn't dropped. Without the
/// key on the command line, nothing changes.
const AAE_RC: &[u8] = b"on post-fs-data && property:ro.boot.vm.adb_key=*
    mkdir /data/misc/adb 02750 system shell
    write /data/misc/adb/adb_keys ${ro.boot.vm.adb_key}
    chown system shell /data/misc/adb/adb_keys
    chmod 0640 /data/misc/adb/adb_keys

on property:sys.boot_completed=1 && property:ro.boot.vm.adb_key=*
    start vm-aae

on property:debug.vm.aae.adb=1 && property:ro.boot.vm.adb_key=*
    setprop service.adb.listen_addrs tcp:5555
    stop adbd
    start adbd

service vm-aae /system/bin/sh /vendor/bin/vm-aae.sh
    disabled
    oneshot
    user shell
    group shell log readproc
    seclabel u:r:shell:s0
";

const AAE_SH: &[u8] = b"#!/system/bin/sh
# Android stops adbd while USB debugging is off.
settings put global development_settings_enabled 1
settings put global adb_enabled 1
# Keep AAE's key authorised however long it goes unused.
settings put global adb_allowed_connection_time 0
for attempt in 1 2 3 4 5 6 7 8 9 10; do
    [ \"$(getprop init.svc.adbd)\" = running ] && break
    sleep 1
done
sleep 1
setprop debug.vm.aae.adb 1
";
