//! Where AAE keeps its files.

use std::path::PathBuf;

use directories::ProjectDirs;

/// AAE's data folder. Set `AAE_HOME` to use another folder, for example on an
/// external drive.
pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("AAE_HOME") {
        return PathBuf::from(dir);
    }
    // On Windows, the local folder: devices are far too big for a roaming profile.
    ProjectDirs::from("io.github", "aaron-gh", "AAE")
        .map(|dirs| dirs.data_local_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from(".aae"))
}

/// The folder holding AAE's devices. It is passed to the emulator as
/// `ANDROID_AVD_HOME`, so AAE's devices stay separate from Android Studio's.
pub fn devices_dir() -> PathBuf {
    data_dir().join("devices")
}

/// Where AAE installs the SDK parts it downloads itself, when the user has no SDK.
pub fn sdk_dir() -> PathBuf {
    data_dir().join("sdk")
}
