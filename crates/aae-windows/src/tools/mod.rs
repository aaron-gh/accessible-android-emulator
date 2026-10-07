//! The Mac app's tools beyond the main window: forms that act on the
//! selected device, and tool windows that stay open beside the main one.

pub mod conditions;
pub mod links;

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
