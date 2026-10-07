//! The Apps window: the device's installed apps, by name, and what can be
//! done with each: open, stop, clear, uninstall, and permissions and
//! special access.

use std::cell::RefCell;

use aae_ffi::{AppInfo, AppPermissions};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetFocus, VK_F5, VK_O, VK_P, VK_R};
use windows::Win32::UI::WindowsAndMessaging::IDOK;

use crate::app::{self, announce, say, session_for, spawn, with_session};
use crate::forms::{self, Field, Form, Role};
use crate::panels::{self, Handler, Panel};
use crate::speech::Tone;
use crate::ui::{self, run_on_ui};

pub const KIND: &str = "apps";

const SYSTEM: u16 = 1000;
const LIST: u16 = 1001;
const OPEN: u16 = 1002;
const STOP: u16 = 1003;
const PERMISSIONS: u16 = 1004;
const CLEAR: u16 = 1005;
const UNINSTALL: u16 = 1006;
const REFRESH: u16 = 1007;

#[derive(Clone, Copy)]
struct Controls {
    system: HWND,
    status: HWND,
    list: HWND,
}

#[derive(Default)]
struct State {
    window: Option<Controls>,
    apps: Vec<AppInfo>,
    /// The device shown, by id, and whether it was running.
    device: Option<(String, bool)>,
    generation: u64,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

fn row(app: &AppInfo) -> String {
    let mut parts = vec![app.label.clone(), app.package.clone()];
    if !app.version.is_empty() {
        parts.push(format!("version {}", app.version));
    }
    if !app.enabled {
        parts.push("turned off".into());
    }
    parts.join(", ")
}

/// Opens the window, or brings it forward.
pub fn show() {
    if panels::bring_forward(KIND).is_some() {
        return;
    }
    let mut panel = Panel::new(KIND, "Apps", 600, 480);
    let system = panel.check("Show Android's own apps", SYSTEM, false);
    let status = panel.text("Reading…");
    let list = panel.list("Apps", LIST, 14, true);
    panel.buttons(
        &[
            ("Open", OPEN),
            ("Force Stop", STOP),
            ("Permissions…", PERMISSIONS),
            ("Clear Data…", CLEAR),
            ("Uninstall…", UNINSTALL),
        ],
        None,
    );
    panel.buttons(&[("Refresh", REFRESH)], None);
    panel.shortcut(VK_O.0, true, OPEN);
    panel.shortcut(VK_P.0, true, PERMISSIONS);
    panel.shortcut(VK_F5.0, false, REFRESH);
    panel.shortcut(VK_R.0, true, REFRESH);
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.window = Some(Controls {
            system,
            status,
            list,
        });
        s.device = None;
    });
    load(&panel);
    panel.every(1000);
    panel.show(Window, Some(list));
}

/// Reads the selected device's apps.
fn load(panel: &Panel) {
    let device = app::selected().0;
    panel.set_title(&match &device {
        Some(d) => format!("Apps on {}", d.name),
        None => "Apps".into(),
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
        STATE.with(|s| s.borrow_mut().apps.clear());
        ui::set_list_items(c.list, &[], None);
        ui::set_text(c.status, "Start the device to see its apps.");
        return;
    };
    ui::set_text(c.status, "Reading…");
    let system = ui::checked(c.system);
    spawn(async move {
        let result = async { session_for(device.id).await?.list_apps(system).await }.await;
        run_on_ui(move || {
            STATE.with(|s| {
                let mut s = s.borrow_mut();
                if s.generation != generation {
                    return;
                }
                let Some(c) = s.window else { return };
                match result {
                    Ok(apps) => {
                        // The same app stays selected.
                        let selected = ui::list_selection(c.list)
                            .and_then(|i| s.apps.get(i))
                            .map(|a| a.package.clone());
                        let rows: Vec<String> = apps.iter().map(row).collect();
                        let index = selected
                            .and_then(|p| apps.iter().position(|a| a.package == p))
                            .or((!apps.is_empty()).then_some(0));
                        ui::set_list_items(c.list, &rows, index);
                        let status = match apps.len() {
                            0 if system => "No apps.".to_string(),
                            0 => {
                                "No apps you've installed. Show Android's own apps to see the rest."
                                    .to_string()
                            }
                            1 => "1 app.".to_string(),
                            n => format!("{n} apps."),
                        };
                        ui::set_text(c.status, &status);
                        s.apps = apps;
                    }
                    Err(e) => ui::set_text(c.status, &e.to_string()),
                }
            })
        });
    });
}

fn selected_app() -> Option<AppInfo> {
    STATE.with(|s| {
        let s = s.borrow();
        let c = s.window?;
        s.apps.get(ui::list_selection(c.list)?).cloned()
    })
}

struct Window;

impl Handler for Window {
    fn command(&mut self, panel: &Panel, id: u16) {
        if matches!(id, SYSTEM | REFRESH) {
            load(panel);
            return;
        }
        let Some(c) = STATE.with(|s| s.borrow().window) else {
            return;
        };
        // Enter in the list opens the app.
        let id = if id == IDOK.0 as u16 && unsafe { GetFocus() } == c.list {
            OPEN
        } else {
            id
        };
        if !matches!(id, OPEN | STOP | PERMISSIONS | CLEAR | UNINSTALL) {
            return;
        }
        let Some(chosen) = selected_app() else {
            announce("Select an app first.", Tone::Failure);
            return;
        };
        match id {
            OPEN if !chosen.launchable => announce(
                &format!("{} has no screen to open.", chosen.label),
                Tone::Failure,
            ),
            OPEN => with_session(move |session| async move {
                session.open_app(chosen.package.clone()).await?;
                say(format!("Opened {}.", chosen.label), Tone::Info);
                Ok(())
            }),
            STOP => with_session(move |session| async move {
                session.force_stop_app(chosen.package.clone()).await?;
                say(format!("Stopped {}.", chosen.label), Tone::Success);
                Ok(())
            }),
            PERMISSIONS => permissions(chosen),
            CLEAR => clear_data(panel.hwnd, chosen),
            UNINSTALL if chosen.system => announce(
                &format!(
                    "{} is part of Android, so it can't be uninstalled.",
                    chosen.label
                ),
                Tone::Failure,
            ),
            UNINSTALL => uninstall(panel.hwnd, chosen),
            _ => {}
        }
    }

    /// Reads the apps again when another device is selected, or it starts
    /// or stops.
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
            s.apps.clear();
        });
    }
}

/// Asks, then deletes an app's data.
fn clear_data(owner: HWND, chosen: AppInfo) {
    if !forms::confirm(
        owner,
        &format!("Clear {}'s data?", chosen.label),
        "It goes back to how it was when first installed: signed out, with its settings and files deleted.",
        "Clear Data",
    ) {
        return;
    }
    with_session(move |session| async move {
        session.clear_app_data(chosen.package.clone()).await?;
        say(format!("Cleared {}'s data.", chosen.label), Tone::Success);
        Ok(())
    });
}

/// Asks, then uninstalls an app.
fn uninstall(owner: HWND, chosen: AppInfo) {
    if !forms::confirm(
        owner,
        &format!("Uninstall {}?", chosen.label),
        "It and its data are removed from the device.",
        "Uninstall",
    ) {
        return;
    }
    with_session(move |session| async move {
        session.uninstall_app(chosen.package.clone()).await?;
        say(format!("Uninstalled {}.", chosen.label), Tone::Success);
        run_on_ui(|| panels::press(KIND, REFRESH));
        Ok(())
    });
}

/// Reads an app's permissions and special access, then shows them, each a
/// checkbox that changes it at once.
fn permissions(chosen: AppInfo) {
    with_session(move |session| async move {
        let found = session.app_permissions(chosen.package.clone()).await?;
        app::on_ui(move || show_permissions(chosen, found)).await;
        Ok(())
    });
}

fn show_permissions(chosen: AppInfo, found: AppPermissions) {
    const GRANT_ALL: i32 = 1;
    const PERMISSION: i32 = 1000;
    const ACCESS: i32 = 2000;
    let mut form = Form::new(&format!("Permissions of {}", chosen.label));
    // Each permission's field, to show changes in.
    let mut fields = Vec::new();
    if found.permissions.is_empty() {
        form = form.text("It asks for no permissions.");
    }
    for (i, permission) in found.permissions.iter().enumerate() {
        fields.push((form.fields.len(), permission.name.clone()));
        form = form.field(Field::Toggle {
            label: permission.label.clone(),
            checked: permission.granted,
            action: PERMISSION + i as i32,
        });
    }
    form = form.text("Special access:");
    let mut access_fields = Vec::new();
    for (i, access) in found.access.iter().enumerate() {
        access_fields.push(form.fields.len());
        form = form.field(Field::Toggle {
            label: access.name.clone(),
            checked: access.allowed,
            action: ACCESS + i as i32,
        });
    }
    let permission_fields: Vec<usize> = fields.iter().map(|(field, _)| *field).collect();
    form.action = Some(Box::new(move |action, handle| {
        let package = chosen.package.clone();
        if action == GRANT_ALL {
            // Shows what's granted now.
            let controls: Vec<(isize, String)> = fields
                .iter()
                .filter_map(|(field, name)| {
                    Some((handle.control(*field)?.0 as isize, name.clone()))
                })
                .collect();
            let label = chosen.label.clone();
            with_session(move |session| async move {
                let count = session.grant_all_permissions(package.clone()).await?;
                let now = session.app_permissions(package).await?;
                run_on_ui(move || {
                    for (control, name) in controls {
                        if let Some(p) = now.permissions.iter().find(|p| p.name == name) {
                            ui::set_checked(HWND(control as *mut _), p.granted);
                        }
                    }
                });
                say(
                    if count == 0 {
                        format!("{label} already has every permission.")
                    } else {
                        format!("Granted {count} permissions.")
                    },
                    Tone::Success,
                );
                Ok(())
            });
        } else if let Some(permission) = action
            .checked_sub(PERMISSION)
            .filter(|_| action < ACCESS)
            .and_then(|i| found.permissions.get(i as usize))
        {
            let granted = handle.checked(permission_fields[(action - PERMISSION) as usize]);
            let (name, label) = (permission.name.clone(), permission.label.clone());
            with_session(move |session| async move {
                session.set_app_permission(package, name, granted).await?;
                let state = if granted { "granted" } else { "revoked" };
                say(format!("{label}: {state}."), Tone::Info);
                Ok(())
            });
        } else if let Some(access) = action
            .checked_sub(ACCESS)
            .and_then(|i| found.access.get(i as usize))
        {
            let allowed = handle.checked(access_fields[(action - ACCESS) as usize]);
            let (kind, name) = (access.kind, access.name.clone());
            with_session(move |session| async move {
                session.set_app_access(package, kind, allowed).await?;
                let state = if allowed { "allowed" } else { "not allowed" };
                say(format!("{name}: {state}."), Tone::Info);
                Ok(())
            });
        }
    }));
    let owner = panels::open_window(KIND).unwrap_or_else(ui::main_window);
    forms::run(
        owner,
        form.button("Grant All", GRANT_ALL, Role::Action)
            .button("Done", 0, Role::Default),
    );
}
