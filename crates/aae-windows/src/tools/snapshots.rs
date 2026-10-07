//! The Snapshots window: the device's saved states it can go back to, each
//! with a name, when it was taken, and notes.

use std::cell::RefCell;

use aae_ffi::SnapshotInfo;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::{VK_F5, VK_R, VK_S};

use crate::app::{self, announce, say, session_for, spawn, with_session};
use crate::forms::{self, Field, Form, Role};
use crate::panels::{self, Handler, Panel};
use crate::speech::Tone;
use crate::ui::{self, run_on_ui};

pub const KIND: &str = "snapshots";

const LIST: u16 = 1000;
const SAVE: u16 = 1001;
const RESTORE: u16 = 1002;
const EDIT: u16 = 1003;
const DELETE: u16 = 1004;
const REFRESH: u16 = 1005;

#[derive(Clone, Copy)]
struct Controls {
    status: HWND,
    list: HWND,
}

#[derive(Default)]
struct State {
    window: Option<Controls>,
    snapshots: Vec<SnapshotInfo>,
    device: Option<(String, bool)>,
    generation: u64,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

fn row(snapshot: &SnapshotInfo) -> String {
    let mut parts = Vec::new();
    if let Some(taken) = &snapshot.taken {
        parts.push(format!("Taken {taken}"));
    }
    parts.push(snapshot.size.clone());
    if snapshot.loaded {
        parts.push("restored last".into());
    }
    if !snapshot.compatible {
        parts.push("this emulator can't restore it".into());
    }
    let mut text = format!("{}. {}.", snapshot.name, parts.join(", "));
    if !snapshot.notes.is_empty() {
        text = format!("{text} {}", snapshot.notes.replace(['\r', '\n'], " "));
    }
    text
}

/// Opens the window, or brings it forward.
pub fn show() {
    if panels::bring_forward(KIND).is_some() {
        return;
    }
    let mut panel = Panel::new(KIND, "Snapshots", 580, 440);
    panel.text("A snapshot saves everything on the device, so you can come back to it later, such as before testing a sign-in.");
    let status = panel.text("Reading…");
    let list = panel.list("Snapshots", LIST, 10, true);
    panel.buttons(
        &[
            ("Save Snapshot…", SAVE),
            ("Restore…", RESTORE),
            ("Edit…", EDIT),
            ("Delete…", DELETE),
            ("Refresh", REFRESH),
        ],
        None,
    );
    panel.shortcut(VK_S.0, true, SAVE);
    panel.shortcut(VK_F5.0, false, REFRESH);
    panel.shortcut(VK_R.0, true, REFRESH);
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.window = Some(Controls { status, list });
        s.device = None;
    });
    load(&panel);
    panel.every(1000);
    panel.show(Window, Some(list));
}

/// Reads the selected device's snapshots.
fn load(panel: &Panel) {
    let device = app::selected().0;
    panel.set_title(&match &device {
        Some(d) => format!("Snapshots of {}", d.name),
        None => "Snapshots".into(),
    });
    let Some(c) = STATE.with(|s| s.borrow().window) else {
        return;
    };
    let generation = STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.device = device.as_ref().map(|d| (d.id.clone(), d.running));
        s.generation += 1;
        s.generation
    });
    let Some(device) = device.filter(|d| d.running) else {
        STATE.with(|s| s.borrow_mut().snapshots.clear());
        ui::set_list_items(c.list, &[], None);
        ui::set_text(c.status, "Start the device to see its snapshots.");
        return;
    };
    ui::set_text(c.status, "Reading…");
    spawn(async move {
        let result = async { session_for(device.id).await?.snapshots().await }.await;
        run_on_ui(move || {
            STATE.with(|s| {
                let mut s = s.borrow_mut();
                if s.generation != generation {
                    return;
                }
                let Some(c) = s.window else { return };
                match result {
                    Ok(snapshots) => {
                        let selected = ui::list_selection(c.list)
                            .and_then(|i| s.snapshots.get(i))
                            .map(|snapshot| snapshot.id.clone());
                        let rows: Vec<String> = snapshots.iter().map(row).collect();
                        let index = selected
                            .and_then(|id| snapshots.iter().position(|x| x.id == id))
                            .or((!snapshots.is_empty()).then_some(0));
                        ui::set_list_items(c.list, &rows, index);
                        let status = match snapshots.len() {
                            0 => "No snapshots yet.".to_string(),
                            1 => "1 snapshot.".to_string(),
                            n => format!("{n} snapshots."),
                        };
                        ui::set_text(c.status, &status);
                        s.snapshots = snapshots;
                    }
                    Err(e) => ui::set_text(c.status, &e.to_string()),
                }
            })
        });
    });
}

fn selected_snapshot() -> Option<SnapshotInfo> {
    STATE.with(|s| {
        let s = s.borrow();
        let c = s.window?;
        s.snapshots.get(ui::list_selection(c.list)?).cloned()
    })
}

/// Reads the snapshots again, once something has changed them.
fn reload() {
    run_on_ui(|| panels::press(KIND, REFRESH));
}

struct Window;

impl Handler for Window {
    fn command(&mut self, panel: &Panel, id: u16) {
        match id {
            REFRESH => load(panel),
            SAVE => {
                if super::running() {
                    edit(panel.hwnd, None);
                }
            }
            RESTORE | EDIT | DELETE => {
                if !super::running() {
                    return;
                }
                let Some(snapshot) = selected_snapshot() else {
                    announce("Select a snapshot first.", Tone::Failure);
                    return;
                };
                match id {
                    RESTORE => restore(panel.hwnd, snapshot),
                    EDIT => edit(panel.hwnd, Some(snapshot)),
                    _ => delete(panel.hwnd, snapshot),
                }
            }
            _ => {}
        }
    }

    /// Reads the snapshots again when another device is selected, or it
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
            s.snapshots.clear();
        });
    }
}

/// Asks for a name and notes, then saves a new snapshot, or the changes to
/// one.
fn edit(owner: HWND, snapshot: Option<SnapshotInfo>) {
    let title = match &snapshot {
        Some(s) => format!("Edit {}", s.name),
        None => "Save a Snapshot".into(),
    };
    let action = if snapshot.is_some() {
        "Save Changes"
    } else {
        "Save"
    };
    let answer = forms::run(
        owner,
        Form::new(&title)
            .field(Field::Edit {
                label: "Name".into(),
                value: snapshot
                    .as_ref()
                    .map(|s| s.name.clone())
                    .unwrap_or_default(),
            })
            .field(Field::Area {
                label: "Notes".into(),
                value: snapshot
                    .as_ref()
                    .map(|s| s.notes.clone())
                    .unwrap_or_default(),
                read_only: false,
                lines: 4,
            })
            .button(action, 1, Role::Default)
            .button("Cancel", 0, Role::Cancel),
    );
    if answer.button != 1 {
        return;
    }
    let name = answer.values[0].text().trim().to_string();
    let notes = answer.values[1].text().replace("\r\n", "\n");
    if name.is_empty() {
        announce("Nothing saved: a snapshot needs a name.", Tone::Failure);
        return;
    }
    match snapshot {
        Some(snapshot) => with_session(move |session| async move {
            session
                .update_snapshot(snapshot.id, name.clone(), notes)
                .await?;
            say(format!("Saved the changes to {name}."), Tone::Success);
            reload();
            Ok(())
        }),
        None => {
            announce(&format!("Saving snapshot {name}."), Tone::Info);
            with_session(move |session| async move {
                session.save_snapshot(name.clone(), notes).await?;
                say(format!("Saved snapshot {name}."), Tone::Success);
                reload();
                Ok(())
            });
        }
    }
}

/// Asks, then puts the device back as it was in a snapshot.
fn restore(owner: HWND, snapshot: SnapshotInfo) {
    if !snapshot.compatible {
        announce(
            &format!("This emulator can't restore {}.", snapshot.name),
            Tone::Failure,
        );
        return;
    }
    if !forms::confirm(
        owner,
        &format!("Restore {}?", snapshot.name),
        "The device goes back to how it was when this snapshot was taken. Anything since then is lost, unless you save a snapshot of it first.",
        "Restore",
    ) {
        return;
    }
    announce(&format!("Restoring {}.", snapshot.name), Tone::Info);
    with_session(move |session| async move {
        session.load_snapshot(snapshot.id).await?;
        say(format!("Restored {}.", snapshot.name), Tone::Success);
        reload();
        Ok(())
    });
}

/// Asks, then deletes a snapshot.
fn delete(owner: HWND, snapshot: SnapshotInfo) {
    if !forms::confirm(
        owner,
        &format!("Delete {}?", snapshot.name),
        &format!("This frees {}. It can't be undone.", snapshot.size),
        "Delete",
    ) {
        return;
    }
    with_session(move |session| async move {
        session.delete_snapshot(snapshot.id).await?;
        say(format!("Deleted {}.", snapshot.name), Tone::Success);
        reload();
        Ok(())
    });
}
