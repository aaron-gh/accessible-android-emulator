//! Serve Devices to Phones: runs `aae serve` for AAE Remote, shows and
//! announces the pairing code, and lists paired phones.

use std::cell::RefCell;
use std::io::{BufRead, BufReader};
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
const AT_LOGIN: u16 = 1004;

#[derive(Clone, Copy)]
struct Controls {
    serve: HWND,
    at_login: HWND,
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
    /// Serving whenever this person logs in, without AAE open.
    at_login: bool,
    /// Serving now, at login or not.
    serving_elsewhere: bool,
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
    panel.text("Serves this PC's devices to AAE Remote. Each phone pairs once.");
    let serving = STATE.with(|s| s.borrow().child.is_some());
    let serve = panel.check("Serve this PC's devices to AAE Remote", SERVE, serving);
    let at_login = panel.check("Serve at login, without AAE open", AT_LOGIN, false);
    let status = panel.text("Not serving.");
    let code = panel.edit("Pairing code", "");
    ui::send(code, windows::Win32::UI::Controls::EM_SETREADONLY, 1, 0);
    panel.buttons(&[("New Pairing Code", NEW_CODE)], None);
    let phones = panel.list("Paired phones", PHONES, 6, true);
    panel.buttons(&[("Unpair", UNPAIR)], None);
    STATE.with(|s| {
        s.borrow_mut().window = Some(Controls {
            serve,
            at_login,
            status,
            code,
            phones,
        })
    });
    show_state();
    refresh();
    panel.show(Window, Some(serve));
}

fn show_state() {
    STATE.with(|s| {
        let s = s.borrow();
        let Some(c) = s.window else { return };
        ui::set_checked(c.serve, s.child.is_some() || s.serving_elsewhere);
        ui::set_checked(c.at_login, s.at_login);
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
        Some("error") => {
            let text = event["text"].as_str().unwrap_or("").to_string();
            say(text.clone(), Tone::Failure);
            STATE.with(|s| s.borrow_mut().status = text);
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

/// Runs aae.exe and reads what it printed as JSON.
fn aae_json(args: &[&str]) -> Option<Value> {
    let out = command(&aae()?).args(args).output().ok()?;
    serde_json::from_slice(&out.stdout).ok()
}

/// Whether serving at login is set up, and whether it's serving now.
fn refresh() {
    std::thread::spawn(|| {
        let status = aae_json(&["daemon", "status", "--json"]);
        run_on_ui(move || {
            STATE.with(|s| {
                let mut s = s.borrow_mut();
                s.at_login = status.as_ref().is_some_and(|v| v["installed"] == true);
                s.serving_elsewhere =
                    s.child.is_none() && status.as_ref().is_some_and(|v| v["running"] == true);
                if s.serving_elsewhere {
                    s.status = "Serving in the background, at login too.".into();
                }
            });
            show_state();
        });
    });
    load_phones();
}

/// Serves at login, or stops: run by Windows, at login and now.
fn set_at_login(wanted: bool) {
    if wanted {
        // The login server takes over from this one.
        let child = STATE.with(|s| s.borrow_mut().child.take());
        if let Some((mut child, stdin)) = child {
            drop(stdin);
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
    }
    let Some(aae) = aae() else { return };
    std::thread::spawn(move || {
        let said = command(&aae)
            .args(["daemon", if wanted { "install" } else { "uninstall" }])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();
        run_on_ui(move || {
            announce(
                if said.is_empty() {
                    "That didn't work."
                } else {
                    &said
                },
                Tone::Info,
            );
            STATE.with(|s| s.borrow_mut().code.clear());
            refresh();
        });
    });
}

/// A code for whichever server is running: this one, or the one at login.
fn new_code() {
    std::thread::spawn(|| {
        let made = aae_json(&["pair", "--json"]);
        run_on_ui(move || {
            let Some(made) = made else { return };
            received(
                serde_json::json!({"event": "code", "code": made["code"], "minutes": made["minutes"]}),
            );
        });
    });
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
                let at_login = STATE.with(|s| s.borrow().at_login);
                match (ui::checked(c.serve), at_login) {
                    (true, true) => set_at_login(true),
                    (true, false) => start(),
                    (false, true) => set_at_login(false),
                    (false, false) => stop(),
                }
            }
            AT_LOGIN => set_at_login(ui::checked(c.at_login)),
            NEW_CODE => {
                let serving = STATE.with(|s| {
                    let s = s.borrow();
                    s.child.is_some() || s.serving_elsewhere
                });
                if serving {
                    new_code();
                } else {
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
