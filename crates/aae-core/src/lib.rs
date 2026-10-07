//! Core of the Accessible Android Emulator (AAE).
//!
//! The core owns everything that is the same on every host platform: finding the
//! Android SDK, the device store, starting and stopping emulators, talking to them
//! over gRPC and adb, playing their audio, and setting each device up so its
//! screen reader is on from the first boot. Host interfaces (the Mac app, the
//! `aae` command line, and later Windows and Linux) are thin layers over this.

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
pub mod geocode;
pub mod gestures;
pub mod inspector;
pub mod keys;
pub mod keytest;
pub mod lifecycle;
pub mod logcat;
pub mod microphone;
pub mod network;
pub mod paths;
mod platform;
pub mod provision;
pub mod recording;
pub mod screenreader;
pub mod sdk;
pub mod services;
pub mod setup;
pub mod tts;
pub mod watch;

pub use error::{Error, Result};

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
