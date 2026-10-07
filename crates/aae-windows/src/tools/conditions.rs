//! The Battery, Location, Phone and Network window: what the device
//! experiences, from its battery, fingerprint sensor, motion and where it is
//! to text messages, phone calls and its network.

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
const AIRPLANE: u16 = 1020;
const WIFI: u16 = 1021;
const DATA: u16 = 1022;
const SPEED: u16 = 1023;
const SET_SPEED: u16 = 1024;
const HEALTH: u16 = 1030;
const FINGER: u16 = 1031;
const TOUCH_FINGERPRINT: u16 = 1032;
const SHAKE: u16 = 1033;
const ROUTE_SPEED: u16 = 1034;
const ROUTE: u16 = 1035;

/// The network controls, to show what the device reports after a change.
#[derive(Clone, Copy)]
struct Network {
    airplane: HWND,
    wifi: HWND,
    data: HWND,
    speed: HWND,
    status: HWND,
}

thread_local! {
    static NETWORK: std::cell::Cell<Option<Network>> = const { std::cell::Cell::new(None) };
    /// The device the network controls show, by name.
    static SHOWN: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// How fast routes play, against their own pace.
const ROUTE_SPEEDS: &[(f64, &str)] = &[
    (0.5, "Half speed"),
    (1.0, "As recorded"),
    (2.0, "Twice as fast"),
    (5.0, "Five times as fast"),
    (10.0, "Ten times as fast"),
];

/// Moves the device along a route from a GPX file, or stops the one playing.
fn toggle_route(owner: HWND, speed: f64) {
    let (device, _) = crate::app::selected();
    let Some(device) = device else { return };
    if let Some(session) = crate::app::session_if_open(&device.id)
        && session.stop_route()
    {
        return;
    }
    let files = ui::open_files(
        owner,
        &format!("Choose a GPX route for {}", device.name),
        &[("GPX routes", "*.gpx")],
        false,
    );
    let Some(path) = files.into_iter().next() else {
        return;
    };
    with_session(move |session| async move {
        let about = session.describe_route(path.clone(), speed)?;
        say(format!("Playing the route: {about}"), Tone::Info);
        say(session.play_route(path, speed).await?, Tone::Success);
        Ok(())
    });
}

/// Numbers the emulator answers to, when none is given.
const NUMBER: &str = "5551234";

pub fn title(device: Option<&str>) -> String {
    match device {
        Some(name) => format!("Battery, Location, Phone and Network: {name}"),
        None => "Battery, Location, Phone and Network".into(),
    }
}

struct Conditions {
    level: HWND,
    charging: HWND,
    health: HWND,
    finger: HWND,
    route_speed: HWND,
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
    let mut panel = Panel::new(KIND, &title(name.as_deref()), 520, 720);
    panel.text("Battery");
    let levels: Vec<String> = (0..=20)
        .rev()
        .map(|i| format!("{} percent", i * 5))
        .collect();
    let level = panel.choice("Battery level", LEVEL, &levels, 0);
    let charging = panel.check("Charging", CHARGING, true);
    let healths: Vec<String> = aae_ffi::battery_healths()
        .into_iter()
        .map(|h| h.label)
        .collect();
    let health = panel.choice("Health", HEALTH, &healths, 0);
    panel.buttons(&[("Set Battery", SET_BATTERY)], None);
    panel.text("Fingerprint and Motion");
    let fingers: Vec<String> = (1..=10).map(|i| format!("Finger {i}")).collect();
    let finger = panel.choice("Finger", FINGER, &fingers, 0);
    panel.buttons(&[("Touch Fingerprint Sensor", TOUCH_FINGERPRINT)], None);
    panel.text("Enroll fingers in Android's security settings.");
    panel.buttons(&[("Shake the Device", SHAKE)], None);
    panel.text("Location");
    let place = panel.edit("Place, address, or latitude and longitude", "");
    panel.buttons(&[("Set Location", SET_LOCATION)], None);
    let route_speed = panel.choice(
        "Route speed",
        ROUTE_SPEED,
        &ROUTE_SPEEDS
            .iter()
            .map(|(_, l)| l.to_string())
            .collect::<Vec<_>>(),
        1,
    );
    panel.buttons(&[("Play GPX Route or Stop It…", ROUTE)], None);
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
    panel.text("Outgoing calls:");
    panel.buttons(
        &[("Answer the Device's Call", ANSWER), ("Be Busy", BUSY)],
        None,
    );
    panel.text("Network");
    let airplane = panel.check("Airplane mode", AIRPLANE, false);
    let wifi = panel.check("Wi-Fi", WIFI, true);
    let data = panel.check("Mobile data", DATA, true);
    let speeds: Vec<String> = aae_ffi::network_speeds()
        .into_iter()
        .map(|s| {
            let mut d = s.description;
            if let Some(first) = d.get_mut(0..1) {
                first.make_ascii_uppercase();
            }
            d
        })
        .collect();
    let speed = panel.choice("Speed", SPEED, &speeds, 0);
    panel.buttons(&[("Set Speed", SET_SPEED)], None);
    let status = panel.text("Start the device to change its network.");
    NETWORK.with(|n| {
        n.set(Some(Network {
            airplane,
            wifi,
            data,
            speed,
            status,
        }))
    });
    load_network();
    panel.show(
        Conditions {
            level,
            charging,
            health,
            finger,
            route_speed,
            place,
            from,
            message,
            number,
        },
        Some(level),
    );
}

impl Handler for Conditions {
    fn closed(&mut self) {
        NETWORK.with(|n| n.set(None));
    }

    fn command(&mut self, _panel: &Panel, id: u16) {
        match id {
            SET_BATTERY => {
                let level = 100 - ui::combo_selection(self.level).unwrap_or(0) as u32 * 5;
                let healths = aae_ffi::battery_healths();
                let health = ui::combo_selection(self.health)
                    .and_then(|i| healths.get(i))
                    .map_or("good".to_string(), |h| h.name.clone());
                set_battery(level, ui::checked(self.charging), health);
            }
            TOUCH_FINGERPRINT => {
                let finger = ui::combo_selection(self.finger).unwrap_or(0) as u32 + 1;
                with_session(move |session| async move {
                    say(session.touch_fingerprint(finger).await?, Tone::Success);
                    Ok(())
                });
            }
            ROUTE => {
                let speed = ui::combo_selection(self.route_speed)
                    .and_then(|i| ROUTE_SPEEDS.get(i))
                    .map_or(1.0, |(s, _)| *s);
                toggle_route(_panel.hwnd, speed);
            }
            SHAKE => with_session(|session| async move {
                session.shake().await?;
                say("Shook the device.", Tone::Success);
                Ok(())
            }),
            SET_LOCATION => set_location(ui::text(self.place)),
            SEND_MESSAGE => send_message(ui::text(self.from), ui::text(self.message)),
            RING => phone_call(CallAction::Ring, &ui::text(self.number)),
            HANG_UP => phone_call(CallAction::HangUp, &ui::text(self.number)),
            HOLD => phone_call(CallAction::Hold, &ui::text(self.number)),
            RESUME => phone_call(CallAction::Resume, &ui::text(self.number)),
            ANSWER => phone_call(CallAction::Answer, &ui::text(self.number)),
            BUSY => phone_call(CallAction::Busy, &ui::text(self.number)),
            AIRPLANE | WIFI | DATA => {
                let Some(n) = NETWORK.with(|n| n.get()) else {
                    return;
                };
                let (control, kind) = match id {
                    AIRPLANE => (n.airplane, Setting::Airplane),
                    WIFI => (n.wifi, Setting::Wifi),
                    _ => (n.data, Setting::Data),
                };
                change_network(kind, ui::checked(control), String::new());
            }
            SET_SPEED => {
                let Some(n) = NETWORK.with(|n| n.get()) else {
                    return;
                };
                let speeds = aae_ffi::network_speeds();
                if let Some(chosen) = ui::combo_selection(n.speed).and_then(|i| speeds.get(i)) {
                    change_network(Setting::Speed, true, chosen.name.clone());
                }
            }
            // Enter acts on the field it was pressed in.
            id if id == IDOK.0 as u16 => {
                let focus = unsafe { GetFocus() };
                if focus == self.place {
                    self.command(_panel, SET_LOCATION);
                } else if focus == self.message || focus == self.from {
                    self.command(_panel, SEND_MESSAGE);
                } else if focus == self.number {
                    self.command(_panel, RING);
                } else if focus == self.level || focus == self.charging || focus == self.health {
                    self.command(_panel, SET_BATTERY);
                }
            }
            _ => {}
        }
    }
}

#[derive(Clone, Copy)]
enum Setting {
    Airplane,
    Wifi,
    Data,
    Speed,
}

/// Shows how the selected device's network is, if it's running.
pub fn load_network() {
    let device = crate::app::selected().0;
    SHOWN.with(|s| *s.borrow_mut() = device.as_ref().map(|d| d.name.clone()));
    let Some(n) = NETWORK.with(|n| n.get()) else {
        return;
    };
    let Some(device) = device.filter(|d| d.running) else {
        crate::panels::set_text(n.status, "Start the device to change its network.");
        return;
    };
    crate::app::spawn(async move {
        let Ok(session) = crate::app::session_for(device.id).await else {
            return;
        };
        if let Ok(info) = session.network().await {
            crate::ui::run_on_ui(move || show_network(&info));
        }
    });
}

/// Shows the network again when another device is selected.
pub fn device_changed(name: Option<&str>) {
    let changed = SHOWN.with(|s| s.borrow().as_deref() != name);
    if changed && NETWORK.with(|n| n.get()).is_some() {
        crate::ui::run_on_ui(load_network);
    }
}

fn show_network(info: &aae_ffi::NetworkInfo) {
    let Some(n) = NETWORK.with(|n| n.get()) else {
        return;
    };
    ui::set_checked(n.airplane, info.airplane);
    ui::set_checked(n.wifi, info.wifi);
    ui::set_checked(n.data, info.data);
    if let Some(i) = info.speed.as_ref().and_then(|name| {
        aae_ffi::network_speeds()
            .iter()
            .position(|s| &s.name == name)
    }) {
        ui::send(
            n.speed,
            windows::Win32::UI::WindowsAndMessaging::CB_SETCURSEL,
            i,
            0,
        );
    }
    crate::panels::set_text(n.status, &info.description);
}

/// Changes the network, then says how it is now, and shows it.
fn change_network(setting: Setting, on: bool, speed: String) {
    with_session(move |session| async move {
        match setting {
            Setting::Airplane => session.set_airplane_mode(on).await?,
            Setting::Wifi => session.set_wifi(on).await?,
            Setting::Data => session.set_mobile_data(on).await?,
            Setting::Speed => session.set_network_speed(speed).await?,
        }
        // Android takes a moment to report changes.
        tokio::time::sleep(std::time::Duration::from_millis(800)).await;
        let info = session.network().await?;
        say(info.description.clone(), Tone::Success);
        crate::ui::run_on_ui(move || show_network(&info));
        Ok(())
    });
}

fn set_battery(level: u32, charging: bool, health: String) {
    with_session(move |session| async move {
        session.set_battery(level, charging).await?;
        let health = session.set_battery_health(health).await?;
        let state = if charging { "charging" } else { "not charging" };
        say(
            format!("Battery at {level} percent, {state}. {health}"),
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
