//! Watching an app's build output, to install each new build on devices as
//! it's made, and the window listing what's watched.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use windows::Win32::Foundation::HWND;

use crate::app::{self, announce, runtime, say};
use crate::forms::{self, Field, Form, Role};
use crate::panels::{self, Handler, Panel};
use crate::speech::Tone;
use crate::ui::{self, run_on_ui};

pub const KIND: &str = "watches";

const LIST: u16 = 1000;
const STOP: u16 = 1001;
const ANOTHER: u16 = 1002;

struct Watch {
    id: u64,
    path: String,
    ids: Vec<String>,
    /// When a build was last installed, as a clock time.
    last: Option<String>,
    task: tokio::task::JoinHandle<()>,
}

#[derive(Default)]
struct State {
    watches: Vec<Watch>,
    next_id: u64,
    window: Option<(HWND, HWND)>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// One build: the newest APK at the path, and when it changed and how big
/// it is.
#[derive(PartialEq, Clone)]
struct Build {
    path: PathBuf,
    modified: SystemTime,
    size: u64,
}

/// The newest APK at a path: the file itself, or the most recently changed
/// one in a folder, a few levels down.
fn current_build(path: &Path) -> Option<Build> {
    fn build(path: &Path) -> Option<Build> {
        let meta = std::fs::metadata(path).ok()?;
        meta.is_file().then(|| Build {
            path: path.to_path_buf(),
            modified: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            size: meta.len(),
        })
    }
    fn is_apk(path: &Path) -> bool {
        path.extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("apk"))
    }
    if is_apk(path) {
        return build(path);
    }
    let mut newest: Option<Build> = None;
    let mut folders = vec![(path.to_path_buf(), 0)];
    while let Some((folder, depth)) = folders.pop() {
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                if depth < 7 {
                    folders.push((path, depth + 1));
                }
            } else if is_apk(&path)
                && let Some(found) = build(&path)
                && newest.as_ref().is_none_or(|n| found.modified > n.modified)
            {
                newest = Some(found);
            }
        }
    }
    newest
}

fn describe(watch: &Watch) -> String {
    let names: Vec<String> = app::with(|app| {
        watch
            .ids
            .iter()
            .filter_map(|id| {
                app.devices
                    .iter()
                    .find(|d| d.id == *id)
                    .map(|d| d.name.clone())
            })
            .collect()
    });
    let mut text = format!("{}, for {}.", watch.path, names.join(", "));
    if let Some(last) = &watch.last {
        text += &format!(" Last installed at {last}.");
    }
    text
}

/// Opens the window listing what's watched, then asks what to watch.
pub fn watch_for_builds() {
    show();
    ask();
}

/// Opens the window, or brings it forward.
pub fn show() {
    if panels::bring_forward(KIND).is_some() {
        return;
    }
    let mut panel = Panel::new(KIND, "Watching for New Builds", 560, 320);
    let list = panel.list("Watched", LIST, 6, true);
    panel.buttons(
        &[("Stop Watching", STOP), ("Watch Another…", ANOTHER)],
        None,
    );
    let window = panel.hwnd;
    STATE.with(|s| s.borrow_mut().window = Some((window, list)));
    show_watches();
    panel.show(Window, Some(list));
}

fn show_watches() {
    let rows: Vec<String> = STATE.with(|s| s.borrow().watches.iter().map(describe).collect());
    let Some((_, list)) = STATE.with(|s| s.borrow().window) else {
        return;
    };
    if rows.is_empty() {
        ui::set_list_items(
            list,
            &["Nothing is being watched. Choose Watch Another.".to_string()],
            Some(0),
        );
    } else {
        let selected = ui::list_selection(list)
            .filter(|&i| i < rows.len())
            .or(Some(0));
        ui::set_list_items(list, &rows, selected);
    }
}

/// Asks for an APK or a build folder, and the devices to install on, then
/// watches it.
fn ask() {
    let running: Vec<_> =
        app::with(|app| app.devices.iter().filter(|d| d.running).cloned().collect());
    if running.is_empty() {
        announce(
            "Start a device first, to install builds on it.",
            Tone::Failure,
        );
        return;
    }
    let selection = app::selected().0.map(|d| d.id);
    let owner = STATE
        .with(|s| s.borrow().window.map(|(w, _)| w))
        .unwrap_or_else(ui::main_window);
    const CHOOSE_APK: i32 = 2;
    const CHOOSE_FOLDER: i32 = 3;
    let mut form = Form::new("Watch for New Builds")
        .field(Field::Edit {
            label: "An app's APK, or the folder its builds go in, such as build\\outputs\\apk"
                .into(),
            value: String::new(),
        })
        .text("Install each new build on:");
    for device in &running {
        form = form.field(Field::Check {
            label: format!("{}, {}", device.name, device.android),
            checked: running.len() == 1 || Some(&device.id) == selection.as_ref(),
        });
    }
    form.action = Some(Box::new(|action, handle| {
        let chosen = if action == CHOOSE_APK {
            ui::open_files(
                handle.dialog,
                "Choose the App's APK",
                &[("App packages", "*.apk")],
                false,
            )
            .into_iter()
            .next()
        } else {
            ui::choose_folder(handle.dialog, "Choose the Folder Its Builds Go In")
        };
        if let Some(path) = chosen {
            handle.set_text(0, &path);
            if let Some(field) = handle.control(0) {
                ui::focus(field);
            }
        }
    }));
    let answer = forms::run(
        owner,
        form.button("Choose APK…", CHOOSE_APK, Role::Action)
            .button("Choose Folder…", CHOOSE_FOLDER, Role::Action)
            .button("Watch", 1, Role::Default)
            .button("Cancel", 0, Role::Cancel),
    );
    if answer.button != 1 {
        return;
    }
    let path = answer.values[0].text().trim().trim_matches('"').to_string();
    if path.is_empty() || !Path::new(&path).exists() {
        announce(
            "Nothing is watched: choose an APK, or a folder, that exists.",
            Tone::Failure,
        );
        return;
    }
    let ids: Vec<String> = running
        .iter()
        .zip(answer.values.iter().skip(2))
        .filter(|(_, v)| v.checked())
        .map(|(d, _)| d.id.clone())
        .collect();
    if ids.is_empty() {
        announce("No devices were chosen, so nothing is watched.", Tone::Info);
        return;
    }
    start(path, ids);
}

/// Installs each new build at a path on the devices, until stopped.
fn start(path: String, ids: Vec<String>) {
    let id = STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.next_id += 1;
        s.next_id
    });
    let watched = PathBuf::from(&path);
    let devices = ids.clone();
    let task = runtime().spawn(async move {
        let mut last = current_build(&watched);
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let Some(build) = current_build(&watched).filter(|b| Some(b) != last.as_ref()) else {
                continue;
            };
            // Waits for it to settle, so a half-written file isn't installed.
            tokio::time::sleep(Duration::from_millis(1200)).await;
            if current_build(&watched).as_ref() != Some(&build) {
                continue;
            }
            last = Some(build.clone());
            let file = app::file_name(&build.path.to_string_lossy());
            say(format!("New build of {file}."), Tone::Info);
            let apk = build.path.to_string_lossy().to_string();
            let devices = devices.clone();
            run_on_ui(move || {
                let now = SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .map_or(0, |d| d.as_millis() as u64);
                let clock = aae_core::tts::clock_time(now);
                STATE.with(|s| {
                    if let Some(w) = s.borrow_mut().watches.iter_mut().find(|w| w.id == id) {
                        w.last = Some(clock.chars().take(5).collect());
                    }
                });
                show_watches();
                app::install(vec![apk], devices);
            });
        }
    });
    let names: Vec<String> = app::with(|app| ids.iter().map(|id| app.device_name(id)).collect());
    STATE.with(|s| {
        s.borrow_mut().watches.push(Watch {
            id,
            path: path.clone(),
            ids,
            last: None,
            task,
        })
    });
    show_watches();
    announce(
        &format!(
            "Watching {}. Each new build goes on {}.",
            app::file_name(&path),
            names.join(" and ")
        ),
        Tone::Success,
    );
}

struct Window;

impl Handler for Window {
    fn command(&mut self, _panel: &Panel, id: u16) {
        match id {
            ANOTHER => ask(),
            STOP => {
                let stopped = STATE.with(|s| {
                    let mut s = s.borrow_mut();
                    let (_, list) = s.window?;
                    let index = ui::list_selection(list).filter(|&i| i < s.watches.len())?;
                    Some(s.watches.remove(index))
                });
                match stopped {
                    Some(watch) => {
                        watch.task.abort();
                        show_watches();
                        announce(
                            &format!("Stopped watching {}.", app::file_name(&watch.path)),
                            Tone::Info,
                        );
                    }
                    None => announce("Nothing is being watched.", Tone::Info),
                }
            }
            _ => {}
        }
    }

    fn closed(&mut self) {
        // Watching goes on with the window closed, as on the Mac.
        STATE.with(|s| s.borrow_mut().window = None);
    }
}
