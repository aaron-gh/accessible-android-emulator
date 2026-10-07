//! The main window's state and everything it does, as the Mac app's AppModel
//! does. Work that takes time runs on AAE's background threads and reports
//! back to the window thread.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use aae_ffi::{
    AaeError, AppChoice, AppPartInfo, AppPartKind, CheckOutcome, DeviceInfo, DeviceProfile,
    DownloadListener, Engine, LicenceInfo, ProgressListener, ScreenPoint, ScreenReaderSource,
    Session, SetupStatus, TouchTarget, TouchTargets, VersionInfo,
};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Controls::{PBM_SETPOS, TBM_SETPAGESIZE, TBM_SETPOS, TBM_SETRANGE};
use windows::Win32::UI::WindowsAndMessaging::WM_USER;

/// A slider's position, missing from the windows crate.
const TBM_GETPOS: u32 = WM_USER;
use windows::Win32::UI::WindowsAndMessaging::{PostQuitMessage, WM_CLOSE};
use windows::core::w;

use crate::forms::{self, Field, Form, Role};
use crate::gestures::{self, GestureAction};
use crate::speech::{Tone, announce as speak};
use crate::ui::{self, run_on_ui};
use crate::{keyboard, menu, settings};

// MARK: - Background work

pub(crate) fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("aae-windows")
            .enable_all()
            .build()
            .expect("the background threads could not start")
    })
}

pub(crate) fn spawn(work: impl Future<Output = ()> + Send + 'static) {
    runtime().spawn(work);
}

/// Runs something on the window thread, such as asking a question, and waits
/// for its answer.
pub(crate) async fn on_ui<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = tokio::sync::oneshot::channel();
    run_on_ui(move || {
        let _ = tx.send(work());
    });
    rx.await.expect("the window thread answers")
}

static ENGINE: OnceLock<Result<Arc<Engine>, String>> = OnceLock::new();

pub(crate) fn engine() -> Option<Arc<Engine>> {
    ENGINE.get().and_then(|e| e.as_ref().ok()).cloned()
}

/// Connections to running devices, by device id.
static SESSIONS: Mutex<Option<HashMap<String, Arc<Session>>>> = Mutex::new(None);

pub(crate) fn session_if_open(id: &str) -> Option<Arc<Session>> {
    SESSIONS.lock().unwrap().as_ref()?.get(id).cloned()
}

/// Speech bridge speakers, by device.
static SPEAKERS: Mutex<Option<HashMap<String, Arc<crate::speech_bridge::Speaker>>>> =
    Mutex::new(None);

fn start_speech_bridge(id: &str, session: &Arc<Session>) {
    let speaker = crate::speech_bridge::Speaker::start(session);
    session.start_speech_bridge(speaker.clone());
    SPEAKERS
        .lock()
        .unwrap()
        .get_or_insert_with(HashMap::new)
        .insert(id.to_string(), speaker);
}

fn stop_speech_bridge(id: &str, session: &Session) {
    session.stop_speech_bridge();
    if let Some(speakers) = SPEAKERS.lock().unwrap().as_mut() {
        speakers.remove(id);
    }
}

fn close_session(id: &str) {
    let session = SESSIONS.lock().unwrap().as_mut().and_then(|s| s.remove(id));
    if let Some(session) = session {
        session.stop_audio();
        stop_speech_bridge(id, &session);
        session.stop_playing_into_microphone();
        if session.recording_screen() {
            let session = session.clone();
            spawn(async move {
                let _ = session.stop_recording().await;
            });
        }
        if session.microphone_on() {
            spawn(async move {
                let _ = session.set_microphone(false).await;
            });
        }
    }
}

fn open_sessions() -> Vec<(String, Arc<Session>)> {
    SESSIONS
        .lock()
        .unwrap()
        .as_ref()
        .map(|s| s.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default()
}

/// The connection to a running device, made the first time it's needed,
/// with its sound playing.
pub(crate) async fn session_for(id: String) -> Result<Arc<Session>, AaeError> {
    if let Some(session) = session_if_open(&id) {
        return Ok(session);
    }
    let engine = engine().ok_or_else(|| failed("AAE's core did not start."))?;
    let session = engine.open_session(id.clone()).await?;
    session.start_audio(settings::get().correct_pitch).await?;
    SESSIONS
        .lock()
        .unwrap()
        .get_or_insert_with(HashMap::new)
        .insert(id.clone(), session.clone());
    if session.speech_bridge() {
        start_speech_bridge(&id, &session);
    }
    run_on_ui(|| with(|app| app.apply_audio_focus()));
    Ok(session)
}

fn failed(message: &str) -> AaeError {
    AaeError::Failed {
        message: message.to_string(),
    }
}

/// Says something from any thread.
pub(crate) fn say(text: impl Into<String>, tone: Tone) {
    let text = text.into();
    run_on_ui(move || announce(&text, tone));
}

pub(crate) fn say_error(error: AaeError) {
    say(error.to_string(), Tone::Failure);
}

struct Progress;

impl ProgressListener for Progress {
    fn progress(&self, message: String) {
        say(message, Tone::Info);
    }
}

struct Download;

impl DownloadListener for Download {
    fn downloaded(&self, percent: u32) {
        run_on_ui(move || {
            with(|app| {
                if let Some((what, before)) = app.download.clone() {
                    if percent / 10 > before / 10 {
                        Tone::Progress.play();
                    }
                    app.download = Some((what, percent));
                    app.render();
                }
            })
        });
    }

    fn stage(&self, message: String) {
        run_on_ui(move || {
            with(|app| {
                app.status = message;
                app.render();
            })
        });
    }
}

// MARK: - State

struct Controls {
    // Shown when AAE couldn't start, and while setting up.
    heading: HWND,
    setup_text: HWND,
    setup_button: HWND,
    retry_button: HWND,
    // The device list and its buttons.
    devices_label: HWND,
    devices: HWND,
    new_device: HWND,
    start_stop: HWND,
    keyboard: HWND,
    back: HWND,
    home: HWND,
    recents: HWND,
    notifications: HWND,
    install: HWND,
    volume_label: HWND,
    volume: HWND,
    // Downloads.
    progress_label: HWND,
    progress: HWND,
    // Device mode.
    mode_text: HWND,
    return_button: HWND,
    // Every screen.
    status_label: HWND,
    status: HWND,
}

// Control ids, for the window's commands.
pub const DEVICE_LIST: u16 = 1000;
pub const SETUP_BUTTON: u16 = 1001;
pub const RETRY_BUTTON: u16 = 1002;
pub const START_STOP_BUTTON: u16 = 1003;
pub const RETURN_BUTTON: u16 = 1004;
pub const VOLUME_SLIDER: u16 = 1005;

#[derive(PartialEq, Clone, Copy)]
enum Screen {
    Error,
    Setup,
    Devices,
    DeviceMode,
}

#[derive(Default)]
struct Touch {
    point: Option<ScreenPoint>,
    label: Option<String>,
    item: Option<TouchTarget>,
}

static TOUCH: Mutex<Touch> = Mutex::new(Touch {
    point: None,
    label: None,
    item: None,
});

pub struct App {
    hwnd: HWND,
    c: Controls,
    startup_error: Option<String>,
    pub(crate) devices: Vec<DeviceInfo>,
    pub(crate) selection: Option<String>,
    pub(crate) busy: HashMap<String, String>,
    status: String,
    shown_status: String,
    shown_rows: Vec<String>,
    /// The screen last shown, to put the focus on a new one.
    shown_screen: Option<Screen>,
    download: Option<(String, u32)>,
    versions: Vec<VersionInfo>,
    needs_setup: bool,
    setup_status: Option<SetupStatus>,
    setup_error: Option<String>,
    setting_up: bool,
    /// First-start choices for devices just created, by device id.
    first_start: HashMap<String, (Option<String>, bool)>,
    device_mode: Option<String>,
    gesture_mode: bool,
    sound_problems: HashSet<String>,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// Runs `work` with the app's state borrowed. Don't open a form inside it:
/// forms pump window messages, which may borrow the state again.
pub(crate) fn with<R>(work: impl FnOnce(&mut App) -> R) -> R {
    APP.with(|cell| {
        let mut app = cell.borrow_mut();
        work(app.as_mut().expect("the app has started"))
    })
}

/// The selected device, and AAE's main window.
pub(crate) fn selected() -> (Option<DeviceInfo>, HWND) {
    with(|app| (app.selected().cloned(), app.hwnd))
}

/// Creates the window's controls and starts AAE's core.
pub fn start_app(hwnd: HWND) {
    ui::set_main_window(hwnd);
    let c = Controls {
        heading: ui::label(hwnd, ""),
        setup_text: ui::text_area(hwnd, "", 0, true),
        setup_button: ui::button(hwnd, "Download and Set Up", SETUP_BUTTON),
        retry_button: ui::button(hwnd, "Try Again", RETRY_BUTTON),
        devices_label: ui::label(hwnd, "&Devices"),
        devices: ui::list_box(hwnd, DEVICE_LIST),
        new_device: ui::button(hwnd, "&New Device…", menu::NEW_DEVICE),
        start_stop: ui::button(hwnd, "&Start", START_STOP_BUTTON),
        keyboard: ui::button(hwnd, "Use Android &Keyboard", menu::KEYBOARD),
        back: ui::button(hwnd, "&Back", menu::BACK),
        home: ui::button(hwnd, "&Home", menu::HOME),
        recents: ui::button(hwnd, "&Recent Apps", menu::RECENTS),
        notifications: ui::button(hwnd, "N&otifications", menu::NOTIFICATIONS),
        install: ui::button(hwnd, "&Install App…", menu::INSTALL_APP),
        volume_label: ui::label(hwnd, "&Volume"),
        volume: ui::control(
            hwnd,
            w!("msctls_trackbar32"),
            "",
            windows::Win32::UI::WindowsAndMessaging::WS_TABSTOP,
            Default::default(),
            VOLUME_SLIDER,
        ),
        progress_label: ui::label(hwnd, ""),
        progress: ui::control(
            hwnd,
            w!("msctls_progress32"),
            "",
            Default::default(),
            Default::default(),
            0,
        ),
        mode_text: ui::label(hwnd, ""),
        return_button: ui::button(hwnd, "&Return to Windows", RETURN_BUTTON),
        status_label: ui::label(hwnd, "&Last message"),
        status: ui::text_area(hwnd, "", 0, true),
    };
    ui::send(c.volume, TBM_SETRANGE, 1, (100 << 16) as isize);
    ui::send(c.volume, TBM_SETPAGESIZE, 0, 10);
    ui::send(
        c.volume,
        windows::Win32::UI::Controls::TBM_SETLINESIZE,
        0,
        5,
    );

    let started = Engine::new().map_err(|e| e.to_string());
    let startup_error = started.as_ref().err().cloned();
    let _ = ENGINE.set(started);
    let needs_setup = engine().is_some_and(|e| e.needs_setup());
    APP.with(|cell| {
        *cell.borrow_mut() = Some(App {
            hwnd,
            c,
            startup_error,
            devices: Vec::new(),
            selection: None,
            busy: HashMap::new(),
            status: String::new(),
            shown_status: String::new(),
            shown_rows: Vec::new(),
            shown_screen: None,
            download: None,
            versions: Vec::new(),
            needs_setup,
            setup_status: None,
            setup_error: None,
            setting_up: false,
            first_start: HashMap::new(),
            device_mode: None,
            gesture_mode: false,
            sound_problems: HashSet::new(),
        })
    });
    keyboard::set_events(keyboard::Events {
        on_escape: || leave_device_mode(false),
        on_gestures: gesture_actions,
    });
    start_gesture_queue();
    if needs_setup {
        check_setup(false);
    } else {
        mention_updates();
    }
    watch_sound();
    watch_devices();
    with(|app| {
        app.refresh();
        app.render();
    });
}

/// Puts the focus where the current screen starts: the device list, the
/// set-up button, or the way back from device mode.
pub fn focus_start() {
    // Focusing can activate the window, which comes back here, so it
    // happens after the state is put down.
    let control = with(|app| match app.screen() {
        Screen::Devices => app.c.devices,
        // The button is unavailable until AAE knows what to download.
        Screen::Setup if ui::is_enabled(app.c.setup_button) => app.c.setup_button,
        Screen::Setup => app.c.setup_text,
        Screen::Error => app.c.setup_text,
        Screen::DeviceMode => app.c.return_button,
    });
    ui::focus(control);
}

impl App {
    fn selected(&self) -> Option<&DeviceInfo> {
        self.devices
            .iter()
            .find(|d| Some(&d.id) == self.selection.as_ref())
    }

    pub(crate) fn device_name(&self, id: &str) -> String {
        self.devices
            .iter()
            .find(|d| d.id == id)
            .map(|d| d.name.clone())
            .unwrap_or_else(|| "the device".into())
    }

    pub(crate) fn refresh(&mut self) {
        let Some(engine) = engine() else { return };
        match engine.devices() {
            Ok(devices) => self.devices = devices,
            Err(e) => {
                let message = e.to_string();
                self.status = message.clone();
                speak(&message, true);
            }
        }
        if self.selected().is_none() {
            self.selection = self.devices.first().map(|d| d.id.clone());
        }
    }

    fn screen(&self) -> Screen {
        if self.startup_error.is_some() {
            Screen::Error
        } else if self.needs_setup {
            Screen::Setup
        } else if self.device_mode.is_some() {
            Screen::DeviceMode
        } else {
            Screen::Devices
        }
    }

    fn row(&self, device: &DeviceInfo) -> String {
        let state = self
            .busy
            .get(&device.id)
            .map(|b| b.to_lowercase())
            .unwrap_or_else(|| if device.running { "running" } else { "stopped" }.into());
        format!(
            "{}, {}, {}, {state}",
            device.name, device.android, device.kind
        )
    }

    /// Brings the controls up to date with the state.
    pub(crate) fn render(&mut self) {
        let screen = self.screen();
        let c = &self.c;
        let device_controls = [
            c.devices_label,
            c.devices,
            c.new_device,
            c.start_stop,
            c.keyboard,
            c.back,
            c.home,
            c.recents,
            c.notifications,
            c.install,
            c.volume_label,
            c.volume,
        ];
        for control in device_controls {
            ui::show(control, screen == Screen::Devices);
        }
        ui::show(c.heading, matches!(screen, Screen::Error | Screen::Setup));
        ui::show(
            c.setup_text,
            matches!(screen, Screen::Error | Screen::Setup),
        );
        ui::show(c.setup_button, screen == Screen::Setup);
        ui::show(
            c.retry_button,
            screen == Screen::Setup && self.setup_error.is_some(),
        );
        ui::show(c.mode_text, screen == Screen::DeviceMode);
        ui::show(c.return_button, screen == Screen::DeviceMode);
        let downloading = self.download.is_some();
        ui::show(c.progress_label, downloading);
        ui::show(c.progress, downloading);

        match screen {
            Screen::Error => {
                ui::set_text(c.heading, "AAE could not start");
                ui::set_text(c.setup_text, self.startup_error.as_deref().unwrap_or(""));
            }
            Screen::Setup => {
                ui::set_text(c.heading, "Set up AAE");
                ui::set_text(c.setup_text, &self.setup_text());
                let ready = self
                    .setup_status
                    .as_ref()
                    .is_some_and(|s| s.virtualisation_problem.is_none() && !s.missing.is_empty());
                ui::enable(c.setup_button, ready && !self.setting_up);
            }
            Screen::DeviceMode => {
                let name = self
                    .selected()
                    .map(|d| d.name.clone())
                    .unwrap_or_else(|| "Android".into());
                let mut text = if self.gesture_mode {
                    format!("Keys perform gestures on {name}.\n\n{}", gestures::help())
                } else {
                    format!("The keyboard is in {name}.")
                };
                text.push_str(&format!(
                    "\n\nPress {} to return to Windows.",
                    keyboard::return_shortcut()
                ));
                ui::set_text(c.mode_text, &text);
            }
            Screen::Devices => {
                let rows: Vec<String> = self.devices.iter().map(|d| self.row(d)).collect();
                let selected = self
                    .devices
                    .iter()
                    .position(|d| Some(&d.id) == self.selection.as_ref());
                if rows.len() != self.shown_rows.len() {
                    ui::set_list_items(c.devices, &rows, selected);
                } else {
                    for (i, (new, old)) in rows.iter().zip(&self.shown_rows).enumerate() {
                        if new != old {
                            ui::set_list_item(c.devices, i, new);
                        }
                    }
                    if let Some(index) = selected
                        && ui::list_selection(c.devices) != Some(index)
                    {
                        ui::send(
                            c.devices,
                            windows::Win32::UI::WindowsAndMessaging::LB_SETCURSEL,
                            index,
                            0,
                        );
                    }
                }
                self.shown_rows = rows;
                let device = self.selected();
                let busy = device.is_some_and(|d| self.busy.contains_key(&d.id));
                let running = device.is_some_and(|d| d.running) && !busy;
                ui::set_text(
                    c.start_stop,
                    if device.is_some_and(|d| d.running) {
                        "St&op"
                    } else {
                        "&Start"
                    },
                );
                ui::enable(c.start_stop, device.is_some() && !busy);
                for control in [
                    c.keyboard,
                    c.back,
                    c.home,
                    c.recents,
                    c.notifications,
                    c.install,
                ] {
                    ui::enable(control, running);
                }
                ui::enable(c.volume, device.is_some());
                let volume = device
                    .map(|d| (d.volume * 100.0).round() as isize)
                    .unwrap_or(100);
                if ui::send(c.volume, TBM_GETPOS, 0, 0) != volume {
                    ui::send(c.volume, TBM_SETPOS, 1, volume);
                }
                if let Some(device) = device {
                    ui::set_text(c.volume_label, &format!("&Volume of {}", device.name));
                }
            }
        }
        if let Some((what, percent)) = &self.download {
            ui::set_text(c.progress_label, &format!("Downloading {what}: {percent}%"));
            ui::send(c.progress, PBM_SETPOS, *percent as usize, 0);
        }
        if self.status != self.shown_status {
            ui::set_text(c.status, &self.status);
            self.shown_status = self.status.clone();
        }
        crate::tools::device_changed(self.selected().map(|d| d.name.as_str()));
        self.layout();
        // A new screen hides the controls of the old one, which may have had
        // the focus, so it goes to where the new one starts. Device mode
        // puts it on its button itself.
        let changed = self
            .shown_screen
            .replace(screen)
            .is_some_and(|s| s != screen);
        if changed
            && screen != Screen::DeviceMode
            && unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() }
                == self.hwnd
        {
            run_on_ui(focus_start);
        }
    }

    fn setup_text(&self) -> String {
        if let Some(error) = &self.setup_error {
            return error.clone();
        }
        let Some(status) = &self.setup_status else {
            return "Checking what AAE needs…".into();
        };
        let mut lines = vec![format!(
            "AAE needs Google's Android emulator and tools. It downloads them once, {} in all, and checks each download.",
            status.missing_size
        )];
        for tool in &status.missing {
            lines.push(format!("{} {}, {}", tool.name, tool.revision, tool.size));
        }
        lines.push(if status.own_sdk {
            format!("They go in AAE's own folder, {}.", status.sdk_path)
        } else {
            format!(
                "They go in the Android SDK at {}, which Android Studio uses too.",
                status.sdk_path
            )
        });
        if let Some(problem) = &status.virtualisation_problem {
            lines.push(problem.clone());
        }
        if let Some(warning) = &status.performance_warning {
            lines.push(warning.clone());
        }
        lines.join("\n")
    }

    /// Places the controls for the window's size.
    pub fn layout(&self) {
        let mut rect = windows::Win32::Foundation::RECT::default();
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(self.hwnd, &mut rect);
        }
        let dpi = unsafe { windows::Win32::UI::HiDpi::GetDpiForWindow(self.hwnd) }.max(96) as i32;
        let width = rect.right * 96 / dpi;
        let height = rect.bottom * 96 / dpi;
        let c = &self.c;
        let m = 12;
        let inner = (width - 2 * m).max(200);

        // From the bottom: the last message, then any download.
        let mut bottom = height - m;
        bottom -= 64;
        ui::place(c.status, m, bottom, inner, 64);
        bottom -= 19;
        ui::place(c.status_label, m, bottom, inner, 16);
        if self.download.is_some() {
            bottom -= 28;
            ui::place(c.progress, m, bottom, inner, 18);
            bottom -= 19;
            ui::place(c.progress_label, m, bottom, inner, 16);
        }
        bottom -= 8;

        match self.screen() {
            Screen::Error | Screen::Setup => {
                ui::place(c.heading, m, m, inner, 18);
                let buttons = 36;
                let text_height = (bottom - m - 24 - buttons).max(60);
                ui::place(c.setup_text, m, m + 24, inner, text_height);
                let y = m + 24 + text_height + 8;
                ui::place(c.setup_button, m, y, 170, 26);
                ui::place(c.retry_button, m + 178, y, 110, 26);
            }
            Screen::DeviceMode => {
                let text_height = (bottom - m - 40).max(60);
                ui::place(c.mode_text, m, m, inner, text_height);
                ui::place(c.return_button, m, m + text_height + 8, 170, 26);
            }
            Screen::Devices => {
                let rows_height = 2 * 34 + 52;
                ui::place(c.devices_label, m, m, inner, 16);
                let list_height = (bottom - m - 19 - rows_height).max(80);
                ui::place(c.devices, m, m + 19, inner, list_height);
                let mut y = m + 19 + list_height + 8;
                let row = |controls: &[(HWND, i32)], y: i32| {
                    let mut x = m;
                    for &(control, w) in controls {
                        ui::place(control, x, y, w, 26);
                        x += w + 8;
                    }
                };
                row(
                    &[(c.new_device, 120), (c.start_stop, 90), (c.keyboard, 170)],
                    y,
                );
                y += 34;
                row(
                    &[
                        (c.back, 80),
                        (c.home, 80),
                        (c.recents, 110),
                        (c.notifications, 110),
                        (c.install, 110),
                    ],
                    y,
                );
                y += 34;
                ui::place(c.volume_label, m, y, 300, 16);
                ui::place(c.volume, m, y + 18, 300.min(inner), 30);
            }
        }
    }

    pub(crate) fn apply_audio_focus(&self) {
        let only_in_use = settings::get().play_only_in_use;
        let in_use = self.device_mode.clone().or_else(|| self.selection.clone());
        let sessions = open_sessions();
        let several = sessions.len() > 1;
        for (id, session) in sessions {
            session.set_background(only_in_use && several && Some(&id) != in_use.as_ref());
        }
    }
}

/// Says something, with its tone, and shows it as the last message.
pub fn announce(text: &str, tone: Tone) {
    if tone == Tone::Failure
        && let Some(engine) = engine()
    {
        engine.log_problem(text.to_string());
    }
    with(|app| {
        app.status = text.to_string();
        app.render();
    });
    tone.play();
    speak(text, tone == Tone::Failure);
}

// MARK: - Window events

pub fn command(id: u16, notification: u32) {
    use menu::*;
    if id == DEVICE_LIST {
        // LBN_SELCHANGE
        if notification == 1 {
            with(|app| {
                if let Some(index) = ui::list_selection(app.c.devices) {
                    app.selection = app.devices.get(index).map(|d| d.id.clone());
                    app.apply_audio_focus();
                    app.render();
                }
            });
        }
        return;
    }
    // In device mode, the only command is the way back.
    if keyboard::is_on() && id != RETURN_BUTTON {
        return;
    }
    match id {
        SETUP_BUTTON => run_setup(false),
        RETRY_BUTTON => check_setup(true),
        START_STOP_BUTTON => {
            if selected().0.is_some_and(|d| d.running) {
                stop(None)
            } else {
                start(None)
            }
        }
        RETURN_BUTTON => leave_device_mode(false),
        NEW_DEVICE => new_device(),
        UPDATE_TOOLS => run_setup(true),
        SETTINGS => edit_settings(),
        EXIT => unsafe {
            let hwnd = with(|app| app.hwnd);
            let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                Some(hwnd),
                WM_CLOSE,
                Default::default(),
                Default::default(),
            );
        },
        PASTE => {
            let hwnd = with(|app| app.hwnd);
            let files = ui::clipboard_files(hwnd);
            if !files.is_empty() {
                install_paths(files);
            }
        }
        START => start(None),
        STOP => stop(None),
        RESTART => restart(),
        KEYBOARD => enter_device_mode(false),
        GESTURES => enter_device_mode(true),
        SPEAK_STATUS => speak_status(),
        BACK => press("back"),
        HOME => press("home"),
        RECENTS => press("recents"),
        POWER => press("power"),
        ASSISTANT => press("assistant"),
        DEVICE_VOLUME_UP => press("volume-up"),
        DEVICE_VOLUME_DOWN => press("volume-down"),
        NOTIFICATIONS => shell_quietly("cmd statusbar expand-notifications"),
        QUICK_SETTINGS => shell_quietly("cmd statusbar expand-settings"),
        ROTATE_LEFT => rotate(true),
        ROTATE_RIGHT => rotate(false),
        MUTE => toggle_mute(),
        VOLUME_UP => step_volume(true),
        VOLUME_DOWN => step_volume(false),
        CHECK_AUDIO => check_audio(),
        AUDIO_OUTPUT => choose_audio_output(),
        SPEECH_BRIDGE => toggle_speech_bridge(),
        MICROPHONE => toggle_microphone(),
        CHECK_MICROPHONE => check_microphone(),
        PLAY_FILE => play_file_into_microphone(),
        COPY_CLIPBOARD => copy_device_clipboard(),
        SEND_CLIPBOARD => send_clipboard(false),
        TYPE_CLIPBOARD => send_clipboard(true),
        INSTALL_APP => install_app(),
        INSTALL_SCREEN_READER => install_screen_reader_build(),
        SCREENSHOT => screenshot(),
        RECORD => toggle_recording(),
        OPEN_LINK => crate::tools::links::open_link(),
        SEND_INTENT => crate::tools::links::send_intent(),
        CONDITIONS => crate::tools::conditions::show(),
        DEVICE_SETTINGS => crate::tools::device_settings::show(),
        SPEECH_LOG => crate::tools::speech_log::show(),
        SHELL => crate::tools::shell::show(),
        DEVICE_LOG => crate::tools::device_log::show(),
        INSPECTOR => crate::tools::inspector::show(),
        APPS => crate::tools::apps::show(),
        SERVICES => crate::tools::services::show(),
        SNAPSHOTS => crate::tools::snapshots::show(),
        OWN_WINDOW => match selected().0 {
            Some(device) => crate::tools::device_window::show(&device.id),
            None => announce("Select a device first.", Tone::Failure),
        },
        ANDROID_VERSIONS => crate::tools::versions::show(),
        SERVE => crate::tools::serve::show(),
        WATCH_BUILDS => crate::tools::watch::watch_for_builds(),
        RENAME => rename(),
        HARDWARE => edit_hardware(),
        COPY_DEVICE => copy_device(),
        EXPORT_DEVICE => export_device(),
        IMPORT_DEVICE => import_device(),
        WIPE => wipe(),
        DELETE => delete(),
        SELF_TEST => self_test(),
        DIAGNOSTIC_REPORT => diagnostic_report(),
        ABOUT => about(),
        CHECK_UPDATES if !crate::updates::check() => announce(
            "This copy of AAE can't update itself: WinSparkle.dll isn't next to it. Download AAE again from its GitHub page.",
            Tone::Failure,
        ),
        _ => {}
    }
}

/// The volume slider moved.
pub fn volume_changed() {
    let position = with(|app| ui::send(app.c.volume, TBM_GETPOS, 0, 0));
    set_volume(position as f32 / 100.0, false);
}

pub fn files_dropped(files: Vec<String>) {
    if !keyboard::is_on() {
        install_paths(files);
    }
}

thread_local! {
    /// Set while closing waits for devices to stop.
    static QUITTING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The devices being stopped, by name.
fn stopping_names() -> Vec<String> {
    with(|app| {
        app.busy
            .iter()
            .filter(|(_, doing)| doing.as_str() == "Stopping")
            .map(|(id, _)| app.device_name(id))
            .collect()
    })
}

/// Before the window closes: the keyboard back, and the sound off. Devices
/// keep running, as on the Mac. False while devices are stopping: a stop first has
/// Android write its data to disk, then tells the emulator to save and exit,
/// and quitting before that would leave the emulator running. The window
/// hides, and closes when they've stopped.
pub fn closing() -> bool {
    let stopping = stopping_names();
    if !stopping.is_empty() {
        QUITTING.with(|q| q.set(true));
        let hwnd = with(|app| app.hwnd);
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::ShowWindow(
                hwnd,
                windows::Win32::UI::WindowsAndMessaging::SW_HIDE,
            );
        }
        announce(
            &format!(
                "Waiting for {} to stop before quitting.",
                stopping.join(" and ")
            ),
            Tone::Info,
        );
        return false;
    }
    crate::updates::stop();
    if keyboard::is_on() {
        leave_device_mode(true);
    }
    for (id, _) in open_sessions() {
        close_session(&id);
    }
    unsafe { PostQuitMessage(0) };
    true
}

pub fn resized() {
    APP.with(|cell| {
        if let Ok(app) = cell.try_borrow()
            && let Some(app) = app.as_ref()
        {
            app.layout();
        }
    });
}

// MARK: - Setting up the SDK

fn check_setup(refresh: bool) {
    let Some(engine) = engine() else { return };
    with(|app| {
        app.setup_error = None;
        app.render();
    });
    spawn(async move {
        let result = engine.setup_status(refresh).await;
        run_on_ui(move || match result {
            Ok(status) => with(|app| {
                app.setup_status = Some(status);
                app.render();
            }),
            Err(e) => {
                let message = e.to_string();
                with(|app| app.setup_error = Some(message.clone()));
                announce(&message, Tone::Failure);
            }
        });
    });
}

/// Says, at most once a day, when tools AAE installed have updates. Quiet
/// when offline or when there are none.
fn mention_updates() {
    let Some(engine) = engine() else { return };
    let now = now();
    if now.saturating_sub(settings::get().last_update_mention) < 24 * 60 * 60 {
        return;
    }
    spawn(async move {
        let Ok(status) = engine.setup_status(false).await else {
            return;
        };
        if status.updates.is_empty() {
            return;
        }
        settings::change(|s| s.last_update_mention = now);
        let names: Vec<String> = status.updates.iter().map(|t| t.name.clone()).collect();
        // After the window has appeared, so the screen reader reads it.
        tokio::time::sleep(Duration::from_secs(2)).await;
        say(
            format!(
                "An update is available for {}. Choose Update Android Tools in the File menu.",
                names.join(" and ")
            ),
            Tone::Info,
        );
    });
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Shows a licence and waits for the user's answer.
async fn ask_licence(licence: LicenceInfo, what: String) -> bool {
    on_ui(move || {
        let hwnd = with(|app| app.hwnd);
        let answer = forms::run(
            hwnd,
            Form {
                width: 620,
                focus: Some(1),
                ..Form::new(&format!("Google's licence for {what}"))
            }
            .text(&format!(
                "To download {what}, you need to accept Google's licence. It's below."
            ))
            .field(Field::Area {
                label: "Licence text".into(),
                value: licence.text.clone(),
                read_only: true,
                lines: 18,
            })
            .button("Decline", 0, Role::Cancel)
            .button("Accept", 1, Role::Close),
        );
        answer.button == 1
    })
    .await
}

/// Downloads the missing tools (and with `update`, newer versions), asking
/// for Google's licence first if needed.
pub(crate) fn run_setup(update: bool) {
    let Some(engine) = engine() else { return };
    let busy = with(|app| std::mem::replace(&mut app.setting_up, true));
    if busy {
        return;
    }
    with(|app| {
        app.setup_error = None;
        app.render();
    });
    spawn(async move {
        let result: Result<bool, AaeError> = async {
            if let Some(licence) = engine.tools_licence(update).await? {
                if !ask_licence(licence.clone(), "the Android emulator and tools".into()).await {
                    say("Licence declined. Nothing was downloaded.", Tone::Info);
                    return Ok(false);
                }
                engine.accept_licence(licence)?;
            }
            let what = if update {
                "the updates"
            } else {
                "the Android emulator and tools"
            };
            run_on_ui(move || with(|app| app.download = Some((what.into(), 0))));
            say(
                format!("Downloading {what}. Control Shift I reports progress."),
                Tone::Info,
            );
            engine.install_tools(update, Arc::new(Download)).await?;
            Ok(true)
        }
        .await;
        let status = engine.setup_status(false).await.ok();
        let needs_setup = engine.needs_setup();
        run_on_ui(move || {
            with(|app| {
                app.setting_up = false;
                app.download = None;
                app.needs_setup = needs_setup;
                app.setup_status = status;
                app.refresh();
                app.render();
            });
            match result {
                Ok(true) if update => announce("Updated.", Tone::Success),
                Ok(true) => announce(
                    "AAE is set up. Next, create a device with Control N.",
                    Tone::Success,
                ),
                Ok(false) => {}
                Err(e) => {
                    let message = e.to_string();
                    with(|app| app.setup_error = Some(message.clone()));
                    announce(&message, Tone::Failure);
                }
            }
        });
    });
}

// MARK: - Devices

fn new_device() {
    let Some(engine) = engine() else { return };
    if with(|app| app.needs_setup) {
        announce("Set up AAE first.", Tone::Failure);
        return;
    }
    spawn(async move {
        let versions = match engine
            .versions(false, settings::get().include_previews)
            .await
        {
            Ok(v) => v,
            Err(e) => return say_error(e),
        };
        let default_reader = engine.default_screen_reader();
        run_on_ui(move || {
            with(|app| app.versions = versions.clone());
            new_device_form(versions, default_reader);
        });
    });
}

fn new_device_form(versions: Vec<VersionInfo>, default_reader: Option<String>) {
    let hwnd = with(|app| app.hwnd);
    if versions.is_empty() {
        forms::message(
            hwnd,
            "New Device",
            "No Android versions are available. Check your internet connection and try again.",
        );
        return;
    }
    let labels = |versions: &[VersionInfo]| -> Vec<String> {
        versions
            .iter()
            .map(|v| {
                if v.installed {
                    format!("{}, installed", v.description)
                } else {
                    format!("{}, {} to download", v.description, v.size)
                }
            })
            .collect()
    };
    // Start on the newest installed version, so nothing downloads by surprise.
    let start = versions.iter().position(|v| v.installed).unwrap_or(0);
    let initial_labels = labels(&versions);
    // The list changes when previews are turned on or off.
    let versions = Arc::new(Mutex::new(versions));
    let reader = Arc::new(Mutex::new(default_reader));
    let reader_text = |path: &Option<String>| match path {
        Some(path) => format!(
            "Screen reader: {}",
            std::path::Path::new(path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| path.clone())
        ),
        None => "Screen reader: the Android image's own, if it has one. If not, AAE asks.".into(),
    };
    let initial_reader = reader_text(&reader.lock().unwrap());
    let chosen = reader.clone();
    let shown = versions.clone();
    // The fields, in order, for the action to find.
    const VERSION: usize = 1;
    const PREVIEWS: usize = 2;
    const READER: usize = 4;
    const CHOOSE_READER: i32 = 2;
    const TOGGLE_PREVIEWS: i32 = 3;
    let form = Form {
        width: 500,
        action: Some(Box::new(move |id, handle| match id {
            CHOOSE_READER => {
                let files = ui::open_files(
                    handle.dialog,
                    "Choose a screen reader APK, such as a Backtalk build",
                    &[("Android apps", "*.apk")],
                    false,
                );
                if let Some(path) = files.into_iter().next() {
                    handle.set_text(READER, &reader_text(&Some(path.clone())));
                    *chosen.lock().unwrap() = Some(path);
                }
            }
            TOGGLE_PREVIEWS => {
                let include = handle.checked(PREVIEWS);
                settings::change(|s| s.include_previews = include);
                let Some(engine) = engine() else { return };
                // Google's list is kept for a day, so this is quick.
                let Ok(updated) = runtime().block_on(engine.versions(false, include)) else {
                    return;
                };
                let mut current = shown.lock().unwrap();
                let was = handle
                    .choice(VERSION)
                    .and_then(|i| current.get(i))
                    .map(|v| v.id.clone());
                let selected = was
                    .and_then(|id| updated.iter().position(|v| v.id == id))
                    .or_else(|| updated.iter().position(|v| v.installed))
                    .unwrap_or(0);
                handle.set_choices(VERSION, &labels(&updated), selected);
                *current = updated;
            }
            _ => {}
        })),
        ..Form::new("New Device")
    }
    .field(Field::Edit {
        label: "&Name".into(),
        value: String::new(),
    })
    .field(Field::Choice {
        label: "&Android version".into(),
        items: initial_labels,
        selected: start,
    })
    .field(Field::Toggle {
        label: "Include &previews of upcoming Android releases".into(),
        checked: settings::get().include_previews,
        action: TOGGLE_PREVIEWS,
    })
    .field(Field::Choice {
        label: "&Size".into(),
        items: vec!["Small phone".into(), "Phone".into(), "Tablet".into()],
        selected: 1,
    })
    .text(&initial_reader)
    .field(Field::Check {
        label: "&Turn the screen reader's volume up to full".into(),
        checked: true,
    })
    .button("&Choose Screen Reader…", CHOOSE_READER, Role::Action)
    .button("Create and Start", 1, Role::Default)
    .button("Cancel", 0, Role::Cancel);
    let answer = forms::run(hwnd, form);
    if answer.button != 1 {
        return;
    }
    let name = answer.values[0].text().trim().to_string();
    if name.is_empty() {
        announce("The device needs a name.", Tone::Failure);
        return;
    }
    let Some(version) = answer.values[VERSION]
        .choice()
        .and_then(|i| versions.lock().unwrap().get(i).cloned())
    else {
        return;
    };
    let profile = match answer.values[3].choice() {
        Some(0) => DeviceProfile::SmallPhone,
        Some(2) => DeviceProfile::Tablet,
        _ => DeviceProfile::Phone,
    };
    let volume_boost = answer.values[5].checked();
    let screen_reader = reader.lock().unwrap().clone();
    create_device(name, version, profile, screen_reader, volume_boost);
}

fn create_device(
    name: String,
    version: VersionInfo,
    profile: DeviceProfile,
    screen_reader: Option<String>,
    volume_boost: bool,
) {
    let Some(engine) = engine() else { return };
    spawn(async move {
        let mut sysdir = version.sysdir.clone();
        if !version.installed {
            match install_version(&engine, &version).await {
                Some(installed) => sysdir = installed,
                None => return,
            }
        }
        run_on_ui(move || match engine.create_device(name, sysdir, profile) {
            Ok(device) => {
                let id = device.id.clone();
                with(|app| {
                    app.first_start
                        .insert(id.clone(), (screen_reader, volume_boost));
                    app.refresh();
                    app.selection = Some(id.clone());
                    app.render();
                });
                announce(&format!("Created {}.", device.name), Tone::Success);
                start(Some(id));
            }
            Err(e) => announce(&e.to_string(), Tone::Failure),
        });
    });
}

/// Downloads a version, asking for its licence first if needed. Returns
/// where it's installed, or none if the user declined or it failed.
async fn install_version(engine: &Arc<Engine>, version: &VersionInfo) -> Option<String> {
    let result: Result<Option<String>, AaeError> = async {
        if let Some(licence) = engine.licence_to_accept(version.id.clone()).await? {
            if !ask_licence(licence.clone(), version.description.clone()).await {
                say(
                    format!(
                        "Licence declined. {} was not downloaded.",
                        version.description
                    ),
                    Tone::Info,
                );
                return Ok(None);
            }
            engine.accept_licence(licence)?;
        }
        let what = version.description.clone();
        run_on_ui(move || with(|app| app.download = Some((what, 0))));
        say(
            format!(
                "Downloading {}, {}. Control Shift I reports progress.",
                version.description, version.size
            ),
            Tone::Info,
        );
        let image = engine
            .install_version(version.id.clone(), Arc::new(Download))
            .await?;
        say(
            format!("{} is installed.", version.description),
            Tone::Success,
        );
        Ok(Some(image.sysdir))
    }
    .await;
    run_on_ui(|| {
        with(|app| {
            app.download = None;
            app.render();
        })
    });
    result.unwrap_or_else(|e| {
        say_error(e);
        None
    })
}

pub(crate) fn start(id: Option<String>) {
    let Some(engine) = engine() else { return };
    let Some((id, choices)) = with(|app| {
        let id = id.or_else(|| app.selection.clone())?;
        if app.busy.contains_key(&id) {
            return None;
        }
        let choices = app
            .first_start
            .get(&id)
            .cloned()
            .unwrap_or_else(|| (engine.default_screen_reader(), true));
        app.busy.insert(id.clone(), "Starting".into());
        app.render();
        Some((id, choices))
    }) else {
        return;
    };
    spawn(async move {
        let result = async {
            engine
                .start_device(id.clone(), choices.0, choices.1, Arc::new(Progress))
                .await?;
            session_for(id.clone()).await?;
            Ok::<(), AaeError>(())
        }
        .await;
        let started = result.is_ok();
        if let Err(e) = result {
            say_error(e);
        }
        run_on_ui(move || {
            let question = with(|app| {
                app.busy.remove(&id);
                if started {
                    app.first_start.remove(&id);
                }
                app.refresh();
                app.render();
                app.devices
                    .iter()
                    .find(|d| d.id == id)
                    .filter(|d| started && d.screen_reader.is_none() && !d.screen_reader_declined)
                    .cloned()
            });
            if started {
                Tone::Success.play();
                check_sound_after_start(id);
            }
            if let Some(device) = question {
                screen_reader_question(device);
            }
        });
    });
}

/// Asks what to do about a device whose Android image has no screen reader.
/// The user may have chosen such an image on purpose, to install their own.
fn screen_reader_question(device: DeviceInfo) {
    let hwnd = with(|app| app.hwnd);
    let detail = if device.backtalk_supported {
        "This Android image doesn't include one.".to_string()
    } else {
        format!(
            "This Android image doesn't include one, and Backtalk needs Android 8 or later. You can install a screen reader APK made for {}, such as an older TalkBack, or continue without.",
            device.android
        )
    };
    let mut form = Form::new(&format!("{} has no screen reader", device.name)).text(&detail);
    if device.backtalk_supported {
        form = form.button("&Download Backtalk", 1, Role::Close);
    }
    let form = form
        .button("&Choose a Screen Reader…", 2, Role::Close)
        .button("Continue Without", 0, Role::Cancel);
    let source = match forms::run(hwnd, form).button {
        1 => Some(ScreenReaderSource::Backtalk),
        2 => {
            let files = ui::open_files(
                hwnd,
                "Choose a screen reader APK",
                &[("Android apps", "*.apk")],
                false,
            );
            match files.into_iter().next() {
                Some(path) => Some(ScreenReaderSource::Apk { path }),
                None => return,
            }
        }
        _ => None,
    };
    set_up_screen_reader(device, source);
}

fn set_up_screen_reader(device: DeviceInfo, source: Option<ScreenReaderSource>) {
    let Some(engine) = engine() else { return };
    let Some(source) = source else {
        match engine.decline_screen_reader(device.id.clone()) {
            Ok(()) => announce(
                &format!("{} has no screen reader. AAE won't ask again.", device.name),
                Tone::Info,
            ),
            Err(e) => announce(&e.to_string(), Tone::Failure),
        }
        return;
    };
    announce(
        if matches!(source, ScreenReaderSource::Backtalk) {
            "Downloading Backtalk."
        } else {
            "Installing the screen reader."
        },
        Tone::Info,
    );
    let id = device.id.clone();
    with(|app| {
        app.busy
            .insert(id.clone(), "Setting up the screen reader".into());
        app.render();
    });
    spawn(async move {
        match engine.add_screen_reader(id.clone(), source).await {
            Ok(package) => say(format!("{package} is installed and on."), Tone::Success),
            Err(e) => say_error(e),
        }
        run_on_ui(move || {
            with(|app| {
                app.busy.remove(&id);
                app.refresh();
                app.render();
            });
            // Closing was waiting for this.
            if QUITTING.with(|q| q.get()) && stopping_names().is_empty() {
                let hwnd = with(|app| app.hwnd);
                unsafe {
                    let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                        Some(hwnd),
                        WM_CLOSE,
                        Default::default(),
                        Default::default(),
                    );
                }
            }
        });
    });
}

/// Restarts Android on the selected device, keeping everything on it.
fn restart() {
    let Some(engine) = engine() else { return };
    let (device, _) = selected();
    let Some(device) = device else { return };
    if with(|app| app.busy.contains_key(&device.id)) {
        return;
    }
    if !device.running {
        announce(
            &format!("{} is not running. Start it first.", device.name),
            Tone::Failure,
        );
        return;
    }
    if with(|app| app.device_mode.as_ref() == Some(&device.id)) {
        leave_device_mode(false);
    }
    let id = device.id.clone();
    with(|app| {
        app.busy.insert(id.clone(), "Restarting".into());
        app.render();
    });
    // Android's audio goes away while it restarts; listen again after.
    close_session(&id);
    spawn(async move {
        let result = async {
            engine
                .restart_device(id.clone(), Arc::new(Progress))
                .await?;
            session_for(id.clone()).await?;
            Ok::<(), AaeError>(())
        }
        .await;
        let ok = result.is_ok();
        if let Err(e) = result {
            say_error(e);
        }
        run_on_ui(move || {
            with(|app| {
                app.busy.remove(&id);
                app.refresh();
                app.render();
            });
            if ok {
                Tone::Success.play();
                check_sound_after_start(id);
            }
        });
    });
}

/// Asks, then wipes the selected device back to its first-boot state and
/// sets it up again with its screen reader.
fn wipe() {
    let Some(engine) = engine() else { return };
    let (device, hwnd) = selected();
    let Some(device) = device else { return };
    if with(|app| app.busy.contains_key(&device.id)) {
        return;
    }
    if !forms::confirm(
        hwnd,
        &format!("Wipe {}?", device.name),
        "Its apps, data and snapshots are deleted, and it's set up again as it was when first created, with its screen reader. It keeps its name, hardware and volume. This can't be undone.",
        "&Wipe",
    ) {
        return;
    }
    if with(|app| app.device_mode.as_ref() == Some(&device.id)) {
        leave_device_mode(false);
    }
    let id = device.id.clone();
    with(|app| {
        app.busy.insert(id.clone(), "Wiping".into());
        app.render();
    });
    close_session(&id);
    announce(
        &format!(
            "Wiping {}. Setting it up again takes a few minutes.",
            device.name
        ),
        Tone::Info,
    );
    spawn(async move {
        let result = async {
            engine.wipe_device(id.clone(), Arc::new(Progress)).await?;
            session_for(id.clone()).await?;
            Ok::<(), AaeError>(())
        }
        .await;
        match result {
            Ok(()) => run_on_ui(|| Tone::Success.play()),
            Err(e) => say_error(e),
        }
        run_on_ui(move || {
            with(|app| {
                app.busy.remove(&id);
                app.refresh();
                app.render();
            })
        });
    });
}

pub(crate) fn stop(id: Option<String>) {
    let Some(engine) = engine() else { return };
    let Some((id, name)) = with(|app| {
        let id = id.or_else(|| app.selection.clone())?;
        if app.busy.contains_key(&id) {
            return None;
        }
        Some((id.clone(), app.device_name(&id)))
    }) else {
        return;
    };
    if with(|app| app.device_mode.as_ref() == Some(&id)) {
        leave_device_mode(false);
    }
    with(|app| {
        app.busy.insert(id.clone(), "Stopping".into());
        app.render();
    });
    announce(&format!("Stopping {name}."), Tone::Info);
    close_session(&id);
    spawn(async move {
        match engine.stop_device(id.clone()).await {
            Ok(()) => say(format!("{name} is stopped."), Tone::Success),
            Err(e) => say_error(e),
        }
        run_on_ui(move || {
            with(|app| {
                app.busy.remove(&id);
                app.refresh();
                app.render();
            })
        });
    });
}

fn delete() {
    let Some(engine) = engine() else { return };
    let (device, hwnd) = selected();
    let Some(device) = device else { return };
    if device.running {
        announce(
            &format!("Stop {} before deleting it.", device.name),
            Tone::Failure,
        );
        return;
    }
    let size = engine
        .device_size(device.id.clone())
        .unwrap_or_else(|_| "all".into());
    if !forms::confirm(
        hwnd,
        &format!("Delete {}?", device.name),
        &format!("This deletes the device and its {size} of files. It can't be undone."),
        "&Delete",
    ) {
        return;
    }
    match engine.delete_device(device.id.clone()) {
        Ok(freed) => {
            with(|app| {
                app.selection = None;
                app.refresh();
                app.render();
            });
            announce(
                &format!("Deleted {}. Freed {freed}.", device.name),
                Tone::Success,
            );
        }
        Err(e) => announce(&e.to_string(), Tone::Failure),
    }
}

fn rename() {
    let Some(engine) = engine() else { return };
    let (device, hwnd) = selected();
    let Some(device) = device else { return };
    let Some(name) = forms::ask_name(
        hwnd,
        &format!("Rename {}", device.name),
        "&New name",
        &device.name,
        "Rename",
    ) else {
        return;
    };
    match engine.rename_device(device.id.clone(), name) {
        Ok(renamed) => {
            with(|app| {
                app.refresh();
                app.render();
            });
            announce(
                &format!("Renamed {} to {}.", device.name, renamed.name),
                Tone::Success,
            );
        }
        Err(e) => announce(&e.to_string(), Tone::Failure),
    }
}

fn copy_device() {
    let Some(engine) = engine() else { return };
    let (device, hwnd) = selected();
    let Some(device) = device else { return };
    let Some(name) = forms::ask_name(
        hwnd,
        &format!("Copy {}", device.name),
        "&Name for the copy",
        &format!("{} copy", device.name),
        "Copy",
    ) else {
        return;
    };
    announce(&format!("Copying {}.", device.name), Tone::Info);
    match engine.clone_device(device.id.clone(), name) {
        Ok(copy) => {
            with(|app| {
                app.refresh();
                app.selection = Some(copy.id.clone());
                app.render();
            });
            announce(
                &format!("Created {}, a copy of {}.", copy.name, device.name),
                Tone::Success,
            );
        }
        Err(e) => announce(&e.to_string(), Tone::Failure),
    }
}

/// Runs an action on the selected device's session, announcing any failure.
pub(crate) fn with_session<F, Fut>(action: F)
where
    F: FnOnce(Arc<Session>) -> Fut + Send + 'static,
    Fut: Future<Output = Result<(), AaeError>> + Send + 'static,
{
    let (device, _) = selected();
    let Some(device) = device else {
        announce("Select a device first.", Tone::Failure);
        return;
    };
    if !device.running {
        announce(
            &format!("{} is not running. Start it first.", device.name),
            Tone::Failure,
        );
        return;
    }
    spawn(async move {
        let result = async { action(session_for(device.id).await?).await }.await;
        if let Err(e) = result {
            say_error(e);
        }
    });
}

// MARK: - Device mode

/// Gives the keyboard to Android: to type (device mode), or with `gestures`,
/// to perform screen reader gestures (gesture mode).
fn enter_device_mode(gestures: bool) {
    if keyboard::is_on() {
        return;
    }
    let (device, _) = selected();
    let Some(device) = device else { return };
    if !device.running {
        announce(
            &format!("{} is not running. Start it first.", device.name),
            Tone::Failure,
        );
        return;
    }
    let id = device.id.clone();
    spawn(async move {
        let session = match session_for(id.clone()).await {
            Ok(s) => s,
            Err(e) => return say_error(e),
        };
        // In case Android dropped AAE's full keyboard, which Meta needs.
        let _ = session.ensure_keyboard_layout().await;
        if gestures {
            // Reading the screen, to move the touch point, needs AAE's helper.
            if let Err(e) = session.use_helper(true).await {
                return say_error(e);
            }
        }
        run_on_ui(move || {
            *TOUCH.lock().unwrap() = Touch::default();
            if !keyboard::enter(session, gestures) {
                announce(
                    "Couldn't give the keyboard to Android. Windows wouldn't let AAE watch the keyboard.",
                    Tone::Failure,
                );
                return;
            }
            let return_button = with(|app| {
                app.device_mode = Some(id);
                app.gesture_mode = gestures;
                app.apply_audio_focus();
                app.render();
                app.c.return_button
            });
            // Keys don't reach the window, but this keeps the focus off the
            // device list, so its name isn't read out again.
            ui::focus(return_button);
            if gestures {
                announce(
                    &format!(
                        "Gesture mode on. Arrows swipe, Space double taps, question mark lists the keys. {} returns to Windows.",
                        keyboard::return_shortcut()
                    ),
                    Tone::Info,
                );
            } else {
                announce(
                    &format!(
                        "Android keyboard on. {} returns to Windows.",
                        keyboard::return_shortcut()
                    ),
                    Tone::Info,
                );
            }
        });
    });
}

fn leave_device_mode(quietly: bool) {
    let Some(id) = with(|app| app.device_mode.clone()) else {
        return;
    };
    let gesture_mode = with(|app| app.gesture_mode);
    // Lifts any held touch, while the session is still there to lift it.
    let actions = keyboard::leave();
    gesture_actions(actions);
    if gesture_mode && let Some(session) = session_if_open(&id) {
        queue_gesture(session, |s| {
            Box::pin(async move { s.use_helper(false).await })
        });
    }
    let devices = with(|app| {
        app.device_mode = None;
        app.gesture_mode = false;
        app.apply_audio_focus();
        app.render();
        app.c.devices
    });
    ui::focus(devices);
    if !quietly {
        announce("Windows keyboard on.", Tone::Info);
    }
}

// MARK: - Gestures

type GestureJob = Box<
    dyn FnOnce(Arc<Session>) -> Pin<Box<dyn Future<Output = Result<(), AaeError>> + Send>> + Send,
>;

static GESTURES: OnceLock<tokio::sync::mpsc::UnboundedSender<(Arc<Session>, GestureJob)>> =
    OnceLock::new();

/// Gestures run one at a time, in order.
fn start_gesture_queue() {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(Arc<Session>, GestureJob)>();
    let _ = GESTURES.set(tx);
    spawn(async move {
        while let Some((session, job)) = rx.recv().await {
            if let Err(e) = job(session).await {
                say_error(e);
            }
        }
    });
}

fn queue_gesture(
    session: Arc<Session>,
    job: impl FnOnce(Arc<Session>) -> Pin<Box<dyn Future<Output = Result<(), AaeError>> + Send>>
    + Send
    + 'static,
) {
    if let Some(queue) = GESTURES.get() {
        let _ = queue.send((session, Box::new(job)));
    }
}

fn gesture_actions(actions: Vec<GestureAction>) {
    let Some(session) = with(|app| app.device_mode.clone()).and_then(|id| session_if_open(&id))
    else {
        return;
    };
    for action in actions {
        let at = TOUCH.lock().unwrap().point;
        match action {
            GestureAction::Gesture(name) => queue_gesture(session.clone(), move |s| {
                Box::pin(async move { s.perform_gesture(name, at).await })
            }),
            GestureAction::Press(name) => queue_gesture(session.clone(), move |s| {
                Box::pin(async move { s.press_gesture(name, at).await })
            }),
            GestureAction::Release => queue_gesture(session.clone(), |s| {
                Box::pin(async move { s.release_gesture().await })
            }),
            GestureAction::NextItem | GestureAction::PreviousItem => {
                let next = action == GestureAction::NextItem;
                queue_gesture(session.clone(), move |s| {
                    Box::pin(async move { move_to_item(next, s).await })
                })
            }
            GestureAction::Step(dx, dy) => queue_gesture(session.clone(), move |s| {
                Box::pin(async move { step_touch_point(dx, dy, s).await })
            }),
            GestureAction::Centre => {
                let mut touch = TOUCH.lock().unwrap();
                touch.point = None;
                touch.item = None;
                drop(touch);
                queue_gesture(session.clone(), |s| {
                    Box::pin(async move { say_touch_point(s, Some("Middle of the screen.")).await })
                })
            }
            GestureAction::WhereIsIt => queue_gesture(session.clone(), |s| {
                Box::pin(async move { say_touch_point(s, None).await })
            }),
            GestureAction::Help => announce(&gestures::help(), Tone::Info),
            GestureAction::Unknown => Tone::Failure.play(),
        }
    }
}

/// Moves the touch point to the next or previous thing on the screen, in
/// reading order, and says what it is.
async fn move_to_item(next: bool, session: Arc<Session>) -> Result<(), AaeError> {
    let screen = session.touch_targets().await?;
    let targets = &screen.targets;
    if targets.is_empty() {
        say("There's nothing on the screen to touch.", Tone::Failure);
        return Ok(());
    }
    let mut touch = TOUCH.lock().unwrap();
    let index: isize = match touch.point {
        Some(point) => {
            let item = touch.item.as_ref().and_then(|item| {
                targets.iter().position(|t| {
                    t.label == item.label
                        && t.left == item.left
                        && t.top == item.top
                        && t.right == item.right
                        && t.bottom == item.bottom
                })
            });
            match item.or_else(|| target_index(point, targets)) {
                Some(current) => current as isize + if next { 1 } else { -1 },
                // Between items: the next one down the screen.
                None if next => targets
                    .iter()
                    .position(|t| t.top >= point.y)
                    .unwrap_or(targets.len()) as isize,
                None => targets
                    .iter()
                    .rposition(|t| t.bottom <= point.y)
                    .map(|i| i as isize)
                    .unwrap_or(-1),
            }
        }
        None if next => 0,
        None => targets.len() as isize - 1,
    };
    if index < 0 || index as usize >= targets.len() {
        drop(touch);
        say(
            if next {
                "End of the screen."
            } else {
                "Start of the screen."
            },
            Tone::Failure,
        );
        return Ok(());
    }
    let target = targets[index as usize].clone();
    touch.point = Some(ScreenPoint {
        x: target.x,
        y: target.y,
    });
    touch.label = Some(target.label.clone());
    touch.item = Some(target.clone());
    drop(touch);
    say(target.label, Tone::Info);
    Ok(())
}

/// Moves the touch point a step across the screen, saying what it reaches.
async fn step_touch_point(dx: i32, dy: i32, session: Arc<Session>) -> Result<(), AaeError> {
    let screen = session.touch_targets().await?;
    let step = (screen.width.min(screen.height) / 10).max(1);
    let mut touch = TOUCH.lock().unwrap();
    let start = touch.point.unwrap_or(ScreenPoint {
        x: screen.width / 2,
        y: screen.height / 2,
    });
    let x = (start.x + dx * step).clamp(0, screen.width - 1);
    let y = (start.y + dy * step).clamp(0, screen.height - 1);
    let at_edge = x == start.x && y == start.y;
    let point = ScreenPoint { x, y };
    touch.point = Some(point);
    touch.item = None;
    let under = target_index(point, &screen.targets).map(|i| screen.targets[i].clone());
    let same = under.as_ref().map(|u| &u.label) == touch.label.as_ref() && under.is_some();
    touch.label = under.as_ref().map(|u| u.label.clone());
    drop(touch);
    if at_edge {
        run_on_ui(|| Tone::Failure.play());
    }
    if same {
        // Still on the same item: a tick, not the whole label again.
        if !at_edge {
            run_on_ui(|| Tone::Progress.play());
        }
    } else {
        say(
            under
                .map(|u| u.label)
                .unwrap_or_else(|| format!("Nothing. {}", position(point, &screen))),
            Tone::Info,
        );
    }
    Ok(())
}

/// Says what's at the touch point, and where it is.
async fn say_touch_point(session: Arc<Session>, prefix: Option<&str>) -> Result<(), AaeError> {
    let screen = session.touch_targets().await?;
    let mut touch = TOUCH.lock().unwrap();
    let point = touch.point.unwrap_or(ScreenPoint {
        x: screen.width / 2,
        y: screen.height / 2,
    });
    let under = target_index(point, &screen.targets).map(|i| screen.targets[i].clone());
    touch.label = under.as_ref().map(|u| u.label.clone());
    drop(touch);
    let mut parts: Vec<String> = Vec::new();
    if let Some(prefix) = prefix {
        parts.push(prefix.into());
    }
    parts.push(under.map(|u| u.label).unwrap_or_else(|| "Nothing.".into()));
    if prefix.is_none() {
        parts.push(position(point, &screen));
    }
    say(parts.join(" "), Tone::Info);
    Ok(())
}

/// The smallest target containing a point.
fn target_index(point: ScreenPoint, targets: &[TouchTarget]) -> Option<usize> {
    targets
        .iter()
        .enumerate()
        .filter(|(_, t)| {
            (t.left..t.right).contains(&point.x) && (t.top..t.bottom).contains(&point.y)
        })
        .min_by_key(|(_, t)| (t.right - t.left) * (t.bottom - t.top))
        .map(|(i, _)| i)
}

/// Where a point is, as percentages across and down the screen.
fn position(point: ScreenPoint, screen: &TouchTargets) -> String {
    let across = point.x * 100 / screen.width.max(1);
    let down = point.y * 100 / screen.height.max(1);
    format!("{across} percent across, {down} percent down.")
}

// MARK: - Device actions

fn press(key: &'static str) {
    with_session(move |s| async move { s.press(key.into()).await });
}

fn shell_quietly(command: &'static str) {
    with_session(move |s| async move { s.shell(command.into()).await.map(|_| ()) });
}

fn rotate(left: bool) {
    with_session(move |s| async move {
        let orientation = s.rotate(left).await?;
        say(format!("{orientation}."), Tone::Info);
        Ok(())
    });
}

/// Sets how loud AAE plays the selected device, from 0 to 1, remembered for
/// the device.
pub(crate) fn set_volume(volume: f32, announce_it: bool) {
    let (device, _) = selected();
    let Some(device) = device else { return };
    let volume = volume.clamp(0.0, 1.0);
    with_session(move |s| async move {
        s.set_audio_volume(volume)?;
        if announce_it {
            say(
                format!("{} at {} percent.", device.name, (volume * 100.0).round()),
                Tone::Info,
            );
        }
        run_on_ui(|| {
            with(|app| {
                app.refresh();
                app.render();
            })
        });
        Ok(())
    });
}

fn step_volume(up: bool) {
    let (device, _) = selected();
    let Some(device) = device else { return };
    let now = (device.volume * 10.0).round() / 10.0;
    set_volume(now + if up { 0.1 } else { -0.1 }, true);
}

fn toggle_mute() {
    with_session(|s| async move {
        let muted = s.toggle_mute();
        say(
            if muted {
                "Device audio muted."
            } else {
                "Device audio on."
            },
            Tone::Info,
        );
        Ok(())
    });
}

/// Exports the selected device, stopped, to one file for another computer.
fn export_device() {
    let (device, hwnd) = selected();
    let Some(device) = device else { return };
    if device.running {
        announce(
            &format!("Stop {} to export it.", device.name),
            Tone::Failure,
        );
        return;
    }
    let Some(path) = ui::save_file(
        hwnd,
        &format!(
            "Export {}, with its apps, data and named snapshots",
            device.name
        ),
        &format!("{}.aaedevice", device.name),
        &[("AAE devices", "*.aaedevice")],
        "aaedevice",
    ) else {
        return;
    };
    let Some(engine) = engine() else { return };
    announce(&format!("Exporting {}.", device.name), Tone::Info);
    let id = device.id.clone();
    with(|app| {
        app.busy.insert(id.clone(), "Exporting".into());
        app.render();
    });
    spawn(async move {
        let result = engine
            .export_device(device.id, path, Arc::new(Progress))
            .await;
        run_on_ui(move || {
            with(|app| {
                app.busy.remove(&id);
                app.render();
            })
        });
        match result {
            Ok(said) => say(said, Tone::Success),
            Err(e) => say(e.to_string(), Tone::Failure),
        }
    });
}

/// Imports a device exported from AAE.
fn import_device() {
    let hwnd = with(|app| app.hwnd);
    let files = ui::open_files(
        hwnd,
        "Choose a device exported from AAE",
        &[("AAE devices", "*.aaedevice")],
        false,
    );
    let Some(path) = files.into_iter().next() else {
        return;
    };
    let Some(engine) = engine() else { return };
    announce("Importing the device.", Tone::Info);
    spawn(async move {
        match engine.import_device(path, Arc::new(Progress)).await {
            Ok(device) => {
                let id = device.id.clone();
                run_on_ui(move || {
                    with(|app| {
                        app.refresh();
                        app.selection = Some(id);
                        app.render();
                    })
                });
                say(format!("Imported {}.", device.name), Tone::Success);
            }
            Err(e) => say(e.to_string(), Tone::Failure),
        }
    });
}

/// A stopped device's advanced hardware, from its next start.
fn edit_hardware() {
    let (device, hwnd) = selected();
    let Some(device) = device else { return };
    if device.running {
        announce(
            &format!("Stop {} to change its hardware.", device.name),
            Tone::Failure,
        );
        return;
    }
    let Some(engine) = engine() else { return };
    let hardware = match engine.device_hardware(device.id.clone()) {
        Ok(hardware) => hardware,
        Err(e) => {
            announce(&e.to_string(), Tone::Failure);
            return;
        }
    };
    let field = |label: &str, value: u32| Field::Edit {
        label: label.into(),
        value: value.to_string(),
    };
    let answer = forms::run(
        hwnd,
        Form::new(&format!("Hardware of {}", device.name))
            .field(field("&Memory, in megabytes", hardware.memory_mb))
            .field(field("Processor &cores", hardware.cores))
            .field(field("&Storage, in megabytes", hardware.storage_mb))
            .field(field("Screen &width, in pixels", hardware.width))
            .field(field("Screen &height, in pixels", hardware.height))
            .field(field("Screen &density, in dots per inch", hardware.density))
            .text("Applies at next start. Reducing storage needs a wipe.")
            .button("Save", 1, Role::Default)
            .button("Cancel", 0, Role::Cancel),
    );
    if answer.button != 1 {
        return;
    }
    let numbers: Option<Vec<u32>> = (0..6)
        .map(|i| answer.values[i].text().trim().parse().ok())
        .collect();
    let Some(n) = numbers else {
        announce("Each one takes a whole number.", Tone::Failure);
        return;
    };
    let chosen = aae_ffi::HardwareInfo {
        memory_mb: n[0],
        cores: n[1],
        storage_mb: n[2],
        width: n[3],
        height: n[4],
        density: n[5],
    };
    match engine.set_device_hardware(device.id, chosen) {
        Ok(said) => announce(&said, Tone::Success),
        Err(e) => announce(&e.to_string(), Tone::Failure),
    }
}

/// Toggles the selected device's speech bridge.
fn toggle_speech_bridge() {
    let (device, _) = selected();
    let Some(device) = device else { return };
    let on = !device.speech_bridge;
    let id = device.id.clone();
    with_session(move |s| async move {
        let mut said = s.set_speech_bridge(on).await?;
        if on {
            start_speech_bridge(&id, &s);
            said.push_str(if crate::screen_readers::nvda::speaks_ssml() {
                " Using NVDA."
            } else {
                " Using SAPI."
            });
        } else {
            stop_speech_bridge(&id, &s);
        }
        run_on_ui(|| {
            with(|app| {
                app.refresh();
                app.render();
            })
        });
        say(said, Tone::Success);
        Ok(())
    });
}

/// Chooses which of this computer's outputs the selected device plays
/// through, remembered for it.
fn choose_audio_output() {
    let (device, hwnd) = selected();
    let Some(device) = device else { return };
    let mut outputs = aae_ffi::audio_outputs();
    if let Some(chosen) = &device.audio_output
        && !outputs.contains(chosen)
    {
        outputs.push(chosen.clone());
    }
    let mut items = vec!["System default".to_string()];
    items.extend(outputs.iter().cloned());
    let selected_index = device
        .audio_output
        .as_ref()
        .and_then(|o| outputs.iter().position(|x| x == o))
        .map_or(0, |i| i + 1);
    let form = Form::new(&format!("Audio Output for {}", device.name))
        .field(Field::Choice {
            label: "&Audio output".into(),
            items,
            selected: selected_index,
        })
        .button("OK", 1, Role::Default)
        .button("Cancel", 0, Role::Cancel);
    let answer = forms::run(hwnd, form);
    if answer.button != 1 {
        return;
    }
    let output = answer.values[0]
        .choice()
        .and_then(|i| i.checked_sub(1))
        .and_then(|i| outputs.get(i).cloned());
    with_session(move |s| async move {
        let said = s
            .set_audio_output(output, settings::get().correct_pitch)
            .await?;
        run_on_ui(|| {
            with(|app| {
                app.apply_audio_focus();
                app.refresh();
                app.render();
            })
        });
        say(said, Tone::Success);
        Ok(())
    });
}

/// Sends this computer's microphone into the selected device, or stops.
fn toggle_microphone() {
    with_session(|s| async move {
        let said = s.set_microphone(!s.microphone_on()).await?;
        say(said, Tone::Info);
        Ok(())
    });
}

/// Injects a tone into the selected device's microphone and reports whether
/// it was recorded. No host audio.
fn check_microphone() {
    let (device, _) = selected();
    let Some(device) = device else { return };
    announce(
        &format!("Checking {}'s microphone.", device.name),
        Tone::Info,
    );
    with_session(|s| async move {
        let said = s.check_microphone().await?;
        say(said, Tone::Success);
        Ok(())
    });
}

/// Plays an audio file into the selected device's microphone from the next
/// recording. Chosen while one is queued or playing, it stops it.
fn play_file_into_microphone() {
    let (device, _) = selected();
    let Some(device) = device else { return };
    if let Some(session) = session_if_open(&device.id)
        && session.stop_playing_into_microphone()
    {
        return;
    }
    let hwnd = with(|app| app.hwnd);
    let files = ui::open_files(
        hwnd,
        &format!(
            "Choose an audio file to play into {}'s microphone",
            device.name
        ),
        &[(
            "Sound files",
            "*.wav;*.mp3;*.flac;*.ogg;*.oga;*.m4a;*.aac;*.mp4",
        )],
        false,
    );
    let Some(path) = files.into_iter().next() else {
        return;
    };
    let file = std::path::Path::new(&path)
        .file_name()
        .map_or(path.clone(), |n| n.to_string_lossy().into_owned());
    announce(
        &format!("{file} queued for the next recording."),
        Tone::Info,
    );
    with_session(move |s| async move {
        let said = s.play_into_microphone(path).await?;
        say(said, Tone::Info);
        Ok(())
    });
}

/// A menu is opening: names its items for the selected device.
pub fn menu_opening(menu: windows::Win32::UI::WindowsAndMessaging::HMENU) {
    let (device, _) = selected();
    let session = device.and_then(|d| session_if_open(&d.id));
    let on = session.as_ref().is_some_and(|s| s.microphone_on());
    let playing = session
        .as_ref()
        .is_some_and(|s| s.playing_into_microphone());
    crate::menu::name_microphone(menu, on, playing);
    let recording = session.as_ref().is_some_and(|s| s.recording_screen());
    crate::menu::name_recording(menu, recording);
    let bridged = selected().0.is_some_and(|d| d.speech_bridge);
    crate::menu::name_speech_bridge(menu, bridged);
}

fn speak_status() {
    let (download, device, busy, mode, gestures) = with(|app| {
        (
            app.download.clone(),
            app.selected().cloned(),
            app.selected().and_then(|d| app.busy.get(&d.id).cloned()),
            app.device_mode.is_some(),
            app.gesture_mode,
        )
    });
    if let Some((what, percent)) = download {
        announce(&format!("Downloading {what}: {percent}%."), Tone::Info);
        return;
    }
    let Some(device) = device else {
        announce("No device is selected.", Tone::Info);
        return;
    };
    let mut parts = vec![format!("{}, {}.", device.name, device.android)];
    match &busy {
        Some(doing) => parts.push(format!("{doing}.")),
        None => parts.push(
            if device.running {
                "Running."
            } else {
                "Stopped."
            }
            .into(),
        ),
    }
    parts.push(
        if !mode {
            "The keyboard is on Windows."
        } else if gestures {
            "Gesture mode is on."
        } else {
            "The keyboard is in Android."
        }
        .into(),
    );
    if !device.running || busy.is_some() {
        announce(&parts.join(" "), Tone::Info);
        return;
    }
    with_session(move |s| async move {
        parts.push(s.screen_reader_status().await?);
        say(parts.join(" "), Tone::Info);
        Ok(())
    });
}

fn copy_device_clipboard() {
    with_session(|s| async move {
        let text = s.device_clipboard().await?;
        if text.is_empty() {
            say("The device's clipboard is empty.", Tone::Info);
            return Ok(());
        }
        run_on_ui(move || {
            let hwnd = with(|app| app.hwnd);
            if ui::set_clipboard_text(hwnd, &text) {
                announce(
                    &format!("Copied from the device: {}", preview(&text)),
                    Tone::Success,
                );
            } else {
                announce("Windows' clipboard couldn't be changed.", Tone::Failure);
            }
        });
        Ok(())
    });
}

/// Puts Windows' clipboard on the device's, or with `type_it`, types it,
/// for fields that block pasting.
fn send_clipboard(type_it: bool) {
    let hwnd = with(|app| app.hwnd);
    let Some(text) = ui::clipboard_text(hwnd).filter(|t| !t.is_empty()) else {
        announce("Windows' clipboard has no text.", Tone::Failure);
        return;
    };
    // Android wants its own line endings.
    let text = text.replace("\r\n", "\n");
    with_session(move |s| async move {
        if type_it {
            let count = text.chars().count();
            s.type_text(text).await?;
            say(format!("Typed {count} characters."), Tone::Success);
        } else {
            s.set_device_clipboard(text.clone()).await?;
            say(
                format!("Sent to the device's clipboard: {}", preview(&text)),
                Tone::Success,
            );
        }
        Ok(())
    });
}

/// The start of some text, to read out.
pub(crate) fn preview(text: &str) -> String {
    if text.chars().count() > 80 {
        format!("{}…", text.chars().take(80).collect::<String>())
    } else {
        text.to_string()
    }
}

fn screenshot() {
    let (device, hwnd) = selected();
    let Some(device) = device else { return };
    let Some(path) = ui::save_file(
        hwnd,
        "Save Screenshot",
        &format!("{} screenshot.png", device.name),
        &[("PNG images", "*.png")],
        "png",
    ) else {
        return;
    };
    with_session(move |s| async move {
        s.screenshot(path.clone()).await?;
        let name = std::path::Path::new(&path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or(path);
        say(format!("Saved the screenshot as {name}."), Tone::Success);
        Ok(())
    });
}

/// Records the selected device's screen and sound into a WebM file, or
/// stops.
fn toggle_recording() {
    let (device, hwnd) = selected();
    let Some(device) = device else { return };
    if let Some(session) = session_if_open(&device.id)
        && session.recording_screen()
    {
        with_session(|s| async move {
            say(s.stop_recording().await?, Tone::Success);
            Ok(())
        });
        return;
    }
    let Some(path) = ui::save_file(
        hwnd,
        "Record the screen and its sound, for up to three minutes",
        &format!("{} recording.webm", device.name),
        &[("WebM videos", "*.webm")],
        "webm",
    ) else {
        return;
    };
    with_session(move |s| async move {
        say(s.start_recording(path).await?, Tone::Success);
        Ok(())
    });
}

// MARK: - Installing apps

fn install_app() {
    let hwnd = with(|app| app.hwnd);
    let files = ui::open_files(
        hwnd,
        "Choose apps to install",
        &[("Android apps", "*.apk")],
        true,
    );
    if !files.is_empty() {
        install_paths(files);
    }
}

/// Installs apps, from Install App, pasted or dropped on the window. With
/// more than one device running, asks which to install on.
fn install_paths(paths: Vec<String>) {
    let apks: Vec<String> = paths
        .into_iter()
        .filter(|p| p.to_lowercase().ends_with(".apk"))
        .collect();
    if apks.is_empty() {
        announce(
            "Only app packages, ending in .apk, can be installed.",
            Tone::Failure,
        );
        return;
    }
    let (running, selection, hwnd) = with(|app| {
        (
            app.devices
                .iter()
                .filter(|d| d.running)
                .cloned()
                .collect::<Vec<_>>(),
            app.selection.clone(),
            app.hwnd,
        )
    });
    match running.len() {
        0 => announce(
            "Start a device first, to install apps on it.",
            Tone::Failure,
        ),
        1 => install(apks, vec![running[0].id.clone()]),
        _ => {
            let mut form = Form::new("Install on Which Devices?").text(&format!(
                "Install {} on:",
                if apks.len() == 1 {
                    file_name(&apks[0])
                } else {
                    format!("{} apps", apks.len())
                }
            ));
            for device in &running {
                form = form.field(Field::Check {
                    label: format!("{}, {}", device.name, device.android),
                    checked: Some(&device.id) == selection.as_ref(),
                });
            }
            let answer = forms::run(
                hwnd,
                form.button("Install", 1, Role::Default)
                    .button("Cancel", 0, Role::Cancel),
            );
            if answer.button != 1 {
                return;
            }
            let ids: Vec<String> = running
                .iter()
                .zip(answer.values.iter().skip(1))
                .filter(|(_, v)| v.checked())
                .map(|(d, _)| d.id.clone())
                .collect();
            if ids.is_empty() {
                announce(
                    "No devices were chosen, so nothing was installed.",
                    Tone::Info,
                );
            } else {
                install(apks, ids);
            }
        }
    }
}

pub(crate) fn file_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string())
}

/// Installs apps on devices, one at a time. An app's parts are asked about
/// once, on the first device that hasn't an answer, and the same answers are
/// used on the others.
pub(crate) fn install(paths: Vec<String>, ids: Vec<String>) {
    let names: HashMap<String, String> = with(|app| {
        ids.iter()
            .map(|id| (id.clone(), app.device_name(id)))
            .collect()
    });
    spawn(async move {
        let mut answers: HashMap<String, HashMap<String, bool>> = HashMap::new();
        for path in &paths {
            let file = file_name(path);
            for id in &ids {
                let name = names.get(id).cloned().unwrap_or_default();
                let result = async {
                    let session = session_for(id.clone()).await?;
                    say(
                        if ids.len() > 1 {
                            format!("Installing {file} on {name}.")
                        } else {
                            format!("Installing {file}.")
                        },
                        Tone::Info,
                    );
                    let result = session.install_apk(path.clone()).await?;
                    let known = answers.get(&result.package).cloned().unwrap_or_default();
                    let made =
                        after_install(result.package.clone(), result.parts, &session, known).await;
                    answers.entry(result.package).or_default().extend(made);
                    Ok::<(), AaeError>(())
                }
                .await;
                if let Err(e) = result {
                    say(format!("{file} on {name}: {e}"), Tone::Failure);
                }
            }
        }
    });
}

/// Says what was installed, and asks about any of its parts not chosen about
/// yet on this device, unless `known` already answers them. Returns the
/// answers given, by component.
async fn after_install(
    package: String,
    parts: Vec<AppPartInfo>,
    session: &Arc<Session>,
    known: HashMap<String, bool>,
) -> HashMap<String, bool> {
    let mut said = vec![format!("Installed {package}.")];
    for part in &parts {
        if let Some(on) = part.choice {
            said.push(format!(
                "{} is {}, as you chose before.",
                part.name,
                if on { "on" } else { "off" }
            ));
        }
    }
    say(said.join(" "), Tone::Success);
    let undecided: Vec<AppPartInfo> = parts.into_iter().filter(|p| p.choice.is_none()).collect();
    if undecided.is_empty() {
        return HashMap::new();
    }
    let mut choices: Vec<AppChoice> = undecided
        .iter()
        .filter_map(|part| {
            known.get(&part.component).map(|&on| AppChoice {
                part: part.clone(),
                on,
            })
        })
        .collect();
    let unknown: Vec<AppPartInfo> = undecided
        .into_iter()
        .filter(|p| !known.contains_key(&p.component))
        .collect();
    if !unknown.is_empty() {
        choices.extend(on_ui(move || ask_parts(&package, unknown)).await);
    }
    let made: HashMap<String, bool> = choices
        .iter()
        .map(|c| (c.part.component.clone(), c.on))
        .collect();
    let on: Vec<String> = choices
        .iter()
        .filter(|c| c.on)
        .map(|c| c.part.name.clone())
        .collect();
    match session.set_app_choices(choices).await {
        Ok(()) => say(
            if on.is_empty() {
                "Left them all off.".to_string()
            } else {
                format!("Turned on {}.", on.join(", "))
            },
            Tone::Success,
        ),
        Err(e) => say_error(e),
    }
    made
}

/// Asks which of an app's special parts to turn on. Accessibility services
/// start on, the rest off. The answers are remembered for this device.
fn ask_parts(package: &str, parts: Vec<AppPartInfo>) -> Vec<AppChoice> {
    let hwnd = with(|app| app.hwnd);
    let mut form = Form::new(&format!("Turn on parts of {package}?"));
    for part in &parts {
        form = form.field(Field::Check {
            label: format!("{}, {}", part.name, part.kind_description),
            checked: matches!(part.kind, AppPartKind::AccessibilityService),
        });
    }
    let answer = forms::run(
        hwnd,
        form.button("Done", 1, Role::Default)
            .button("Leave All Off", 0, Role::Cancel),
    );
    parts
        .into_iter()
        .zip(answer.values.iter().skip(1))
        .map(|(part, value)| AppChoice {
            part,
            on: answer.button == 1 && value.checked(),
        })
        .collect()
}

/// Chooses a screen reader build, then asks which devices to install it on.
fn install_screen_reader_build() {
    let Some(engine) = engine() else { return };
    let hwnd = with(|app| app.hwnd);
    let Some(path) = ui::open_files(
        hwnd,
        "Choose a screen reader build to install",
        &[("Android apps", "*.apk")],
        false,
    )
    .into_iter()
    .next() else {
        return;
    };
    let package = match engine.apk_package(path.clone()) {
        Ok(p) => p,
        Err(e) => return announce(&e.to_string(), Tone::Failure),
    };
    let devices = with(|app| app.devices.clone());
    let mut form = Form::new(&format!("Install {package}")).text(
        "Which devices should get this build? A new build of a device's screen reader keeps its settings. Stopped devices get it when they next start.",
    );
    for device in &devices {
        let mut label = vec![format!("{}, {}", device.name, device.android)];
        if let Some(reader) = &device.screen_reader {
            label.push(if *reader == package {
                "uses this screen reader".into()
            } else {
                format!("uses {reader}")
            });
        }
        if !device.running {
            label.push("stopped".into());
        }
        form = form.field(Field::Check {
            label: label.join(", "),
            checked: device.screen_reader.as_ref() == Some(&package),
        });
    }
    let count = devices.len();
    form.action = Some(Box::new(move |_, handle| {
        for i in 0..count {
            handle.set_checked(i + 1, true);
        }
    }));
    let answer = forms::run(
        hwnd,
        form.button("Choose &All", 2, Role::Action)
            .button("Install", 1, Role::Default)
            .button("Cancel", 0, Role::Cancel),
    );
    if answer.button != 1 {
        return;
    }
    let chosen: Vec<DeviceInfo> = devices
        .into_iter()
        .zip(answer.values.iter().skip(1))
        .filter(|(_, v)| v.checked())
        .map(|(d, _)| d)
        .collect();
    if chosen.is_empty() {
        return;
    }
    spawn(async move {
        for device in chosen {
            say(format!("Installing on {}.", device.name), Tone::Info);
            match engine
                .install_screen_reader_build(device.id.clone(), path.clone(), false)
                .await
            {
                Ok(said) => say(said, Tone::Success),
                Err(e) => {
                    let message = e.to_string();
                    if !message.contains("signed differently") {
                        say(format!("{}: {message}", device.name), Tone::Failure);
                        continue;
                    }
                    // A build signed differently can only replace the old
                    // one, losing its settings.
                    let name = device.name.clone();
                    let replace = on_ui(move || {
                        let hwnd = with(|app| app.hwnd);
                        forms::confirm(
                            hwnd,
                            &format!("Replace the screen reader on {name}?"),
                            &format!("This build is signed differently from the one on {name}, so it can't be installed over it. Replacing it removes the old one first, and with it the screen reader's settings."),
                            "&Replace",
                        )
                    })
                    .await;
                    if !replace {
                        continue;
                    }
                    match engine
                        .install_screen_reader_build(device.id.clone(), path.clone(), true)
                        .await
                    {
                        Ok(said) => say(said, Tone::Success),
                        Err(e) => say(format!("{}: {e}", device.name), Tone::Failure),
                    }
                }
            }
        }
        run_on_ui(|| {
            with(|app| {
                app.refresh();
                app.render();
            })
        });
    });
}

// MARK: - Sound

/// Checks the selected device's audio reaches AAE with a muted test tone,
/// and offers to restart AAE's audio if not.
fn check_audio() {
    let (device, _) = selected();
    let Some(device) = device else { return };
    announce(&format!("Checking {}'s sound.", device.name), Tone::Info);
    with_session(move |s| async move {
        let result = s.check_audio(true).await?;
        if result.working {
            let id = device.id.clone();
            run_on_ui(move || {
                with(|app| app.sound_problems.remove(&id));
            });
            say(result.message, Tone::Success);
            return Ok(());
        }
        say(result.message.clone(), Tone::Failure);
        let name = device.name.clone();
        let restart = on_ui(move || {
            let hwnd = with(|app| app.hwnd);
            forms::confirm(
                hwnd,
                &format!("Restart {name}'s audio?"),
                &format!("{} Restarting AAE's audio reconnects to the device's sound and to this computer's output. The device itself keeps running.", result.message),
                "&Restart Audio",
            )
        })
        .await;
        if !restart {
            return Ok(());
        }
        s.restart_audio(settings::get().correct_pitch).await?;
        run_on_ui(|| with(|app| app.apply_audio_focus()));
        let again = s.check_audio(true).await?;
        if again.working {
            let id = device.id.clone();
            run_on_ui(move || {
                with(|app| app.sound_problems.remove(&id));
            });
            say(
                format!("Restarted the audio. {}", again.message),
                Tone::Success,
            );
        } else {
            say(
                format!(
                    "Restarted the audio, but: {} Restarting the device may help.",
                    again.message
                ),
                Tone::Failure,
            );
        }
        Ok(())
    });
}

/// After a device starts, checks its sound once, without a test tone, which
/// would mean muting the screen reader's first words.
fn check_sound_after_start(id: String) {
    let Some(session) = session_if_open(&id) else {
        return;
    };
    spawn(async move {
        // Long enough for the screen reader to have said something.
        tokio::time::sleep(Duration::from_secs(8)).await;
        if let Ok(result) = session.check_audio(false).await
            && !result.working
        {
            run_on_ui(move || report_sound_problem(id, result.message));
        }
    });
}

/// Every half minute, checks each device AAE is playing without making a
/// sound, and says once when one's sound has stopped reaching AAE.
/// Every two seconds, notices devices started or stopped elsewhere, such as
/// with the aae command. A device this app was connected to that was started
/// again is reconnected, so its sound plays here again; one that's stopped
/// is let go. Devices it's starting or stopping itself are left alone.
fn watch_devices() {
    spawn(async {
        let mut shown = String::new();
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let Some(engine) = engine() else { continue };
            let Ok(fresh) = engine.devices() else {
                continue;
            };
            let signature = fresh
                .iter()
                .map(|d| format!("{} {:?}", d.id, d.instance))
                .collect::<Vec<_>>()
                .join(",");
            if signature != shown {
                shown = signature;
                run_on_ui(|| {
                    with(|app| {
                        app.refresh();
                        app.render();
                    })
                });
            }
            for (id, session) in open_sessions() {
                let device = fresh.iter().find(|d| d.id == id);
                if device.and_then(|d| d.instance.clone()) == Some(session.instance()) {
                    continue;
                }
                let busy = {
                    let id = id.clone();
                    on_ui(move || with(|app| app.busy.contains_key(&id))).await
                };
                if busy {
                    continue;
                }
                let name = device.map_or_else(|| "A device".to_string(), |d| d.name.clone());
                let running = device.is_some_and(|d| d.running);
                {
                    let id = id.clone();
                    on_ui(move || {
                        if with(|app| app.device_mode.as_deref() == Some(id.as_str())) {
                            leave_device_mode(true);
                        }
                        close_session(&id);
                    })
                    .await;
                }
                if !running {
                    say(format!("{name} was stopped outside this app."), Tone::Info);
                } else if session_for(id).await.is_ok() {
                    say(
                        format!("{name} was restarted outside this app. Reconnected."),
                        Tone::Info,
                    );
                }
            }
        }
    });
}

fn watch_sound() {
    spawn(async {
        loop {
            tokio::time::sleep(Duration::from_secs(30)).await;
            for (id, session) in open_sessions() {
                let Ok(result) = session.check_audio(false).await else {
                    continue;
                };
                run_on_ui(move || {
                    if result.working {
                        with(|app| app.sound_problems.remove(&id));
                    } else {
                        report_sound_problem(id, result.message);
                    }
                });
            }
        }
    });
}

fn report_sound_problem(id: String, message: String) {
    let Some(name) = with(|app| {
        app.sound_problems
            .insert(id.clone())
            .then(|| app.device_name(&id))
    }) else {
        return;
    };
    announce(
        &format!("{name}: {message} Choose Check Audio in the Device menu to restart it."),
        Tone::Failure,
    );
}

// MARK: - Help and settings

fn self_test() {
    let Some(engine) = engine() else { return };
    announce("Running the self-test.", Tone::Info);
    spawn(async move {
        let mut results: Vec<(String, CheckOutcome, String)> = engine
            .self_test()
            .await
            .into_iter()
            .map(|c| (c.name, c.outcome, c.detail))
            .collect();
        let capture = on_ui(keyboard::can_capture).await;
        results.insert(
            0,
            if capture {
                (
                    "Keyboard capture".into(),
                    CheckOutcome::Passed,
                    "AAE can watch the keyboard in device mode, so Windows' shortcuts reach Android.".into(),
                )
            } else {
                (
                    "Keyboard capture".into(),
                    CheckOutcome::Failed,
                    "Windows wouldn't let AAE watch the keyboard, so device mode can't work.".into(),
                )
            },
        );
        let names: HashMap<String, String> = on_ui(|| {
            with(|app| {
                app.devices
                    .iter()
                    .map(|d| (d.id.clone(), d.name.clone()))
                    .collect()
            })
        })
        .await;
        for (id, session) in open_sessions() {
            if let Ok(check) = session.check_audio(false).await {
                results.push((
                    format!("{}: sound", names.get(&id).cloned().unwrap_or_default()),
                    if check.working {
                        CheckOutcome::Passed
                    } else {
                        CheckOutcome::Failed
                    },
                    check.message,
                ));
            }
        }
        let problems: Vec<&(String, CheckOutcome, String)> = results
            .iter()
            .filter(|r| r.1 == CheckOutcome::Failed)
            .collect();
        let warnings: Vec<&(String, CheckOutcome, String)> = results
            .iter()
            .filter(|r| r.1 == CheckOutcome::Warning)
            .collect();
        let summary = if problems.is_empty() && warnings.is_empty() {
            format!("All {} checks passed.", results.len())
        } else {
            let found: Vec<String> = problems
                .iter()
                .chain(&warnings)
                .map(|r| format!("{}: {}", r.0, r.2))
                .collect();
            format!(
                "{} problems and {} warnings. {}",
                problems.len(),
                warnings.len(),
                found.join(" ")
            )
        };
        let tone = if !problems.is_empty() {
            Tone::Failure
        } else if warnings.is_empty() {
            Tone::Success
        } else {
            Tone::Info
        };
        let text: Vec<String> = results
            .iter()
            .map(|(name, outcome, detail)| {
                let outcome = match outcome {
                    CheckOutcome::Passed => "passed",
                    CheckOutcome::Warning => "warning",
                    CheckOutcome::Failed => "failed",
                };
                format!("{name}: {outcome}. {detail}")
            })
            .collect();
        run_on_ui(move || {
            announce(&summary, tone);
            let hwnd = with(|app| app.hwnd);
            forms::run(
                hwnd,
                Form {
                    width: 620,
                    ..Form::new("Self-Test")
                }
                .field(Field::Area {
                    label: "Results".into(),
                    value: text.join("\n"),
                    read_only: true,
                    lines: 16,
                })
                .button("Close", 0, Role::Cancel),
            );
        });
    });
}

/// The version people see, such as "0.3.0", or for a development build,
/// "0.3.0 (development build 12, from 1a2b3c4)".
pub fn display_version() -> String {
    match option_env!("AAE_BUILD_LABEL").filter(|l| !l.is_empty()) {
        Some(label) => format!("{} ({label})", env!("CARGO_PKG_VERSION")),
        None => env!("CARGO_PKG_VERSION").to_string(),
    }
}

fn version() -> String {
    format!(
        "{}, build {}, Windows app",
        display_version(),
        crate::updates::BUILD_NUMBER
    )
}

/// Saves a diagnostic report for a bug report, where the user chooses.
fn diagnostic_report() {
    let Some(engine) = engine() else { return };
    let hwnd = with(|app| app.hwnd);
    forms::message(
        hwnd,
        "Save Diagnostic Report",
        "The report has AAE's log and details of this computer and your devices, with your home folder and computer name taken out. It never includes what you typed on a device. It's plain text, so you can read it before sending it.",
    );
    let stamp = {
        let secs = now();
        format!("{}", secs)
    };
    let Some(path) = ui::save_file(
        hwnd,
        "Save Diagnostic Report",
        &format!("AAE report {stamp}.txt"),
        &[("Text files", "*.txt")],
        "txt",
    ) else {
        return;
    };
    let report = engine.diagnostic_report(version());
    match std::fs::write(&path, report.replace('\n', "\r\n")) {
        Ok(()) => announce(&format!("Saved {}.", file_name(&path)), Tone::Success),
        Err(e) => announce(&e.to_string(), Tone::Failure),
    }
}

fn edit_settings() {
    let hwnd = with(|app| app.hwnd);
    let current = settings::get();
    let answer = forms::run(
        hwnd,
        Form::new("Settings")
            .field(Field::Check {
                label: "Play a &sound before each announcement".into(),
                checked: current.play_sounds,
            })
            .field(Field::Check {
                label: "When several devices are running, play only the one you're &using".into(),
                checked: current.play_only_in_use,
            })
            .field(Field::Check {
                label: "&Correct the pitch of older Android versions".into(),
                checked: current.correct_pitch,
            })
            .field(Field::Choice {
                label: "&Return to Windows with".into(),
                items: keyboard::RETURN_SHORTCUTS
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
                selected: current.return_shortcut as usize,
            })
            .button("OK", 1, Role::Default)
            .button("Cancel", 0, Role::Cancel),
    );
    if answer.button != 1 {
        return;
    }
    let pitch_changed = answer.values[2].checked() != current.correct_pitch;
    settings::change(|s| {
        s.play_sounds = answer.values[0].checked();
        s.play_only_in_use = answer.values[1].checked();
        s.correct_pitch = answer.values[2].checked();
        s.return_shortcut = answer.values[4].choice().unwrap_or(0).min(2) as u8;
    });
    with(|app| app.apply_audio_focus());
    if pitch_changed {
        // Applies to audio already playing.
        let correct = settings::get().correct_pitch;
        for (_, session) in open_sessions() {
            session.stop_audio();
            spawn(async move {
                let _ = session.start_audio(correct).await;
            });
        }
    }
}

fn about() {
    let hwnd = with(|app| app.hwnd);
    forms::message(
        hwnd,
        "About AAE",
        &format!(
            "Accessible Android Emulator {}\n\nCreate, run and test Android virtual devices with a screen reader.\n\nhttps://github.com/aaron-gh/accessible-android-emulator\n\nLicensed under the Apache License 2.0.",
            version()
        ),
    );
}
