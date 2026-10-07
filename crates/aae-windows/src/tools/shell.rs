//! The Shell window: runs commands in the device's shell and shows what they
//! printed, as a transcript that can be read line by line.

use std::cell::RefCell;

use windows::Win32::Foundation::HWND;

use crate::app::{self, announce, say, with_session};
use crate::panels::{self, Handler, Panel};
use crate::speech::Tone;
use crate::ui::{self, run_on_ui};

pub const KIND: &str = "shell";

const RUN: u16 = 1000;
const RECENT: u16 = 1001;
const COPY: u16 = 1002;
const SAVE: u16 = 1003;
const CLEAR: u16 = 1004;

/// What the Shell remembers while AAE is open, so closing the window keeps it.
#[derive(Default)]
struct State {
    transcript: String,
    /// Commands run, oldest first.
    history: Vec<String>,
    running: bool,
    window: Option<Controls>,
}

#[derive(Clone, Copy)]
struct Controls {
    command: HWND,
    recent: HWND,
    run: HWND,
    output: HWND,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

fn title() -> String {
    match app::selected().0 {
        Some(device) => format!("Shell of {}", device.name),
        None => "Shell".into(),
    }
}

/// Opens the window, or brings it forward.
pub fn show() {
    if let Some(window) = panels::bring_forward(KIND) {
        ui::set_text(window, &title());
        if let Some(c) = STATE.with(|s| s.borrow().window) {
            ui::focus(c.command);
        }
        return;
    }
    let mut panel = Panel::new(KIND, &title(), 640, 520);
    let command = panel.edit("Command", "");
    let recent = panel.choice("Recent commands", RECENT, &[], 0);
    let run = panel.buttons(&[("Run", RUN)], Some(RUN))[0];
    panel.text("Runs as the shell user, with a two-minute limit.");
    let output = panel.area("Output", "", true, 16, true);
    panel.buttons(
        &[("Copy All", COPY), ("Save…", SAVE), ("Clear", CLEAR)],
        None,
    );
    let controls = Controls {
        command,
        recent,
        run,
        output,
    };
    STATE.with(|s| s.borrow_mut().window = Some(controls));
    show_state();
    panel.show(Window, Some(command));
}

/// Shows the transcript, the recent commands, and whether one is running.
fn show_state() {
    STATE.with(|s| {
        let s = s.borrow();
        let Some(c) = s.window else { return };
        ui::set_transcript(c.output, &s.transcript);
        let recent: Vec<String> = s.history.iter().rev().cloned().collect();
        ui::set_combo_items(c.recent, &recent, 0);
        ui::enable(c.recent, !recent.is_empty());
        ui::set_text(c.run, if s.running { "Running…" } else { "Run" });
        ui::enable(c.run, !s.running);
    });
}

struct Window;

impl Handler for Window {
    fn command(&mut self, panel: &Panel, id: u16) {
        let Some(c) = STATE.with(|s| s.borrow().window) else {
            return;
        };
        match id {
            RUN => run(&ui::text(c.command)),
            RECENT => {
                let recent = STATE.with(|s| {
                    let s = s.borrow();
                    let index = ui::combo_selection(c.recent)?;
                    s.history.iter().rev().nth(index).cloned()
                });
                if let Some(command) = recent {
                    ui::set_text(c.command, &command);
                }
            }
            COPY => {
                let text = STATE.with(|s| s.borrow().transcript.clone());
                super::copy_all(panel.hwnd, &text, "Copied the output.");
            }
            SAVE => {
                let text = STATE.with(|s| s.borrow().transcript.clone());
                let name = app::selected().0.map_or("device".into(), |d| d.name);
                super::save_text(panel.hwnd, &format!("{name} shell.txt"), &text);
            }
            CLEAR => {
                STATE.with(|s| s.borrow_mut().transcript.clear());
                show_state();
                announce("Cleared.", Tone::Info);
            }
            _ => {}
        }
    }

    fn closed(&mut self) {
        STATE.with(|s| s.borrow_mut().window = None);
    }
}

/// Runs a command and adds it, with what it printed, to the transcript.
fn run(command: &str) {
    let command = command.trim().to_string();
    if command.is_empty() || STATE.with(|s| s.borrow().running) {
        return;
    }
    if !super::running() {
        return;
    }
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.history.retain(|c| *c != command);
        s.history.push(command.clone());
        s.running = true;
        if let Some(c) = s.window {
            ui::set_text(c.command, "");
        }
    });
    show_state();
    with_session(move |session| async move {
        let result = session.run_command(command.clone()).await;
        let (entry, spoken) = match &result {
            Ok(result) => {
                let mut output = result.output.clone();
                if !output.is_empty() && !output.ends_with('\n') {
                    output.push('\n');
                }
                let lines = output.lines().count();
                let printed = match lines {
                    0 => "no output".to_string(),
                    1 => "1 line".to_string(),
                    n => format!("{n} lines"),
                };
                if result.status == 0 {
                    (
                        format!("$ {command}\n{output}"),
                        Some((format!("Done, {printed}."), Tone::Success)),
                    )
                } else {
                    (
                        format!("$ {command}\n{output}Exit status {}.\n", result.status),
                        Some((
                            format!("Failed with exit status {}, {printed}.", result.status),
                            Tone::Failure,
                        )),
                    )
                }
            }
            Err(e) => (format!("$ {command}\n{e}\n"), None),
        };
        run_on_ui(move || {
            STATE.with(|s| {
                let mut s = s.borrow_mut();
                s.transcript.push_str(&entry);
                s.running = false;
            });
            show_state();
        });
        if let Some((text, tone)) = spoken {
            say(text, tone);
        }
        result.map(|_| ())
    });
}
