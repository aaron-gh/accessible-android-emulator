//! The Accessibility Services window: screen readers first, as one choice,
//! since only one runs at a time; then every other service, each turned on
//! or off. What's turned on stays on, and what's turned off stays off.

use std::cell::RefCell;

use aae_ffi::ServiceInfo;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetFocus, VK_F5, VK_R};
use windows::Win32::UI::WindowsAndMessaging::IDOK;

use crate::app::{self, announce, say, session_for, spawn, with_session};
use crate::panels::{self, Handler, Panel};
use crate::speech::Tone;
use crate::ui::{self, run_on_ui};

pub const KIND: &str = "services";

const READER: u16 = 1000;
const USE_READER: u16 = 1001;
const OTHERS: u16 = 1002;
const TOGGLE: u16 = 1003;
const REFRESH: u16 = 1004;

#[derive(Clone, Copy)]
struct Controls {
    status: HWND,
    reader: HWND,
    others: HWND,
    toggle: HWND,
}

#[derive(Default)]
struct State {
    window: Option<Controls>,
    services: Vec<ServiceInfo>,
    device: Option<(String, bool)>,
    generation: u64,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

fn screen_readers(s: &State) -> Vec<ServiceInfo> {
    s.services
        .iter()
        .filter(|s| s.screen_reader)
        .cloned()
        .collect()
}

fn others(s: &State) -> Vec<ServiceInfo> {
    s.services
        .iter()
        .filter(|s| !s.screen_reader)
        .cloned()
        .collect()
}

/// Opens the window, or brings it forward.
pub fn show() {
    if panels::bring_forward(KIND).is_some() {
        return;
    }
    let mut panel = Panel::new(KIND, "Accessibility Services", 560, 500);
    let status = panel.text("Reading…");
    let reader = panel.choice("Screen reader", READER, &["None".into()], 0);
    panel.buttons(&[("Use This Screen Reader", USE_READER)], None);
    let others = panel.list("Other services", OTHERS, 10, true);
    let toggle = panel.buttons(&[("Turn On or Off", TOGGLE), ("Refresh", REFRESH)], None)[0];
    panel.shortcut(VK_F5.0, false, REFRESH);
    panel.shortcut(VK_R.0, true, REFRESH);
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.window = Some(Controls {
            status,
            reader,
            others,
            toggle,
        });
        s.device = None;
    });
    load(&panel);
    panel.every(1000);
    panel.show(Window, Some(reader));
}

/// Reads the selected device's services.
fn load(panel: &Panel) {
    let device = app::selected().0;
    panel.set_title(&match &device {
        Some(d) => format!("Accessibility Services on {}", d.name),
        None => "Accessibility Services".into(),
    });
    let generation = STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.device = device.as_ref().map(|d| (d.id.clone(), d.running));
        s.generation += 1;
        s.generation
    });
    let Some(device) = device.filter(|d| d.running) else {
        show_services(
            Vec::new(),
            "Start the device to see its accessibility services.",
        );
        return;
    };
    if let Some(c) = STATE.with(|s| s.borrow().window) {
        panels::set_text(c.status, "Reading…");
    }
    spawn(async move {
        let result = async { session_for(device.id).await?.list_services().await }.await;
        run_on_ui(move || {
            if STATE.with(|s| s.borrow().generation) != generation {
                return;
            }
            match result {
                Ok(services) if services.is_empty() => {
                    show_services(services, "No accessibility services are installed.")
                }
                Ok(services) => show_services(services, ""),
                Err(e) => show_services(Vec::new(), &e.to_string()),
            }
        });
    });
}

fn show_services(services: Vec<ServiceInfo>, status: &str) {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.services = services;
        let Some(c) = s.window else { return };
        panels::set_text(c.status, status);
        let readers = screen_readers(&s);
        let mut items = vec!["None".to_string()];
        items.extend(readers.iter().map(|r| r.label.clone()));
        let on = readers.iter().position(|r| r.on).map_or(0, |i| i + 1);
        ui::set_combo_items(c.reader, &items, on);
        let rows: Vec<String> = others(&s)
            .iter()
            .map(|service| {
                let state = if service.on { "on" } else { "off" };
                if service.description.is_empty() {
                    format!("{}: {state}", service.label)
                } else {
                    format!("{}: {state}. {}", service.label, service.description)
                }
            })
            .collect();
        let selected = ui::list_selection(c.others)
            .filter(|&i| i < rows.len())
            .or((!rows.is_empty()).then_some(0));
        ui::set_list_items(c.others, &rows, selected);
    });
    show_toggle();
}

/// Names the button after what it does to the selected service.
fn show_toggle() {
    STATE.with(|s| {
        let s = s.borrow();
        let Some(c) = s.window else { return };
        let service = ui::list_selection(c.others).and_then(|i| others(&s).get(i).cloned());
        ui::set_text(
            c.toggle,
            match &service {
                Some(service) if service.on => "Turn Off",
                Some(_) => "Turn On",
                None => "Turn On or Off",
            },
        );
        ui::enable(c.toggle, service.is_some());
    });
}

/// Turns a service on, to stay on, or off. A screen reader turned on
/// becomes the device's screen reader instead of the one it had.
fn set_service(service: ServiceInfo, on: bool) {
    if on && service.screen_reader {
        announce(&format!("Switching to {}.", service.label), Tone::Info);
    }
    with_session(move |session| async move {
        session.set_service(service.component.clone(), on).await?;
        if on && service.screen_reader {
            say(
                format!("{} is now the screen reader.", service.label),
                Tone::Success,
            );
        } else {
            let state = if on { "on" } else { "off" };
            say(format!("{} {state}.", service.label), Tone::Success);
        }
        run_on_ui(|| {
            panels::press(KIND, REFRESH);
            app::with(|app| {
                app.refresh();
                app.render();
            });
        });
        Ok(())
    });
}

struct Window;

impl Handler for Window {
    fn command(&mut self, panel: &Panel, id: u16) {
        let Some(c) = STATE.with(|s| s.borrow().window) else {
            return;
        };
        // Enter acts on the control it's pressed in.
        let id = match id {
            id if id == IDOK.0 as u16 => match unsafe { GetFocus() } {
                f if f == c.reader => USE_READER,
                f if f == c.others => TOGGLE,
                _ => return,
            },
            id => id,
        };
        match id {
            REFRESH => load(panel),
            OTHERS => show_toggle(),
            USE_READER => {
                let (readers, chosen) = STATE.with(|s| {
                    let s = s.borrow();
                    (
                        screen_readers(&s),
                        ui::combo_selection(c.reader).unwrap_or(0),
                    )
                });
                let current = readers.iter().find(|r| r.on).cloned();
                match chosen.checked_sub(1).and_then(|i| readers.get(i)).cloned() {
                    // None turns the one that's on off.
                    None => match current {
                        Some(current) => set_service(current, false),
                        None => announce("No screen reader is on.", Tone::Info),
                    },
                    Some(reader) if reader.on => announce(
                        &format!("{} is already the screen reader.", reader.label),
                        Tone::Info,
                    ),
                    Some(reader) => set_service(reader, true),
                }
            }
            TOGGLE => {
                let service = STATE.with(|s| {
                    let s = s.borrow();
                    ui::list_selection(c.others).and_then(|i| others(&s).get(i).cloned())
                });
                if let Some(service) = service {
                    let on = !service.on;
                    set_service(service, on);
                }
            }
            _ => {}
        }
    }

    /// Reads the services again when another device is selected, or it
    /// starts or stops.
    fn tick(&mut self, panel: &Panel) {
        let device = app::selected().0.map(|d| (d.id, d.running));
        if STATE.with(|s| s.borrow().device != device) {
            load(panel);
        }
    }

    fn closed(&mut self) {
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            s.window = None;
            s.services.clear();
        });
    }
}
