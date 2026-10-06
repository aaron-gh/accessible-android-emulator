//! Core of the Accessible Android Emulator (AAE).
//!
//! The core owns everything that is the same on every host platform: finding the
//! Android SDK, the device store, starting and stopping emulators, talking to them
//! over gRPC and adb, playing their audio, and setting each device up so its
//! screen reader is on from the first boot. Host interfaces (the Mac app, the
//! `aae` command line, and later Windows and Linux) are thin layers over this.

pub mod adb;
pub mod apk;
pub mod audio;
pub mod control;
pub mod device;
pub mod emulator;
pub mod error;
pub mod keys;
pub mod keytest;
pub mod lifecycle;
pub mod paths;
pub mod provision;
pub mod sdk;
pub mod speech;
pub mod tts;

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
