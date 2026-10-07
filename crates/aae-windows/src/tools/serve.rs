//! Serve Devices to Phones: runs the aae command next to the app as `aae
//! serve`, so AAE Remote, the Android app, can use this PC's devices, with
//! their sound and vibrations on the phone. Shows and speaks the pairing
//! code, and lists the paired phones.

use std::cell::RefCell;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};

use serde_json::Value;
use windows::Win32::Foundation::HWND;

use crate::app::{announce, say};
use crate::panels::{self, Handler, Panel};
use crate::speech::Tone;
use crate::ui::{self, run_on_ui};

pub const KIND: &str = "serve";

const SERVE: u16 = 1000;
const NEW_CODE: u16 = 1001;
const PHONES: u16 = 1002;
const UNPAIR: u16 = 1003;

#[derive(Clone, Copy)]
struct Controls {
    serve: HWND,
    status: HWND,
    code: HWND,
    phones: HWND,
}

#[derive(serde::Deserialize, Clone)]
struct Phone {
    id: String,
    name: String,
}

#[derive(Default)]
struct State {
    window: Option<Controls>,
    child: Option<(Child, ChildStdin)>,
    status: String,
    code: String,
    phones: Vec<Phone>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State {
        status: "Not serving.".into(),
        ..Default::default()
    });
}

fn aae() -> Option<std::path::PathBuf> {
    let path = std::env::current_exe().ok()?.parent()?.join("aae.exe");
    path.is_file().then_some(path)
}

fn command(aae: &std::path::Path) -> Command {
    let mut command = Command::new(aae);
    {
        use std::os::windows::process::CommandExt;
        // No console window.
        command.creation_flags(0x0800_0000);
    }
    command
}

/// A code read a character at a time, in its groups: "A B C D, E F G H".
fn spoken(code: &str) -> String {
    code.split('-')
        .map(|group| {
            group
                .chars()
                .map(String::from)
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Opens the window, or brings it forward.
pub fn show() {
    if panels::bring_forward(KIND).is_some() {
        return;
    }
    let mut panel = Panel::new(KIND, "Serve Devices to Phones", 520, 460);
    panel.text("AAE Remote, the Android app, uses this PC's devices: their sound and vibrations play on the phone, and it sends them keys and touches. Phones on this network find the PC; each pairs once with a code.");
    let serving = STATE.with(|s| s.borrow().child.is_some());
    let serve = panel.check("Serve this PC's devices to AAE Remote", SERVE, serving);
    let status = panel.text("Not serving.");
    let code = panel.edit("Pairing code", "");
    ui::send(code, windows::Win32::UI::Controls::EM_SETREADONLY, 1, 0);
    panel.buttons(&[("New Pairing Code", NEW_CODE)], None);
    let phones = panel.list("Paired phones", PHONES, 6, true);
    panel.buttons(&[("Unpair", UNPAIR)], None);
    STATE.with(|s| {
        s.borrow_mut().window = Some(Controls {
            serve,
            status,
            code,
            phones,
        })
    });
    show_state();
    load_phones();
    panel.show(Window, Some(serve));
}

fn show_state() {
    STATE.with(|s| {
        let s = s.borrow();
        let Some(c) = s.window else { return };
        ui::set_checked(c.serve, s.child.is_some());
        panels::set_text(c.status, &s.status);
        if ui::text(c.code) != s.code {
            ui::set_text(c.code, &s.code);
        }
        let rows: Vec<String> = s.phones.iter().map(|p| p.name.clone()).collect();
        let selected = ui::list_selection(c.phones)
            .filter(|&i| i < rows.len())
            .or((!rows.is_empty()).then_some(0));
        ui::set_list_items(c.phones, &rows, selected);
    });
}

fn start() {
    let Some(aae) = aae() else {
        announce(
            "This copy of AAE doesn't include aae.exe, which serves devices. Download AAE again.",
            Tone::Failure,
        );
        return;
    };
    let child = command(&aae)
        .args(["serve", "--json"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let mut child = match child {
        Ok(child) => child,
        Err(e) => {
            announce(&format!("Couldn't start serving: {e}"), Tone::Failure);
            return;
        }
    };
    let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        return;
    };
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.child = Some((child, stdin));
        s.status = "Starting.".into();
    });
    show_state();
    // Its events, as JSON lines, until it stops.
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            let Ok(event) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            run_on_ui(move || received(event));
        }
        run_on_ui(|| {
            let was = STATE.with(|s| {
                let mut s = s.borrow_mut();
                s.code.clear();
                s.status = "Not serving.".into();
                s.child.take().is_some()
            });
            show_state();
            if was {
                announce("Serving stopped.", Tone::Info);
            }
        });
    });
}

fn received(event: Value) {
    match event["event"].as_str() {
        Some("serving") => {
            STATE.with(|s| {
                s.borrow_mut().status = format!("Serving to AAE Remote on port {}.", event["port"]);
            });
        }
        Some("code") => {
            let code = event["code"].as_str().unwrap_or("").to_string();
            let minutes = event["minutes"].as_u64().unwrap_or(10);
            say(
                format!(
                    "Pairing code: {}. It works once, for {minutes} minutes.",
                    spoken(&code)
                ),
                Tone::Info,
            );
            STATE.with(|s| s.borrow_mut().code = code);
        }
        Some("notice") => {
            let text = event["text"].as_str().unwrap_or("").to_string();
            say(text.clone(), Tone::Success);
            STATE.with(|s| s.borrow_mut().status = text);
            load_phones();
        }
        _ => {}
    }
    show_state();
}

/// Stops serving: closing its input stops it.
fn stop() {
    let child = STATE.with(|s| s.borrow_mut().child.take());
    if let Some((mut child, stdin)) = child {
        drop(stdin);
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        announce("Stopped serving.", Tone::Info);
    }
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.code.clear();
        s.status = "Not serving.".into();
    });
    show_state();
}

fn load_phones() {
    let Some(aae) = aae() else { return };
    std::thread::spawn(move || {
        let phones: Vec<Phone> = command(&aae)
            .args(["phones", "--json"])
            .output()
            .ok()
            .and_then(|out| serde_json::from_slice(&out.stdout).ok())
            .unwrap_or_default();
        run_on_ui(move || {
            STATE.with(|s| s.borrow_mut().phones = phones);
            show_state();
        });
    });
}

struct Window;

impl Handler for Window {
    fn command(&mut self, _panel: &Panel, id: u16) {
        let Some(c) = STATE.with(|s| s.borrow().window) else {
            return;
        };
        match id {
            SERVE => {
                if ui::checked(c.serve) {
                    start();
                } else {
                    stop();
                }
            }
            NEW_CODE => {
                let sent = STATE.with(|s| {
                    s.borrow_mut().child.as_mut().map(|(_, stdin)| {
                        stdin.write_all(b"p\n").and_then(|_| stdin.flush()).is_ok()
                    })
                });
                if sent != Some(true) {
                    announce("Turn on serving first.", Tone::Failure);
                }
            }
            UNPAIR => {
                let phone = STATE.with(|s| {
                    let s = s.borrow();
                    ui::list_selection(c.phones).and_then(|i| s.phones.get(i).cloned())
                });
                let (Some(phone), Some(aae)) = (phone, aae()) else {
                    announce("Select a phone first.", Tone::Failure);
                    return;
                };
                let _ = command(&aae)
                    .args(["phones", "--unpair", &phone.id])
                    .output();
                announce(&format!("Unpaired {}.", phone.name), Tone::Info);
                load_phones();
            }
            _ => {}
        }
    }

    fn closed(&mut self) {
        // Serving goes on with the window closed.
        STATE.with(|s| s.borrow_mut().window = None);
    }
}
