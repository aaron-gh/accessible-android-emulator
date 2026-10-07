//! The Display and Language window: the device's language, display and
//! accessibility settings, changed without going through Android's Settings.
//! Each setting is a list of its choices; Apply Changes sets those changed.

use std::cell::RefCell;
use std::collections::HashMap;

use aae_core::device_settings::{LANGUAGE, SETTINGS};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::VK_F5;
use windows::Win32::UI::WindowsAndMessaging::{CB_SETCURSEL, IDOK};

use crate::app::{say, with_session};
use crate::panels::{self, Handler, Panel};
use crate::speech::Tone;
use crate::ui;

pub const KIND: &str = "device-settings";

const APPLY: u16 = 1000;
const REFRESH: u16 = 1001;
/// The first setting's list; the others follow.
const FIRST_CHOICE: u16 = 1100;

thread_local! {
    /// Each setting's list, in the order of SETTINGS, and the language tags field.
    static CONTROLS: RefCell<Option<(Vec<HWND>, HWND, HWND)>> = const { RefCell::new(None) };
    /// The values the device had when last read, by setting name.
    static LOADED: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
    /// The device shown, by name.
    static SHOWN: RefCell<Option<String>> = const { RefCell::new(None) };
}

pub fn title(device: Option<&str>) -> String {
    match device {
        Some(name) => format!("Display and Language: {name}"),
        None => "Display and Language".into(),
    }
}

struct DeviceSettings;

/// Opens the window, or brings it forward.
pub fn show() {
    if panels::bring_forward(KIND).is_some() {
        return;
    }
    let name = crate::app::selected().0.map(|d| d.name);
    let mut panel = Panel::new(KIND, &title(name.as_deref()), 480, 640);
    let mut lists = Vec::new();
    for (i, setting) in SETTINGS.iter().enumerate() {
        let choices: Vec<String> = setting.choices.iter().map(|(_, l)| l.to_string()).collect();
        lists.push(panel.choice(setting.label, FIRST_CHOICE + i as u16, &choices, 0));
    }
    let tags = panel.edit("Or language tags, such as fr-CA, or fr-FR,en-US", "");
    panel.buttons(
        &[("Apply Changes", APPLY), ("Refresh", REFRESH)],
        Some(APPLY),
    );
    let status = panel.text("Start the device to change its settings.");
    panel.shortcut(VK_F5.0, false, REFRESH);
    let first = lists.first().copied();
    CONTROLS.with(|c| *c.borrow_mut() = Some((lists, tags, status)));
    load();
    panel.show(DeviceSettings, first);
}

impl Handler for DeviceSettings {
    fn closed(&mut self) {
        CONTROLS.with(|c| *c.borrow_mut() = None);
    }

    fn command(&mut self, _panel: &Panel, id: u16) {
        match id {
            APPLY => apply(),
            id if id == IDOK.0 as u16 => apply(),
            REFRESH => load(),
            _ => {}
        }
    }
}

/// Shows the selected device's settings, if it's running.
pub fn load() {
    let device = crate::app::selected().0;
    SHOWN.with(|s| *s.borrow_mut() = device.as_ref().map(|d| d.name.clone()));
    let Some((_, _, status)) = CONTROLS.with(|c| c.borrow().clone()) else {
        return;
    };
    let Some(device) = device.filter(|d| d.running) else {
        panels::set_text(status, "Start the device to change its settings.");
        return;
    };
    panels::set_text(status, "Reading the device's settings.");
    crate::app::spawn(async move {
        let Ok(session) = crate::app::session_for(device.id).await else {
            return;
        };
        let result = session.device_settings().await;
        ui::run_on_ui(move || match result {
            Ok(settings) => shown(&settings),
            Err(e) => {
                if let Some((_, _, status)) = CONTROLS.with(|c| c.borrow().clone()) {
                    panels::set_text(status, &e.to_string());
                }
            }
        });
    });
}

fn shown(settings: &[aae_ffi::DeviceSettingInfo]) {
    let Some((lists, tags, status)) = CONTROLS.with(|c| c.borrow().clone()) else {
        return;
    };
    let mut loaded = HashMap::new();
    for (i, setting) in SETTINGS.iter().enumerate() {
        let now = settings.iter().find(|s| s.name == setting.name);
        // A setting this Android version lacks can't be changed.
        ui::enable(lists[i], now.is_some());
        let Some(now) = now else { continue };
        loaded.insert(now.name.clone(), now.value.clone());
        let index = setting.choices.iter().position(|(v, _)| *v == now.value);
        // -1 shows nothing chosen, for a language that isn't in the list.
        ui::send(lists[i], CB_SETCURSEL, index.unwrap_or(usize::MAX), 0);
        if setting.name == LANGUAGE {
            ui::set_text(tags, if index.is_some() { "" } else { &now.value });
        }
    }
    LOADED.with(|l| *l.borrow_mut() = loaded);
    panels::set_text(status, "Choose settings, then Apply Changes.");
}

/// Sets each setting whose choice differs from the device's, then reads
/// them all again.
fn apply() {
    let Some((lists, tags, _)) = CONTROLS.with(|c| c.borrow().clone()) else {
        return;
    };
    let loaded = LOADED.with(|l| l.borrow().clone());
    let mut changes = Vec::new();
    for (i, setting) in SETTINGS.iter().enumerate() {
        let Some(was) = loaded.get(setting.name) else {
            continue;
        };
        let typed = ui::text(tags).trim().to_string();
        let chosen = if setting.name == LANGUAGE && !typed.is_empty() {
            Some(typed)
        } else {
            ui::combo_selection(lists[i])
                .and_then(|c| setting.choices.get(c))
                .map(|(v, _)| v.to_string())
        };
        if let Some(chosen) = chosen
            && &chosen != was
        {
            changes.push((setting.name.to_string(), chosen));
        }
    }
    if changes.is_empty() {
        say("Nothing has changed.", Tone::Info);
        return;
    }
    with_session(move |session| async move {
        let mut said = Vec::new();
        for (name, value) in changes {
            said.push(session.change_device_setting(name, value).await?);
        }
        say(said.join(" "), Tone::Success);
        ui::run_on_ui(load);
        Ok(())
    });
}

/// Shows the settings again when another device is selected.
pub fn device_changed(name: Option<&str>) {
    let changed = SHOWN.with(|s| s.borrow().as_deref() != name);
    if changed && CONTROLS.with(|c| c.borrow().is_some()) {
        ui::run_on_ui(load);
    }
}
