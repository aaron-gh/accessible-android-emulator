use std::path::PathBuf;

/// Errors from the core. Every message is written to be read aloud to the user:
/// it says what went wrong and, where it can, what to do about it.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(
        "No Android SDK was found. Install Android Studio, or set ANDROID_HOME to the folder that holds the SDK."
    )]
    SdkNotFound,

    #[error(
        "The Android SDK at {0} has no emulator. Run aae setup, or open AAE's app, to install it."
    )]
    EmulatorMissing(PathBuf),

    #[error("The Android SDK at {0} has no adb. Run aae setup, or open AAE's app, to install it.")]
    AdbMissing(PathBuf),

    #[error("There is no device called \"{0}\". Use the list command to see your devices.")]
    DeviceNotFound(String),

    #[error("A device called \"{0}\" already exists. Choose another name.")]
    DeviceExists(String),

    #[error("\"{0}\" can't be used as a device name. Use letters or numbers.")]
    InvalidName(String),

    #[error("{0}")]
    NoSystemImage(String),

    #[error("\"{0}\" is already running.")]
    AlreadyRunning(String),

    #[error("\"{0}\" must be stopped before it can be {1}.")]
    MustStop(String, &'static str),

    #[error("{0} can't be deleted while devices use it: {1}. Delete those devices first.")]
    ImageInUse(String, String),

    #[error("\"{0}\" is not running.")]
    NotRunning(String),

    #[error("No free port was found to run another device. Stop a device and try again.")]
    NoFreePorts,

    #[error(
        "The device did not finish starting within {0} seconds. To report it, save a diagnostic report, which has the emulator's log with your personal details taken out: Help, Save Diagnostic Report, or aae report. The full log is at {1}."
    )]
    BootTimeout(u64, PathBuf),

    #[error("The emulator stopped while starting. The last lines of its log were:\n{0}")]
    EmulatorExited(String),

    #[error("{0}")]
    Download(String),

    #[error(
        "Google's licence for this download ({0}) hasn't been accepted yet. Read it and accept it first."
    )]
    LicenceNotAccepted(String),

    #[error("adb failed: {0}")]
    Adb(String),

    /// Something asked of AAE that can't be done, said as it is.
    #[error("{0}")]
    Message(String),

    #[error("Could not read the app package {path}: {reason}")]
    Apk { path: PathBuf, reason: String },

    #[error("No audio output device was found on this computer.")]
    NoAudioOutput,

    #[error("Audio problem: {0}")]
    Audio(String),

    #[error("Could not reach the device's control service: {0}")]
    Connect(#[from] tonic::transport::Error),

    #[error("The device's control service refused a request: {}", .0.message())]
    Rpc(#[from] tonic::Status),

    #[error("{context}: {source}")]
    Io {
        context: String,
        #[source]
        source: std::io::Error,
    },

    #[error("Could not read {path}: {reason}")]
    Config { path: PathBuf, reason: String },
}

pub type Result<T> = std::result::Result<T, Error>;

/// Adds a plain-language description of what was being done to an I/O error.
pub(crate) trait IoContext<T> {
    fn context(self, what: impl FnOnce() -> String) -> Result<T>;
}

impl<T> IoContext<T> for std::io::Result<T> {
    fn context(self, what: impl FnOnce() -> String) -> Result<T> {
        self.map_err(|source| Error::Io {
            context: what(),
            source,
        })
    }
}
