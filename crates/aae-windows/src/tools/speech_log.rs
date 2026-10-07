//! The Speech Log window: what the screen reader has said, with times, while
//! the speech log is on. New speech appears as it happens.

use std::cell::RefCell;

use aae_ffi::UtteranceInfo;
use windows::Win32::Foundation::HWND;

use crate::app::{self, announce, say, session_for, spawn, with_session};
use crate::panels::{self, Handler, Panel};
use crate::speech::Tone;
use crate::ui::{self, run_on_ui};

pub const KIND: &str = "speech-log";

const RECORD: u16 = 1000;
const LIST: u16 = 1001;
const COPY: u16 = 1002;
const SAVE: u16 = 1003;
const CLEAR: u16 = 1004;

#[derive(Default)]
struct State {
    /// The device whose speech is shown, by id, and its name.
    device: Option<(String, String)>,
    entries: Vec<UtteranceInfo>,
    /// The time of the newest speech shown.
    since: u64,
    /// Counts the times the log was started afresh, so late answers for an
    /// earlier device are dropped.
    generation: u64,
    fetching: bool,
    busy: bool,
    window: Option<(HWND, HWND)>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

fn line(u: &UtteranceInfo) -> String {
    format!("{}, at {}", u.text, u.clock)
}

/// Opens the window, or brings it forward.
pub fn show() {
    if panels::bring_forward(KIND).is_some() {
        return;
    }
    let mut panel = Panel::new(KIND, "Speech Log", 580, 480);
    let record = panel.check("Record speech", RECORD, false);
    panel.text("While recording, speech goes through AAE's speech log on its way to the device's speech engine. It adds about 10 milliseconds.");
    let list = panel.list("Speech", LIST, 14, true);
    panel.buttons(
        &[("Copy All", COPY), ("Save…", SAVE), ("Clear", CLEAR)],
        None,
    );
    STATE.with(|s| s.borrow_mut().window = Some((record, list)));
    watch(&panel);
    panel.every(500);
    panel.show(Window, Some(list));
}

/// Starts showing the selected device's speech, from the start.
fn watch(panel: &Panel) {
    let device = app::selected().0;
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.device = device.as_ref().map(|d| (d.id.clone(), d.name.clone()));
        s.entries.clear();
        s.since = 0;
        s.generation += 1;
        s.fetching = false;
        if let Some((record, list)) = s.window {
            ui::set_list_items(list, &[], None);
            ui::set_checked(record, device.as_ref().is_some_and(|d| d.speech_log));
            ui::enable(record, device.is_some() && !s.busy);
        }
    });
    panel.set_title(&match &device {
        Some(d) => format!("What {} said", d.name),
        None => "Speech Log".into(),
    });
}

struct Window;

impl Handler for Window {
    fn command(&mut self, panel: &Panel, id: u16) {
        match id {
            RECORD => {
                let Some((record, _)) = STATE.with(|s| s.borrow().window) else {
                    return;
                };
                set_recording(ui::checked(record));
            }
            COPY => super::copy_all(panel.hwnd, &text(), "Copied the speech log."),
            SAVE => {
                let name = STATE
                    .with(|s| s.borrow().device.clone())
                    .map_or("device".into(), |(_, name)| name);
                super::save_text(panel.hwnd, &format!("{name} speech.txt"), &text());
            }
            CLEAR => {
                STATE.with(|s| {
                    let mut s = s.borrow_mut();
                    s.entries.clear();
                    if let Some((_, list)) = s.window {
                        ui::set_list_items(list, &[], None);
                    }
                });
                with_session(|session| async move {
                    session.speech_log(u64::MAX, true).await?;
                    say("Cleared.", Tone::Info);
                    Ok(())
                });
            }
            _ => {}
        }
    }

    /// Checks for new speech every half second.
    fn tick(&mut self, panel: &Panel) {
        let device = app::selected().0;
        let watched = STATE.with(|s| s.borrow().device.as_ref().map(|(id, _)| id.clone()));
        if device.as_ref().map(|d| &d.id) != watched.as_ref() {
            watch(panel);
        }
        let Some(device) = device.filter(|d| d.running) else {
            return;
        };
        let Some((since, generation)) = STATE.with(|s| {
            let mut s = s.borrow_mut();
            if s.fetching {
                return None;
            }
            s.fetching = true;
            Some((s.since, s.generation))
        }) else {
            return;
        };
        spawn(async move {
            let found = async {
                let session = session_for(device.id).await.ok()?;
                let new = session.speech_log(since, false).await.ok()?;
                Some((new, session.speech_log_on()))
            }
            .await;
            run_on_ui(move || show_new(generation, found));
        });
    }

    fn closed(&mut self) {
        STATE.with(|s| s.borrow_mut().window = None);
    }
}

fn show_new(generation: u64, found: Option<(Vec<UtteranceInfo>, bool)>) {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        if s.generation != generation {
            return;
        }
        s.fetching = false;
        let Some((new, on)) = found else { return };
        if let Some((record, list)) = s.window {
            if !s.busy {
                ui::set_checked(record, on);
            }
            for u in &new {
                ui::add_list_item(list, &line(u));
            }
        }
        if let Some(last) = new.last() {
            s.since = last.time;
        }
        s.entries.extend(new);
    });
}

fn text() -> String {
    STATE.with(|s| {
        s.borrow()
            .entries
            .iter()
            .map(|u| format!("{}  {}\n", u.clock, u.text))
            .collect()
    })
}

/// Turns the speech log on or off, which restarts the screen reader.
fn set_recording(on: bool) {
    if !app::selected().0.is_some_and(|d| d.running) {
        // Says why, and puts the checkbox back.
        STATE.with(|s| {
            if let Some((record, _)) = s.borrow().window {
                ui::set_checked(record, !on);
            }
        });
        with_session(|_| async { Ok(()) });
        return;
    }
    let set_busy = |busy: bool| {
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            s.busy = busy;
            if let Some((record, _)) = s.window {
                ui::enable(record, !busy);
            }
        })
    };
    set_busy(true);
    announce(
        if on {
            "Turning on the speech log. The screen reader restarts."
        } else {
            "Turning off the speech log."
        },
        Tone::Info,
    );
    with_session(move |session| async move {
        let result = session.set_speech_log(on).await;
        let now_on = session.speech_log_on();
        run_on_ui(move || {
            set_busy(false);
            STATE.with(|s| {
                if let Some((record, _)) = s.borrow().window {
                    ui::set_checked(record, now_on);
                }
            });
            app::with(|app| {
                app.refresh();
                app.render();
            });
        });
        say(result?, Tone::Success);
        Ok(())
    });
}
