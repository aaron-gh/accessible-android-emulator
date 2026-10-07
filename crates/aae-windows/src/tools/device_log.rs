//! The Device Log window: the device log, one line per row, filtered by app,
//! tag, level and text. New lines appear as they're written.

use std::cell::RefCell;
use std::sync::Arc;
use std::time::{Duration, Instant};

use aae_ffi::{LogEntryInfo, LogFilter, LogLevel, Session};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::GetFocus;
use windows::Win32::UI::WindowsAndMessaging::{LB_DELETESTRING, LB_SETCURSEL, WM_SETREDRAW};

use crate::app::{self, announce, session_for, spawn};
use crate::panels::{self, Handler, Panel};
use crate::speech::Tone;
use crate::ui::{self, run_on_ui};

pub const KIND: &str = "device-log";

/// The most lines the window shows at once.
const LIMIT: usize = 5000;

const APP: u16 = 1000;
const LEVEL: u16 = 1001;
const PAUSE: u16 = 1002;
const ANNOUNCE: u16 = 1003;
const LIST: u16 = 1004;
const COPY_LINE: u16 = 1005;
const COPY_ALL: u16 = 1006;
const SAVE: u16 = 1007;
const CLEAR: u16 = 1008;

const LEVELS: [(&str, Option<LogLevel>); 6] = [
    ("All levels", None),
    ("Debug and above", Some(LogLevel::Debug)),
    ("Info and above", Some(LogLevel::Info)),
    ("Warnings and errors", Some(LogLevel::Warning)),
    ("Errors", Some(LogLevel::Error)),
    ("Fatal errors", Some(LogLevel::Fatal)),
];

#[derive(Clone, Copy)]
struct Controls {
    app: HWND,
    level: HWND,
    tag: HWND,
    search: HWND,
    pause: HWND,
    announce: HWND,
    status: HWND,
    list: HWND,
}

/// The filter as chosen: app, level, tag and text.
#[derive(Clone, PartialEq, Default)]
struct Chosen {
    app: String,
    level: usize,
    tag: String,
    search: String,
}

impl Chosen {
    fn filter(&self) -> LogFilter {
        let value = |s: &str| Some(s.trim().to_string()).filter(|s| !s.is_empty());
        LogFilter {
            process: value(&self.app),
            tag: value(&self.tag),
            level: LEVELS[self.level].1,
            text: value(&self.search),
        }
    }
}

#[derive(Default)]
struct State {
    window: Option<Controls>,
    /// The device whose log is shown, by id.
    device: Option<String>,
    session: Option<Arc<Session>>,
    /// A session is being opened, so ticks don't ask for another.
    waiting: bool,
    /// Counts the times the log was started afresh, so a late session for an
    /// earlier device is dropped.
    generation: u64,
    entries: Vec<LogEntryInfo>,
    /// The newest line read.
    seen: u64,
    chosen: Chosen,
    processes: Vec<String>,
    status: String,
    unannounced: Vec<LogEntryInfo>,
    last_announcement: Option<Instant>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

fn row(entry: &LogEntryInfo) -> String {
    let process = entry.process.as_deref().unwrap_or("Unknown process");
    format!("{}. {process}, at {}", entry.spoken, entry.clock)
}

/// Opens the window, or brings it forward.
pub fn show() {
    if panels::bring_forward(KIND).is_some() {
        return;
    }
    let mut panel = Panel::new(KIND, "Device Log", 700, 560);
    let app = panel.choice("App", APP, &["All apps".into()], 0);
    let levels: Vec<String> = LEVELS.iter().map(|(name, _)| name.to_string()).collect();
    let level = panel.choice("Level", LEVEL, &levels, 0);
    let tag = panel.edit("Tag", "");
    let search = panel.edit("Search", "");
    let pause = panel.check("Pause", PAUSE, false);
    let announce = panel.check("Announce new errors in this list", ANNOUNCE, false);
    let status = panel.text("0 lines");
    let list = panel.list("Log lines", LIST, 14, true);
    panel.buttons(
        &[
            ("Copy Line", COPY_LINE),
            ("Copy All Shown", COPY_ALL),
            ("Save…", SAVE),
            ("Clear", CLEAR),
        ],
        None,
    );
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.window = Some(Controls {
            app,
            level,
            tag,
            search,
            pause,
            announce,
            status,
            list,
        });
        s.chosen = Chosen::default();
        s.processes.clear();
    });
    watch(&panel);
    panel.every(500);
    panel.show(Window, Some(list));
}

/// Starts reading the selected device's log, from the start.
fn watch(panel: &Panel) {
    stop();
    let device = app::selected().0;
    let generation = STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.device = device.as_ref().map(|d| d.id.clone());
        s.waiting = device.as_ref().is_some_and(|d| d.running);
        s.generation += 1;
        s.generation
    });
    reload();
    panel.set_title(&match &device {
        Some(d) => format!("Log of {}", d.name),
        None => "Device Log".into(),
    });
    let Some(device) = device.filter(|d| d.running) else {
        return;
    };
    spawn(async move {
        let session = session_for(device.id).await.ok();
        run_on_ui(move || {
            STATE.with(|s| {
                let mut s = s.borrow_mut();
                if s.generation != generation || s.window.is_none() {
                    return;
                }
                s.waiting = false;
                if let Some(session) = session {
                    session.start_logs();
                    s.session = Some(session);
                }
            })
        });
    });
}

/// Stops reading the log, which forgets the lines read.
fn stop() {
    if let Some(session) = STATE.with(|s| s.borrow_mut().session.take()) {
        session.stop_logs();
    }
}

/// Shows the lines matching the filter again, from the start.
fn reload() {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.entries.clear();
        s.unannounced.clear();
        s.seen = 0;
        if let Some(c) = s.window {
            ui::set_list_items(c.list, &[], None);
        }
    });
}

struct Window;

impl Handler for Window {
    fn command(&mut self, panel: &Panel, id: u16) {
        let lines = |all: bool| {
            STATE.with(|s| {
                let s = s.borrow();
                let selected = s.window.and_then(|c| ui::list_selection(c.list));
                s.entries
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| all || Some(*i) == selected)
                    .map(|(_, e)| format!("{}\n", e.line))
                    .collect::<String>()
            })
        };
        match id {
            COPY_LINE => super::copy_all(panel.hwnd, &lines(false), "Copied."),
            COPY_ALL => super::copy_all(panel.hwnd, &lines(true), "Copied."),
            SAVE => {
                let name = app::selected().0.map_or("device".into(), |d| d.name);
                super::save_text(panel.hwnd, &format!("{name} log.txt"), &lines(true));
            }
            CLEAR => {
                reload();
                if let Some(session) = STATE.with(|s| s.borrow().session.clone()) {
                    session.clear_logs();
                }
                announce("Cleared.", Tone::Info);
            }
            _ => {}
        }
    }

    /// Reads what's new every half second.
    fn tick(&mut self, panel: &Panel) {
        let device = app::selected().0;
        let (watched, connected) = STATE.with(|s| {
            let s = s.borrow();
            (s.device.clone(), s.session.is_some() || s.waiting)
        });
        // Another device was selected, or this one has started.
        if device.as_ref().map(|d| &d.id) != watched.as_ref()
            || !connected && device.as_ref().is_some_and(|d| d.running)
        {
            watch(panel);
        }
        let Some(c) = STATE.with(|s| s.borrow().window) else {
            return;
        };
        let chosen = Chosen {
            app: STATE.with(|s| {
                let s = s.borrow();
                ui::combo_selection(c.app)
                    .and_then(|i| i.checked_sub(1))
                    .and_then(|i| app_choices(&s).get(i).cloned())
                    .unwrap_or_default()
            }),
            level: ui::combo_selection(c.level).unwrap_or(0),
            tag: ui::text(c.tag),
            search: ui::text(c.search),
        };
        if STATE.with(|s| s.borrow().chosen != chosen) {
            STATE.with(|s| s.borrow_mut().chosen = chosen);
            reload();
        }
        if !ui::checked(c.pause) {
            read_new(c);
        }
        announce_errors();
    }

    fn closed(&mut self) {
        stop();
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            s.window = None;
            s.device = None;
            s.entries.clear();
        });
    }
}

/// The apps to choose from, keeping the chosen one even before it logs.
fn app_choices(s: &State) -> Vec<String> {
    let mut apps = s.processes.clone();
    if !s.chosen.app.is_empty() && !apps.contains(&s.chosen.app) {
        apps.insert(0, s.chosen.app.clone());
    }
    apps
}

fn read_new(c: Controls) {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        let Some(session) = s.session.clone() else {
            return;
        };
        let latest = session.log_latest();
        let new = session.log_entries(s.seen, s.chosen.filter(), LIMIT as u32);
        // Don't announce lines present when the window opened or the filter
        // changed.
        let news = s.seen != 0;
        s.seen = latest.max(new.last().map_or(0, |e| e.seq));
        if !new.is_empty() {
            ui::send(c.list, WM_SETREDRAW, 0, 0);
            for entry in &new {
                ui::add_list_item(c.list, &row(entry));
            }
            s.entries.extend(new.iter().cloned());
            let excess = s.entries.len().saturating_sub(LIMIT);
            if excess > 0 {
                let selected = ui::list_selection(c.list);
                s.entries.drain(..excess);
                for _ in 0..excess {
                    ui::send(c.list, LB_DELETESTRING, 0, 0);
                }
                if let Some(index) = selected.and_then(|i| i.checked_sub(excess)) {
                    ui::send(c.list, LB_SETCURSEL, index, 0);
                }
            }
            ui::send(c.list, WM_SETREDRAW, 1, 0);
            if news && ui::checked(c.announce) {
                s.unannounced.extend(
                    new.into_iter()
                        .filter(|e| matches!(e.level, LogLevel::Error | LogLevel::Fatal)),
                );
            }
        }
        // The apps that have logged, unless the list is in use.
        let processes = session.log_processes();
        if processes != s.processes && unsafe { GetFocus() } != c.app {
            s.processes = processes;
            let apps = app_choices(&s);
            let mut items = vec!["All apps".to_string()];
            items.extend(apps.iter().cloned());
            let selected = apps
                .iter()
                .position(|a| *a == s.chosen.app)
                .map_or(0, |i| i + 1);
            ui::set_combo_items(c.app, &items, selected);
        }
        let count = match s.entries.len() {
            1 => "1 line".to_string(),
            n => format!("{n} lines"),
        };
        let status = match session.log_problem() {
            Some(problem) => format!("{count}. {problem}"),
            None => count,
        };
        if status != s.status {
            panels::set_text(c.status, &status);
            s.status = status;
        }
    });
}

/// Says new errors, at most once every two seconds, so a burst of them
/// doesn't bury everything else.
fn announce_errors() {
    let spoken = STATE.with(|s| {
        let mut s = s.borrow_mut();
        let first = s.unannounced.first()?.spoken.clone();
        if s.last_announcement
            .is_some_and(|t| t.elapsed() < Duration::from_secs(2))
        {
            return None;
        }
        let more = s.unannounced.len() - 1;
        s.unannounced.clear();
        s.last_announcement = Some(Instant::now());
        Some(match more {
            0 => first,
            1 => format!("{first}. And 1 more error."),
            n => format!("{first}. And {n} more errors."),
        })
    });
    if let Some(text) = spoken {
        announce(&text, Tone::Failure);
    }
}
