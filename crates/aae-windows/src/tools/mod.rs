//! The Mac app's tools beyond the main window: forms that act on the
//! selected device, and tool windows that stay open beside the main one.

pub mod apps;
pub mod conditions;
pub mod device_log;
pub mod device_window;
pub mod inspector;
pub mod links;
pub mod services;
pub mod shell;
pub mod snapshots;
pub mod speech_log;
pub mod versions;
pub mod watch;

use windows::Win32::Foundation::HWND;

use crate::app::announce;
use crate::speech::Tone;
use crate::{panels, ui};

/// Keeps tool windows' titles naming the selected device.
pub fn device_changed(name: Option<&str>) {
    if let Some(hwnd) = panels::open_window(conditions::KIND) {
        let title = conditions::title(name);
        if ui::text(hwnd) != title {
            ui::set_text(hwnd, &title);
        }
    }
}

/// Puts a window's text on the clipboard.
fn copy_all(owner: HWND, text: &str, done: &str) {
    if text.is_empty() {
        announce("There's nothing to copy.", Tone::Failure);
    } else if ui::set_clipboard_text(owner, text) {
        announce(done, Tone::Info);
    } else {
        announce("Windows' clipboard couldn't be changed.", Tone::Failure);
    }
}

/// Asks where to save a window's text, then saves it.
fn save_text(owner: HWND, name: &str, text: &str) {
    if text.is_empty() {
        announce("There's nothing to save.", Tone::Failure);
        return;
    }
    let Some(path) = ui::save_file(owner, "Save", name, &[("Text files", "*.txt")], "txt") else {
        return;
    };
    match std::fs::write(&path, text.replace('\n', "\r\n")) {
        Ok(()) => announce(
            &format!("Saved {}.", crate::app::file_name(&path)),
            Tone::Success,
        ),
        Err(e) => announce(&format!("Couldn't save: {e}"), Tone::Failure),
    }
}

/// True if the selected device is running; otherwise says why not.
fn running() -> bool {
    if crate::app::selected().0.is_some_and(|d| d.running) {
        return true;
    }
    // Says why.
    crate::app::with_session(|_| async { Ok(()) });
    false
}
