//! Device mode: the keyboard belongs to Android until the user presses
//! Control-Windows-Escape (or the return shortcut chosen in Settings), as
//! Control-Command-Escape does on the Mac.
//!
//! A low-level keyboard hook sees every key before Windows acts on it, so
//! Windows' own shortcuts, such as Alt-Tab, the Windows key and Alt-F4, reach
//! Android instead. Only Control-Alt-Delete and Windows-L can't be taken.
//!
//! - Keys are captured only while AAE's window is in front. Clicking another
//!   window gives Windows its keyboard back until AAE is in front again.
//! - The screen reader keeps its own keys, as VoiceOver does on the Mac:
//!   while Insert or Caps Lock is held, which NVDA and JAWS use, keys go to
//!   Windows, so screen reader commands still work.
//! - Keys other programs send, such as a screen reader passing a key on, go
//!   to Windows.
//! - When device mode ends, keys still held on Android are released there,
//!   and the release of keys Windows never saw go down is kept from Windows,
//!   so the Windows key's release doesn't open Start.
//! - If AAE ever stops responding, Windows removes the hook by itself after a
//!   moment, so the keyboard can't be trapped.

use std::cell::RefCell;
use std::collections::HashSet;
use std::sync::Arc;

use aae_ffi::Session;
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{MAPVK_VK_TO_VSC, MapVirtualKeyW};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::gestures::{GestureAction, GestureKeys};
use crate::ui;

const EXTENDED: u16 = 0x100;
const ESCAPE: u16 = 0x01;
const CONTROL: [u16; 2] = [0x1D, EXTENDED | 0x1D];
const WINDOWS: [u16; 2] = [EXTENDED | 0x5B, EXTENDED | 0x5C];
const SHIFT: [u16; 2] = [0x2A, 0x36];
const ALT: [u16; 2] = [0x38, EXTENDED | 0x38];

/// The shortcuts that can bring the keyboard back to Windows: Escape with
/// Control and Windows, and Shift or Alt too if chosen in Settings, for when
/// an app under test needs Control-Windows-Escape. Never fewer modifiers, so
/// no key Android needs on its own is taken.
pub const RETURN_SHORTCUTS: [&str; 3] = [
    "Control Windows Escape",
    "Control Shift Windows Escape",
    "Control Alt Windows Escape",
];

/// The chosen one, as the hook reads it.
static RETURN_SHORTCUT: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// The return shortcut chosen, as it's said.
pub fn return_shortcut() -> &'static str {
    RETURN_SHORTCUTS
        [(crate::settings::get().return_shortcut as usize).min(RETURN_SHORTCUTS.len() - 1)]
}
/// Insert, on its own and on the keypad, and Caps Lock: screen reader keys.
const SCREEN_READER_KEYS: [u16; 3] = [EXTENDED | 0x52, 0x52, 0x3A];

struct Mode {
    session: Arc<Session>,
    gestures: Option<GestureKeys>,
    /// Keys held, as far as AAE knows.
    held: HashSet<u16>,
    /// Keys sent to Android and not yet released there.
    sent: HashSet<u16>,
    /// Keys whose press Windows was given, so their release goes there too.
    passed: HashSet<u16>,
}

struct Hook {
    handle: HHOOK,
    /// Device mode, while it's on.
    mode: Option<Mode>,
    /// Keys whose press was kept from Windows; their release is kept too,
    /// even after device mode ends.
    swallowed: HashSet<u16>,
}

thread_local! {
    static HOOK: RefCell<Option<Hook>> = const { RefCell::new(None) };
}

/// What happens in device mode, run on the window thread.
pub struct Events {
    /// The user pressed Control-Windows-Escape.
    pub on_escape: fn(),
    /// Gesture mode turned keys into these.
    pub on_gestures: fn(Vec<GestureAction>),
}

static EVENTS: std::sync::OnceLock<Events> = std::sync::OnceLock::new();

pub fn set_events(events: Events) {
    let _ = EVENTS.set(events);
}

/// Gives the keyboard to Android, to type, or with `gestures`, to perform
/// gestures. Returns false if Windows wouldn't install the hook.
pub fn enter(session: Arc<Session>, gestures: bool) -> bool {
    RETURN_SHORTCUT.store(
        crate::settings::get().return_shortcut,
        std::sync::atomic::Ordering::Relaxed,
    );
    HOOK.with(|cell| {
        let mut hook = cell.borrow_mut();
        if hook.is_none() {
            let Ok(handle) = (unsafe {
                SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), Some(ui::hinstance()), 0)
            }) else {
                return false;
            };
            *hook = Some(Hook {
                handle,
                mode: None,
                swallowed: HashSet::new(),
            });
        }
        if let Some(hook) = hook.as_mut() {
            hook.mode = Some(Mode {
                session,
                gestures: gestures.then(GestureKeys::default),
                held: HashSet::new(),
                sent: HashSet::new(),
                passed: HashSet::new(),
            });
        }
        true
    })
}

/// Gives the keyboard back to Windows. Returns what gesture mode still had
/// to do, such as lifting a held touch.
pub fn leave() -> Vec<GestureAction> {
    let actions = HOOK.with(|cell| {
        let mut hook = cell.borrow_mut();
        let Some(hook) = hook.as_mut() else {
            return Vec::new();
        };
        let Some(mut mode) = hook.mode.take() else {
            return Vec::new();
        };
        for key in mode.sent.drain() {
            mode.session
                .windows_key(key & 0xFF, key & EXTENDED != 0, false);
        }
        mode.gestures
            .as_mut()
            .map(|g| g.reset())
            .unwrap_or_default()
    });
    unhook_when_done();
    actions
}

pub fn is_on() -> bool {
    HOOK.with(|cell| cell.borrow().as_ref().is_some_and(|h| h.mode.is_some()))
}

/// True if Windows will install a keyboard hook, for the self-test.
pub fn can_capture() -> bool {
    unsafe {
        match SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), Some(ui::hinstance()), 0) {
            Ok(handle) => {
                let _ = UnhookWindowsHookEx(handle);
                true
            }
            Err(_) => false,
        }
    }
}

/// Removes the hook once device mode is off and every key it kept from
/// Windows is released.
fn unhook_when_done() {
    HOOK.with(|cell| {
        let mut hook = cell.borrow_mut();
        if hook
            .as_ref()
            .is_some_and(|h| h.mode.is_none() && h.swallowed.is_empty())
        {
            if let Some(h) = hook.take() {
                unsafe {
                    let _ = UnhookWindowsHookEx(h.handle);
                }
            }
        }
    });
}

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        let down = matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
        // After a bug, the key goes to Windows, so the keyboard isn't lost.
        let swallow = std::panic::catch_unwind(|| handle(info, down)).unwrap_or_else(|panic| {
            crate::log_panic(&panic);
            false
        });
        if swallow {
            return LRESULT(1);
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// Decides what happens to a key. Returns true to keep it from Windows.
fn handle(info: &KBDLLHOOKSTRUCT, down: bool) -> bool {
    let mut scan = info.scanCode as u16;
    if scan == 0 {
        scan = unsafe { MapVirtualKeyW(info.vkCode, MAPVK_VK_TO_VSC) } as u16;
    }
    let extended = info.flags.contains(LLKHF_EXTENDED);
    let key = (scan & 0xFF) | if extended { EXTENDED } else { 0 };
    let injected = info.flags.contains(LLKHF_INJECTED);

    let mut escape = false;
    let mut gesture_actions = Vec::new();
    let swallow = HOOK.with(|cell| {
        let Ok(mut hook) = cell.try_borrow_mut() else {
            return false;
        };
        let Some(hook) = hook.as_mut() else {
            return false;
        };
        // A release whose press Windows didn't see stays hidden from it.
        if !down && hook.swallowed.remove(&key) {
            if let Some(mode) = hook.mode.as_mut() {
                mode.held.remove(&key);
                release(mode, key, &mut gesture_actions);
            }
            return true;
        }
        let Some(mode) = hook.mode.as_mut() else {
            return false;
        };
        if injected || !in_front() {
            return false;
        }
        if !down {
            // Windows saw this key go down, before device mode or with a
            // screen reader key, so it sees it come up.
            mode.held.remove(&key);
            mode.passed.remove(&key);
            return false;
        }
        mode.held.insert(key);
        // The screen reader's keys, and anything pressed with them, go to Windows.
        if mode.passed.contains(&key)
            || SCREEN_READER_KEYS.contains(&key)
            || SCREEN_READER_KEYS.iter().any(|k| mode.passed.contains(k))
        {
            mode.passed.insert(key);
            return false;
        }
        hook.swallowed.insert(key);
        let held = |keys: &[u16; 2]| keys.iter().any(|k| mode.held.contains(k));
        let extra = match RETURN_SHORTCUT.load(std::sync::atomic::Ordering::Relaxed) {
            1 => held(&SHIFT),
            2 => held(&ALT),
            _ => true,
        };
        if key == ESCAPE && held(&CONTROL) && held(&WINDOWS) && extra {
            escape = true;
            return true;
        }
        match mode.gestures.as_mut() {
            Some(gestures) => gesture_actions.extend(gestures.handle(key, true)),
            None => {
                if mode.session.windows_key(key & 0xFF, extended, true) {
                    mode.sent.insert(key);
                }
            }
        }
        true
    });
    if escape && let Some(events) = EVENTS.get() {
        let on_escape = events.on_escape;
        ui::run_on_ui(on_escape);
    }
    if !gesture_actions.is_empty()
        && let Some(events) = EVENTS.get()
    {
        let on_gestures = events.on_gestures;
        ui::run_on_ui(move || on_gestures(gesture_actions));
    }
    if !down {
        unhook_when_done_later();
    }
    swallow
}

fn release(mode: &mut Mode, key: u16, gesture_actions: &mut Vec<GestureAction>) {
    match mode.gestures.as_mut() {
        Some(gestures) => gesture_actions.extend(gestures.handle(key, false)),
        None => {
            if mode.sent.remove(&key) {
                mode.session
                    .windows_key(key & 0xFF, key & EXTENDED != 0, false);
            }
        }
    }
}

/// The hook can't remove itself while it runs, so that waits for the window thread.
fn unhook_when_done_later() {
    let done = HOOK.with(|cell| {
        cell.try_borrow()
            .ok()
            .and_then(|h| {
                h.as_ref()
                    .map(|h| h.mode.is_none() && h.swallowed.is_empty())
            })
            .unwrap_or(false)
    });
    if done {
        ui::run_on_ui(unhook_when_done);
    }
}

/// True when AAE's main window is the one in front.
fn in_front() -> bool {
    let front = unsafe { GetForegroundWindow() };
    front == ui::main_window()
}
