//! Updates through WinSparkle: reads the Windows update feed, verifies the
//! download's signature with AAE's update key (shared with the Mac), and runs
//! the installer, which restarts AAE. Versions compare by build number (commit
//! count), so a development build is offered the next stable release.
//!
//! WinSparkle.dll ships next to AAE and is loaded when AAE starts; without
//! it, AAE works but doesn't update.

use std::sync::OnceLock;

use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::core::{PCSTR, PCWSTR, s};

use crate::ui;

/// AAE's Windows update feed.
const FEED_URL: &str = match option_env!("AAE_WINDOWS_FEED_URL") {
    Some(url) => url,
    None => {
        "https://raw.githubusercontent.com/aaron-gh/accessible-android-emulator/master/appcast-windows.xml"
    }
};

/// The public half of AAE's update key, shared with the Mac app.
const PUBLIC_KEY: &str = include_str!("../../../macos/Support/sparkle-public-key.txt");

/// The build number updates are compared by, set by windows/build.sh.
pub const BUILD_NUMBER: &str = match option_env!("AAE_BUILD_NUMBER") {
    Some(number) => number,
    None => "0",
};

type NoArguments = unsafe extern "C" fn();
type WithText = unsafe extern "C" fn(*const u8);
type WithKey = unsafe extern "C" fn(*const u8) -> i32;
type WithDetails = unsafe extern "C" fn(PCWSTR, PCWSTR, PCWSTR);
type WithWideText = unsafe extern "C" fn(PCWSTR);
type WithFlag = unsafe extern "C" fn(i32);
type CanShutdown = unsafe extern "C" fn() -> i32;
type WithCanShutdown = unsafe extern "C" fn(CanShutdown);
type WithShutdown = unsafe extern "C" fn(NoArguments);

struct WinSparkle {
    init: NoArguments,
    cleanup: NoArguments,
    check_with_ui: NoArguments,
    set_appcast_url: WithText,
    set_eddsa_public_key: WithKey,
    set_app_details: WithDetails,
    set_app_build_version: WithWideText,
    set_can_shutdown_callback: WithCanShutdown,
    set_shutdown_request_callback: WithShutdown,
    set_automatic_check_for_updates: WithFlag,
}

fn library() -> Option<&'static WinSparkle> {
    static LIBRARY: OnceLock<Option<WinSparkle>> = OnceLock::new();
    LIBRARY.get_or_init(load).as_ref()
}

fn load() -> Option<WinSparkle> {
    let path = std::env::current_exe()
        .ok()?
        .parent()?
        .join("WinSparkle.dll");
    let path = ui::wide(&path.to_string_lossy());
    let module = unsafe { LoadLibraryW(PCWSTR(path.as_ptr())) }.ok()?;
    let find = |name: PCSTR| unsafe { GetProcAddress(module, name) };
    // Each is looked up as a general function, then called as what it is.
    type Found = unsafe extern "system" fn() -> isize;
    macro_rules! cast {
        ($found:expr, $type:ty) => {
            unsafe { std::mem::transmute::<Found, $type>($found?) }
        };
    }
    Some(WinSparkle {
        init: cast!(find(s!("win_sparkle_init")), NoArguments),
        cleanup: cast!(find(s!("win_sparkle_cleanup")), NoArguments),
        check_with_ui: cast!(find(s!("win_sparkle_check_update_with_ui")), NoArguments),
        set_appcast_url: cast!(find(s!("win_sparkle_set_appcast_url")), WithText),
        set_eddsa_public_key: cast!(find(s!("win_sparkle_set_eddsa_public_key")), WithKey),
        set_app_details: cast!(find(s!("win_sparkle_set_app_details")), WithDetails),
        set_app_build_version: cast!(find(s!("win_sparkle_set_app_build_version")), WithWideText),
        set_can_shutdown_callback: cast!(
            find(s!("win_sparkle_set_can_shutdown_callback")),
            WithCanShutdown
        ),
        set_shutdown_request_callback: cast!(
            find(s!("win_sparkle_set_shutdown_request_callback")),
            WithShutdown
        ),
        set_automatic_check_for_updates: cast!(
            find(s!("win_sparkle_set_automatic_check_for_updates")),
            WithFlag
        ),
    })
}

/// AAE is always ready to close for an update: devices keep running, and
/// come back when AAE starts again.
unsafe extern "C" fn can_shutdown() -> i32 {
    1
}

/// WinSparkle has started the installer; AAE closes so it can be replaced.
unsafe extern "C" fn shutdown() {
    ui::run_on_ui(|| unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
            Some(ui::main_window()),
            windows::Win32::UI::WindowsAndMessaging::WM_CLOSE,
            Default::default(),
            Default::default(),
        );
    });
}

/// Starts WinSparkle, once AAE's window is showing. It may check for updates
/// straight away, in the background.
pub fn start(display_version: &str) {
    let Some(sparkle) = library() else {
        tracing::info!("WinSparkle.dll isn't next to AAE, so AAE won't update itself");
        return;
    };
    let url: Vec<u8> = FEED_URL.bytes().chain([0]).collect();
    let key: Vec<u8> = PUBLIC_KEY.trim().bytes().chain([0]).collect();
    let company = ui::wide("aaron-gh");
    let name = ui::wide("Accessible Android Emulator");
    let version = ui::wide(display_version);
    let build = ui::wide(BUILD_NUMBER);
    unsafe {
        (sparkle.set_appcast_url)(url.as_ptr());
        if (sparkle.set_eddsa_public_key)(key.as_ptr()) != 1 {
            tracing::warn!("WinSparkle didn't accept AAE's update key, so AAE won't update itself");
            return;
        }
        (sparkle.set_app_details)(
            PCWSTR(company.as_ptr()),
            PCWSTR(name.as_ptr()),
            PCWSTR(version.as_ptr()),
        );
        (sparkle.set_app_build_version)(PCWSTR(build.as_ptr()));
        (sparkle.set_can_shutdown_callback)(can_shutdown);
        (sparkle.set_shutdown_request_callback)(shutdown);
        // A build made without a build number can't be compared, so it
        // doesn't check by itself.
        if BUILD_NUMBER == "0" {
            (sparkle.set_automatic_check_for_updates)(0);
        }
        (sparkle.init)();
    }
}

/// Checks for updates now, saying so even when there are none, because the
/// user asked. Returns false when this copy of AAE can't update.
pub fn check() -> bool {
    match library() {
        Some(sparkle) => {
            unsafe { (sparkle.check_with_ui)() };
            true
        }
        None => false,
    }
}

/// Stops WinSparkle's background work, before AAE closes.
pub fn stop() {
    if let Some(sparkle) = library() {
        unsafe { (sparkle.cleanup)() };
    }
}
