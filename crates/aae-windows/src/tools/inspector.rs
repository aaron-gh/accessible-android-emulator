//! The Accessibility Inspector: the screen's accessibility tree as a screen
//! reader sees it, the details of the selected element, and the problems
//! found. Read it as a tree, or as a flat list in reading order with each
//! element's level, which needs no expanding.

use std::cell::RefCell;
use std::collections::HashMap;

use aae_ffi::Inspection;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Controls::HTREEITEM;
use windows::Win32::UI::Input::KeyboardAndMouse::{VK_F5, VK_R};
use windows::Win32::UI::WindowsAndMessaging::LB_SETCURSEL;

use crate::app::{self, announce, say, with_session};
use crate::panels::{self, Handler, Panel};
use crate::speech::Tone;
use crate::ui::{self, run_on_ui};

pub const KIND: &str = "inspector";

const REFRESH: u16 = 1000;
const COPY: u16 = 1001;
const SAVE: u16 = 1002;
const VIEW: u16 = 1003;
const TREE: u16 = 1004;
const FLAT: u16 = 1005;
const PROBLEMS: u16 = 1006;

#[derive(Clone, Copy)]
struct Controls {
    refresh: HWND,
    status: HWND,
    view: HWND,
    tree: HWND,
    flat: HWND,
    details: HWND,
    problems: HWND,
}

#[derive(Default)]
struct State {
    inspection: Option<Inspection>,
    /// The name of the device inspected.
    device: Option<String>,
    inspecting: bool,
    window: Option<Controls>,
    /// The elements in reading order: their row in the inspection, and
    /// their item in the tree.
    order: Vec<(usize, HTREEITEM)>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// Opens the window, or brings it forward.
pub fn show() {
    if panels::bring_forward(KIND).is_some() {
        return;
    }
    let mut panel = Panel::new(KIND, "Accessibility Inspector", 660, 640);
    let refresh = panel.buttons(
        &[
            ("Refresh", REFRESH),
            ("Copy as Text", COPY),
            ("Save…", SAVE),
        ],
        None,
    )[0];
    panel.shortcut(VK_F5.0, false, REFRESH);
    panel.shortcut(VK_R.0, true, REFRESH);
    let status =
        panel.text("Choose Refresh, or press F5, to read the screen of the selected device.");
    let view = panel.choice("View", VIEW, &["Tree".into(), "Flat list".into()], 0);
    let tree = panel.tree("Screen elements", TREE, 12, true);
    let flat = ui::list_box(panel.hwnd, FLAT);
    ui::show(flat, false);
    panel.twin(flat);
    let details = panel.area("Details of the selected element", "", true, 6, false);
    let problems = panel.list("Problems", PROBLEMS, 4, false);
    STATE.with(|s| {
        s.borrow_mut().window = Some(Controls {
            refresh,
            status,
            view,
            tree,
            flat,
            details,
            problems,
        })
    });
    show_inspection();
    let inspected = STATE.with(|s| s.borrow().inspection.is_some());
    panel.show(Window, Some(if inspected { tree } else { refresh }));
    if !inspected {
        inspect();
    }
}

/// Reads the selected device's screen and checks it.
fn inspect() {
    if STATE.with(|s| s.borrow().inspecting) || !super::running() {
        return;
    }
    let name = app::selected().0.map(|d| d.name);
    set_inspecting(true);
    with_session(move |session| async move {
        let result = session.inspect().await;
        let inspection = result.as_ref().ok().cloned();
        run_on_ui(move || {
            if let Some(inspection) = inspection {
                STATE.with(|s| {
                    let mut s = s.borrow_mut();
                    s.inspection = Some(inspection);
                    s.device = name;
                });
                show_inspection();
            }
            set_inspecting(false);
        });
        let inspection = result?;
        let problems = match inspection.issues.len() {
            0 => "no problems".to_string(),
            1 => "1 problem".to_string(),
            n => format!("{n} problems"),
        };
        let tone = if inspection.issues.is_empty() {
            Tone::Success
        } else {
            Tone::Info
        };
        say(
            format!("Read {} elements, {problems}.", inspection.rows.len()),
            tone,
        );
        Ok(())
    });
}

fn set_inspecting(on: bool) {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.inspecting = on;
        if let Some(c) = s.window {
            ui::enable(c.refresh, !on);
            if on {
                ui::set_text(c.status, "Reading the screen.");
            }
        }
    });
}

/// Shows the inspection in the window: the tree, the flat list and the problems.
fn show_inspection() {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        let Some(c) = s.window else { return };
        let Some(inspection) = s.inspection.clone() else {
            return;
        };
        let title = match &s.device {
            Some(name) => format!("Accessibility of {name}"),
            None => "Accessibility Inspector".into(),
        };
        if let Some(window) = panels::open_window(KIND) {
            ui::set_text(window, &title);
        }
        let issues = match inspection.issues.len() {
            0 => "No problems found".to_string(),
            1 => "1 problem".to_string(),
            n => format!("{n} problems"),
        };
        ui::set_text(
            c.status,
            &format!(
                "{} elements, {}.",
                inspection.rows.len(),
                issues.to_lowercase()
            ),
        );

        // The rows, inside their parents, in reading order.
        let mut children: HashMap<Option<u32>, Vec<usize>> = HashMap::new();
        for (i, row) in inspection.rows.iter().enumerate() {
            children.entry(row.parent).or_default().push(i);
        }
        let mut order = Vec::new();
        let mut flat = Vec::new();
        ui::tree_clear(c.tree);
        // Depth first, so each element follows the one it's inside.
        let mut stack: Vec<(usize, Option<HTREEITEM>, usize)> = children
            .get(&None)
            .into_iter()
            .flatten()
            .rev()
            .map(|&row| (row, None, 0))
            .collect();
        while let Some((row, parent, depth)) = stack.pop() {
            let r = &inspection.rows[row];
            let item = ui::tree_add(c.tree, parent, &r.summary, row as isize);
            order.push((row, item));
            flat.push(format!("Level {}, {}", depth + 1, r.summary));
            for &child in children.get(&Some(r.index)).into_iter().flatten().rev() {
                stack.push((child, Some(item), depth + 1));
            }
        }
        // Open, so every element can be reached with the arrow keys.
        for &(_, item) in &order {
            ui::tree_expand(c.tree, item);
        }
        ui::set_list_items(c.flat, &flat, None);
        if let Some(&(_, first)) = order.first() {
            ui::tree_select(c.tree, first);
            ui::send(c.flat, LB_SETCURSEL, 0, 0);
        }
        s.order = order;

        let problems: Vec<String> = if inspection.issues.is_empty() {
            vec!["No problems found".into()]
        } else {
            inspection
                .issues
                .iter()
                .map(|issue| {
                    let kind = if issue.error { "Error" } else { "Warning" };
                    let id = issue
                        .id
                        .as_ref()
                        .map(|id| format!(" ({id})"))
                        .unwrap_or_default();
                    format!("{kind}: {} Element: {}{id}", issue.message, issue.element)
                })
                .collect()
        };
        ui::set_list_items(c.problems, &problems, None);
    });
    show_details();
}

/// Shows the selected element's details.
fn show_details() {
    STATE.with(|s| {
        // Changing the tree reports its selection changing, while the state
        // is being changed.
        let Ok(s) = s.try_borrow() else { return };
        let (Some(c), Some(inspection)) = (s.window, s.inspection.as_ref()) else {
            return;
        };
        let row = if flat_view(c) {
            ui::list_selection(c.flat)
                .and_then(|i| s.order.get(i))
                .map(|(row, _)| *row)
        } else {
            ui::tree_selection(c.tree).map(|row| row as usize)
        };
        let text = match row.and_then(|row| inspection.rows.get(row)) {
            Some(row) => std::iter::once(row.summary.as_str())
                .chain(row.details.iter().map(String::as_str))
                .collect::<Vec<_>>()
                .join("\r\n"),
            None => "Select an element to see its details.".into(),
        };
        ui::set_text(c.details, &text);
    });
}

fn flat_view(c: Controls) -> bool {
    ui::combo_selection(c.view) == Some(1)
}

struct Window;

impl Handler for Window {
    fn command(&mut self, panel: &Panel, id: u16) {
        let Some(c) = STATE.with(|s| s.try_borrow().ok().and_then(|s| s.window)) else {
            return;
        };
        match id {
            REFRESH => inspect(),
            COPY => match STATE.with(|s| s.borrow().inspection.as_ref().map(|i| i.text.clone())) {
                Some(text) => {
                    super::copy_all(panel.hwnd, &text, "Copied the screen's elements as text.")
                }
                None => announce("Read the screen first.", Tone::Failure),
            },
            SAVE => save(panel.hwnd),
            VIEW => {
                // The same element stays selected in the other view.
                let flat = flat_view(c);
                STATE.with(|s| {
                    let s = s.borrow();
                    if flat {
                        let row = ui::tree_selection(c.tree).map(|r| r as usize);
                        if let Some(i) = s.order.iter().position(|(r, _)| Some(*r) == row) {
                            ui::send(c.flat, LB_SETCURSEL, i, 0);
                        }
                    } else if let Some(&(_, item)) =
                        ui::list_selection(c.flat).and_then(|i| s.order.get(i))
                    {
                        ui::tree_select(c.tree, item);
                    }
                });
                ui::show(c.tree, !flat);
                ui::show(c.flat, flat);
                show_details();
            }
            TREE | FLAT => show_details(),
            _ => {}
        }
    }

    fn closed(&mut self) {
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            s.window = None;
            s.order.clear();
        });
    }
}

/// Saves the inspection as JSON, with every property, or as text.
fn save(owner: HWND) {
    let Some((inspection, device)) = STATE.with(|s| {
        let s = s.borrow();
        Some((s.inspection.clone()?, s.device.clone()))
    }) else {
        announce("Read the screen first.", Tone::Failure);
        return;
    };
    let name = format!(
        "{} accessibility.json",
        device.as_deref().unwrap_or("screen")
    );
    let Some(path) = ui::save_file(
        owner,
        "Save as JSON, with every property, or as text",
        &name,
        &[("JSON, with every property", "*.json"), ("Text", "*.txt")],
        "json",
    ) else {
        return;
    };
    let contents = if path.to_lowercase().ends_with(".json") {
        inspection.json
    } else {
        inspection.text.replace('\n', "\r\n")
    };
    match std::fs::write(&path, contents) {
        Ok(()) => announce(&format!("Saved {}.", app::file_name(&path)), Tone::Success),
        Err(e) => announce(&format!("Couldn't save: {e}"), Tone::Failure),
    }
}
