//! AAE's records as JSON, for the phone.

use aae_ffi::{
    DeviceInfo, ImageInfo, InstalledImageInfo, LicenceInfo, SetupStatus, ToolInfo, VersionInfo,
};
use serde_json::{Value, json};

pub fn device(d: &DeviceInfo) -> Value {
    json!({
        "id": d.id,
        "name": d.name,
        "android": d.android,
        "kind": d.kind,
        "profile": d.profile,
        "running": d.running,
        "screen_reader": d.screen_reader,
        "screen_reader_declined": d.screen_reader_declined,
        "speech_log": d.speech_log,
        "backtalk_supported": d.backtalk_supported,
        "volume": d.volume,
    })
}

pub fn image(i: &ImageInfo) -> Value {
    json!({
        "sysdir": i.sysdir,
        "api": i.api,
        "description": i.description,
    })
}

pub fn installed_image(i: &InstalledImageInfo) -> Value {
    json!({
        "sysdir": i.sysdir,
        "description": i.description,
        "size": i.size,
        "devices": i.devices,
        "other_devices": i.other_devices,
    })
}

pub fn version(v: &VersionInfo) -> Value {
    json!({
        "id": v.id,
        "description": v.description,
        "size": v.size,
        "installed": v.installed,
        "sysdir": v.sysdir,
        "preview": v.preview,
    })
}

pub fn licence(l: &LicenceInfo) -> Value {
    json!({ "id": l.id, "text": l.text })
}

fn tool(t: &ToolInfo) -> Value {
    json!({ "name": t.name, "revision": t.revision, "size": t.size })
}

pub fn setup(s: &SetupStatus) -> Value {
    json!({
        "sdk_path": s.sdk_path,
        "own_sdk": s.own_sdk,
        "virtualisation_problem": s.virtualisation_problem,
        "missing": s.missing.iter().map(tool).collect::<Vec<_>>(),
        "updates": s.updates.iter().map(tool).collect::<Vec<_>>(),
        "managed_elsewhere": s.managed_elsewhere,
        "missing_size": s.missing_size,
        "performance_warning": s.performance_warning,
    })
}
