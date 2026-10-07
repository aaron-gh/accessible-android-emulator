//! A device in a window of its own, so several devices can each have one.
//! While its window is in front, the device is the selected one, so the
//! menu's shortcuts, which work there too, act on it.

use std::cell::RefCell;
use std::collections::HashMap;

use aae_ffi::DeviceInfo;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::GetFocus;
use windows::Win32::UI::WindowsAndMessaging::SetForegroundWindow;

use crate::app;
use crate::menu::{BACK, GESTURES, HOME, KEYBOARD, NOTIFICATIONS, RECENTS};
use crate::panels::{self, Handler, Panel};
use crate::ui;

const START_STOP: u16 = 1000;
const VOLUME: u16 = 1001;

thread_local! {
    /// Each device's window kind, such as "device:Pixel_9", made once.
    static KINDS: RefCell<HashMap<String, &'static str>> = RefCell::new(HashMap::new());
}

fn kind(id: &str) -> &'static str {
    KINDS.with(|k| {
        *k.borrow_mut()
            .entry(id.to_string())
            .or_insert_with(|| Box::leak(format!("device:{id}").into_boxed_str()))
    })
}

struct Window {
    id: String,
    status: HWND,
    start_stop: HWND,
    device_buttons: Vec<HWND>,
    volume: HWND,
    shown: String,
}

/// Opens a device's own window, or brings it forward.
pub fn show(id: &str) {
    let kind = kind(id);
    if panels::bring_forward(kind).is_some() {
        return;
    }
    let mut panel = Panel::new(kind, "Device", 480, 320);
    let status = panel.text("");
    let mut first = panel.buttons(
        &[
            ("Start", START_STOP),
            ("Use Android Keyboard", KEYBOARD),
            ("Use Gestures", GESTURES),
        ],
        None,
    );
    let start_stop = first.remove(0);
    let mut device_buttons = first;
    device_buttons.extend(panel.buttons(
        &[
            ("Back", BACK),
            ("Home", HOME),
            ("Recent Apps", RECENTS),
            ("Notifications", NOTIFICATIONS),
        ],
        None,
    ));
    let levels: Vec<String> = (0..=20).map(|i| format!("{} percent", i * 5)).collect();
    let volume = panel.choice("Volume", VOLUME, &levels, 20);
    panel.use_menu_shortcuts();
    panel.every(1000);
    let mut window = Window {
        id: id.to_string(),
        status,
        start_stop,
        device_buttons,
        volume,
        shown: String::new(),
    };
    window.update(&panel);
    panel.show(window, Some(start_stop));
}

impl Window {
    fn device(&self) -> Option<DeviceInfo> {
        app::with(|app| app.devices.iter().find(|d| d.id == self.id).cloned())
    }

    /// Shows the device as it is now.
    fn update(&mut self, panel: &Panel) {
        let device = self.device();
        let busy = app::with(|app| app.busy.get(&self.id).cloned());
        let Some(device) = device else {
            panel.set_title("Device");
            panels::set_text(self.status, "This device no longer exists.");
            ui::enable(self.start_stop, false);
            for &button in &self.device_buttons {
                ui::enable(button, false);
            }
            ui::enable(self.volume, false);
            return;
        };
        let state = match &busy {
            Some(busy) => busy.to_lowercase(),
            None if device.running => "running".into(),
            None => "stopped".into(),
        };
        let text = format!("{}, {}, {state}.", device.android, device.kind);
        // Only on changes, so a screen reader reading it isn't interrupted.
        if text != self.shown {
            panel.set_title(&device.name);
            panels::set_text(self.status, &text);
            self.shown = text;
        }
        let ready = busy.is_none();
        let label = if device.running { "Stop" } else { "Start" };
        if ui::text(self.start_stop) != label {
            ui::set_text(self.start_stop, label);
        }
        ui::enable(self.start_stop, ready);
        for &button in &self.device_buttons {
            ui::enable(button, ready && device.running);
        }
        ui::enable(self.volume, ready && device.running);
        if unsafe { GetFocus() } != self.volume {
            let level = (device.volume * 20.0).round().clamp(0.0, 20.0) as usize;
            if ui::combo_selection(self.volume) != Some(level) {
                ui::set_combo_items(
                    self.volume,
                    &(0..=20)
                        .map(|i| format!("{} percent", i * 5))
                        .collect::<Vec<_>>(),
                    level,
                );
            }
        }
    }

    /// Makes this the selected device, so commands act on it.
    fn select(&self) {
        app::with(|app| {
            if app.selection.as_deref() != Some(&self.id) {
                app.selection = Some(self.id.clone());
                app.apply_audio_focus();
                app.render();
            }
        });
    }
}

impl Handler for Window {
    fn command(&mut self, panel: &Panel, id: u16) {
        if self.device().is_none() {
            return;
        }
        self.select();
        match id {
            START_STOP => {
                if self.device().is_some_and(|d| d.running) {
                    app::stop(Some(self.id.clone()));
                } else {
                    app::start(Some(self.id.clone()));
                }
            }
            VOLUME => {
                if let Some(level) = ui::combo_selection(self.volume) {
                    app::set_volume(level as f32 / 20.0, false);
                }
            }
            // Device mode takes the keyboard in the main window, which has
            // the way back.
            KEYBOARD | GESTURES => {
                unsafe {
                    let _ = SetForegroundWindow(ui::main_window());
                }
                app::command(id, 0);
            }
            BACK | HOME | RECENTS | NOTIFICATIONS => app::command(id, 0),
            _ => {}
        }
        self.update(panel);
    }

    fn activated(&mut self, panel: &Panel) {
        if self.device().is_some() {
            self.select();
        }
        self.update(panel);
    }

    fn tick(&mut self, panel: &Panel) {
        self.update(panel);
    }
}
