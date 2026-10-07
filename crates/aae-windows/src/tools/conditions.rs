//! The Battery, Location and Phone window: what the device experiences,
//! from its battery and where it is to text messages and phone calls.

use aae_ffi::CallAction;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::GetFocus;
use windows::Win32::UI::WindowsAndMessaging::IDOK;

use crate::app::{announce, say, with_session};
use crate::panels::{self, Handler, Panel};
use crate::speech::Tone;
use crate::ui;

pub const KIND: &str = "conditions";

const LEVEL: u16 = 1000;
const SET_BATTERY: u16 = 1001;
const CHARGING: u16 = 1002;
const SET_LOCATION: u16 = 1003;
const SEND_MESSAGE: u16 = 1004;
const RING: u16 = 1010;
const HANG_UP: u16 = 1011;
const HOLD: u16 = 1012;
const RESUME: u16 = 1013;
const ANSWER: u16 = 1014;
const BUSY: u16 = 1015;

/// Numbers the emulator answers to, when none is given.
const NUMBER: &str = "5551234";

pub fn title(device: Option<&str>) -> String {
    match device {
        Some(name) => format!("Battery, Location and Phone: {name}"),
        None => "Battery, Location and Phone".into(),
    }
}

struct Conditions {
    level: HWND,
    charging: HWND,
    place: HWND,
    from: HWND,
    message: HWND,
    number: HWND,
}

/// Opens the window, or brings it forward.
pub fn show() {
    if panels::bring_forward(KIND).is_some() {
        return;
    }
    let name = crate::app::selected().0.map(|d| d.name);
    let mut panel = Panel::new(KIND, &title(name.as_deref()), 520, 620);
    panel.text("Battery");
    let levels: Vec<String> = (0..=20)
        .rev()
        .map(|i| format!("{} percent", i * 5))
        .collect();
    let level = panel.choice("Battery level", LEVEL, &levels, 0);
    let charging = panel.check("Charging", CHARGING, true);
    panel.buttons(&[("Set Battery", SET_BATTERY)], None);
    panel.text("Location");
    let place = panel.edit("Place, address, or latitude and longitude", "");
    panel.buttons(&[("Set Location", SET_LOCATION)], None);
    panel.text("Text Message");
    let from = panel.edit("From", NUMBER);
    let message = panel.edit("Message", "");
    panel.buttons(&[("Send Text Message", SEND_MESSAGE)], None);
    panel.text("Phone Call");
    let number = panel.edit("Number", NUMBER);
    panel.buttons(
        &[
            ("Call the Device", RING),
            ("Hang Up", HANG_UP),
            ("Hold", HOLD),
            ("Resume", RESUME),
        ],
        None,
    );
    panel.text("When the device calls out, the number it calls can answer or be busy:");
    panel.buttons(
        &[("Answer the Device's Call", ANSWER), ("Be Busy", BUSY)],
        None,
    );
    panel.show(
        Conditions {
            level,
            charging,
            place,
            from,
            message,
            number,
        },
        Some(level),
    );
}

impl Handler for Conditions {
    fn command(&mut self, _panel: &Panel, id: u16) {
        match id {
            SET_BATTERY => {
                let level = 100 - ui::combo_selection(self.level).unwrap_or(0) as u32 * 5;
                set_battery(level, ui::checked(self.charging));
            }
            SET_LOCATION => set_location(ui::text(self.place)),
            SEND_MESSAGE => send_message(ui::text(self.from), ui::text(self.message)),
            RING => phone_call(CallAction::Ring, &ui::text(self.number)),
            HANG_UP => phone_call(CallAction::HangUp, &ui::text(self.number)),
            HOLD => phone_call(CallAction::Hold, &ui::text(self.number)),
            RESUME => phone_call(CallAction::Resume, &ui::text(self.number)),
            ANSWER => phone_call(CallAction::Answer, &ui::text(self.number)),
            BUSY => phone_call(CallAction::Busy, &ui::text(self.number)),
            // Enter acts on the field it was pressed in.
            id if id == IDOK.0 as u16 => {
                let focus = unsafe { GetFocus() };
                if focus == self.place {
                    self.command(_panel, SET_LOCATION);
                } else if focus == self.message || focus == self.from {
                    self.command(_panel, SEND_MESSAGE);
                } else if focus == self.number {
                    self.command(_panel, RING);
                } else if focus == self.level || focus == self.charging {
                    self.command(_panel, SET_BATTERY);
                }
            }
            _ => {}
        }
    }
}

fn set_battery(level: u32, charging: bool) {
    with_session(move |session| async move {
        session.set_battery(level, charging).await?;
        let state = if charging { "charging" } else { "not charging" };
        say(
            format!("Battery at {level} percent, {state}."),
            Tone::Success,
        );
        Ok(())
    });
}

/// Sets the location from "latitude, longitude", or a place or address,
/// looked up on OpenStreetMap.
fn set_location(text: String) {
    let text = text.trim().to_string();
    if text.is_empty() {
        return;
    }
    let typed = aae_core::geocode::coordinates(&text);
    if typed.is_none() {
        announce(&format!("Looking up {text}."), Tone::Info);
    }
    with_session(move |session| async move {
        let (latitude, longitude, place) = match typed {
            Some((lat, lon)) => (lat, lon, None),
            None => {
                let query = text.clone();
                let found = tokio::task::spawn_blocking(move || aae_core::geocode::look_up(&query))
                    .await
                    .map_err(|e| aae_ffi::AaeError::Failed {
                        message: e.to_string(),
                    })?
                    .map_err(aae_ffi::AaeError::from)?;
                let Some(place) = found else {
                    say(
                        format!("Couldn't find {text}. Try an address, or latitude and longitude."),
                        Tone::Failure,
                    );
                    return Ok(());
                };
                (place.latitude, place.longitude, Some(place.name))
            }
        };
        session.set_location(latitude, longitude).await?;
        let place = place.map(|p| format!("{p}, ")).unwrap_or_default();
        say(
            format!("Location set to {place}{latitude:.5}, {longitude:.5}."),
            Tone::Success,
        );
        Ok(())
    });
}

fn send_message(from: String, text: String) {
    if text.is_empty() {
        announce("Write a message first.", Tone::Failure);
        return;
    }
    let from = if from.trim().is_empty() {
        NUMBER.to_string()
    } else {
        from
    };
    with_session(move |session| async move {
        session.send_sms(from.clone(), text).await?;
        say(format!("Sent a text message from {from}."), Tone::Success);
        Ok(())
    });
}

fn phone_call(action: CallAction, number: &str) {
    let number = if number.trim().is_empty() {
        NUMBER
    } else {
        number.trim()
    }
    .to_string();
    with_session(move |session| async move {
        let said = match action {
            CallAction::Ring => format!("{number} is calling the device."),
            CallAction::HangUp => "Hung up.".into(),
            CallAction::Answer => "Answered the device's call.".into(),
            CallAction::Busy => "Busy for the device's call.".into(),
            CallAction::Hold => "Call on hold.".into(),
            CallAction::Resume => "Call taken off hold.".into(),
        };
        session.phone_call(action, number).await?;
        say(said, Tone::Info);
        Ok(())
    });
}
