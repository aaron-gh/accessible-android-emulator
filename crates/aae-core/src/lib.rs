//! Core of the Accessible Android Emulator (AAE).
//!
//! Platform-independent parts: Android SDK, device store, emulator control
//! over gRPC and adb, audio, and provisioning. The apps, CLI, MCP and remote
//! server are layers over this.

pub mod adb;
pub mod apk;
pub mod apk_signing;
pub mod apps;
pub mod audio;
pub mod catalog;
pub mod control;
pub mod device;
pub mod device_settings;
pub mod diagnostics;
pub mod emulator;
pub mod error;
pub mod fold;
pub mod geocode;
pub mod gestures;
pub mod googlebook;
pub mod hardware;
pub mod inspector;
pub mod keyboard_layouts;
mod keyboard_layouts_table;
pub mod keys;
pub mod keytest;
pub mod l10n;
pub mod lifecycle;
pub mod logcat;
pub mod microphone;
pub mod network;
pub mod paths;
mod platform;
pub mod provision;
pub mod qmp;
pub mod recording;
pub mod route;
pub mod screenreader;
pub mod sdk;
pub mod services;
pub mod setup;
pub mod speech_bridge;
pub mod spice;
pub mod transfer;
pub mod tts;
pub mod watch;

pub use error::{Error, Result};

/// The `adb shell` command that runs AAE's helper's shell tool as the shell
/// user, with these arguments, such as `helper_shell_tool!("locale")`.
/// Android 5 and 6 have no `cut`, so the APK's path is taken from `pm path`
/// in the shell itself.
#[macro_export]
macro_rules! helper_shell_tool {
    () => {
        "P=$(pm path io.github.aaron_gh.aae.helper); CLASSPATH=${P#package:} \
         app_process / io.github.aaron_gh.aae.helper.ShellTool"
    };
    ($args:literal) => {
        concat!($crate::helper_shell_tool!(), " ", $args)
    };
}

/// Code generated from the Android Emulator's gRPC definitions. The module
/// nesting has to match the protobuf packages so cross-package references resolve.
#[allow(clippy::all, clippy::pedantic)]
pub mod proto {
    pub mod android {
        pub mod emulation {
            pub mod control {
                tonic::include_proto!("android.emulation.control");
            }
        }
    }
    pub mod emulator_snapshot {
        tonic::include_proto!("emulator_snapshot");
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn helper_shell_tool_commands() {
        assert_eq!(
            helper_shell_tool!("locale"),
            "P=$(pm path io.github.aaron_gh.aae.helper); CLASSPATH=${P#package:} \
             app_process / io.github.aaron_gh.aae.helper.ShellTool locale"
        );
        assert!(helper_shell_tool!().ends_with(".ShellTool"));
    }
}
