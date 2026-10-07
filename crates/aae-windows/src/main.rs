//! AAE's Windows app: the Accessible Android Emulator with Windows' own
//! standard controls, over the same core as the Mac app. It's built on the
//! Mac, cross-compiled with MinGW: see the README.

#![cfg_attr(windows, windows_subsystem = "windows")]

#[allow(dead_code)]
mod gestures;

#[cfg(windows)]
mod app;
#[cfg(windows)]
mod forms;
#[cfg(windows)]
mod keyboard;
#[cfg(windows)]
mod menu;
#[cfg(windows)]
mod panels;
#[cfg(windows)]
mod screen_readers;
#[cfg(windows)]
mod settings;
#[cfg(windows)]
mod speech;
#[cfg(windows)]
mod speech_bridge;
#[cfg(windows)]
mod tools;
#[cfg(windows)]
mod ui;
#[cfg(windows)]
mod updates;

#[cfg(not(windows))]
fn main() {
    eprintln!("This is AAE's Windows app. On the Mac, use the Mac app in macos/.");
    std::process::exit(1);
}

#[cfg(windows)]
fn main() {
    if std::env::args().any(|a| a == "--serve") {
        serve_hidden();
    }
    window::run();
}

/// Serving at login (`aae daemon install` sets it up): runs `aae serve`, the
/// aae.exe next to this app, with no console window, which aae.exe started
/// directly would show at every login, until it stops. The app's own window
/// doesn't open.
#[cfg(windows)]
fn serve_hidden() -> ! {
    use std::os::windows::process::CommandExt;
    let aae = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|d| d.join("aae.exe")));
    let log = aae_core::paths::data_dir().join("logs");
    let _ = std::fs::create_dir_all(&log);
    let output = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log.join("serve.log"));
    let status = match (aae, output) {
        (Some(aae), Ok(output)) => {
            let errors = output.try_clone();
            let mut command = std::process::Command::new(aae);
            command
                .arg("serve")
                .stdin(std::process::Stdio::null())
                .stdout(output)
                // No console window.
                .creation_flags(0x0800_0000);
            if let Ok(errors) = errors {
                command.stderr(errors);
            }
            command.status().map(|s| s.code().unwrap_or(1)).unwrap_or(1)
        }
        _ => 1,
    };
    std::process::exit(status);
}

/// Records a panic caught on its way out of a Windows callback in AAE's log.
#[cfg(windows)]
pub fn log_panic(panic: &Box<dyn std::any::Any + Send>) {
    let message = panic
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_else(|| "unknown".into());
    tracing::error!("the Windows app hit a bug: {message}");
}

#[cfg(windows)]
mod window {
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
    use windows::Win32::UI::Controls::{
        ICC_BAR_CLASSES, ICC_PROGRESS_CLASS, ICC_STANDARD_CLASSES, ICC_TREEVIEW_CLASSES,
        INITCOMMONCONTROLSEX, InitCommonControlsEx,
    };
    use windows::Win32::UI::HiDpi::GetDpiForSystem;
    use windows::Win32::UI::Shell::{DragAcceptFiles, DragFinish, HDROP};
    use windows::Win32::UI::WindowsAndMessaging::*;
    use windows::core::w;

    use std::sync::atomic::{AtomicIsize, Ordering};

    use windows::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled;

    use crate::{app, menu, panels, speech, ui, updates};

    /// The control that had the focus when AAE's window was last active.
    static LAST_FOCUS: AtomicIsize = AtomicIsize::new(0);

    pub fn run() {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let _ = InitCommonControlsEx(&INITCOMMONCONTROLSEX {
                dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
                dwICC: ICC_STANDARD_CLASSES
                    | ICC_BAR_CLASSES
                    | ICC_PROGRESS_CLASS
                    | ICC_TREEVIEW_CLASSES,
            });
            let class = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(window_proc),
                hInstance: ui::hinstance(),
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                // The dialog colour, behind standard controls.
                hbrBackground: windows::Win32::Graphics::Gdi::HBRUSH(
                    (windows::Win32::Graphics::Gdi::COLOR_BTNFACE.0 + 1) as usize as *mut _,
                ),
                lpszClassName: w!("AAEMain"),
                hIcon: LoadIconW(None, IDI_APPLICATION).unwrap_or_default(),
                ..Default::default()
            };
            RegisterClassExW(&class);
            let (menu_bar, shortcuts) = menu::build();
            let hwnd = CreateWindowExW(
                WS_EX_CONTROLPARENT,
                w!("AAEMain"),
                w!("Accessible Android Emulator"),
                WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                720 * GetDpiForSystem() as i32 / 96,
                560 * GetDpiForSystem() as i32 / 96,
                None,
                Some(menu_bar),
                Some(ui::hinstance()),
                None,
            )
            .expect("AAE's window could not be created");
            app::start_app(hwnd);
            DragAcceptFiles(hwnd, true);
            let _ = ShowWindow(hwnd, SW_SHOW);
            app::resized();
            app::focus_start();
            updates::start(&app::display_version());

            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
                // The window the message is for: the main window, where the
                // menu's shortcuts work, or a tool window, where they don't,
                // so Delete or Control-V in a text field does what it says,
                // unless it has no text fields, as a device's own window.
                let root = GetAncestor(msg.hwnd, GA_ROOT);
                if root == hwnd || msg.hwnd.is_invalid() {
                    if TranslateAcceleratorW(hwnd, shortcuts, &msg) != 0 {
                        continue;
                    }
                    if IsDialogMessageW(hwnd, &msg).as_bool() {
                        continue;
                    }
                } else if panels::is_panel(root)
                    && ((panels::wants_menu_shortcuts(root)
                        && TranslateAcceleratorW(hwnd, shortcuts, &msg) != 0)
                        || panels::shortcut(root, &msg)
                        || IsDialogMessageW(root, &msg).as_bool())
                {
                    continue;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }

    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        // A panic can't cross into Windows, which would end AAE at once; it's
        // logged instead, and the message treated as handled.
        std::panic::catch_unwind(|| unsafe { handle(hwnd, msg, wparam, lparam) }).unwrap_or_else(
            |panic| {
                crate::log_panic(&panic);
                LRESULT(0)
            },
        )
    }

    unsafe fn handle(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        match msg {
            ui::WM_RUN => {
                ui::run_queued();
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == speech::ANNOUNCE_TIMER => {
                speech::on_timer();
                LRESULT(0)
            }
            WM_COMMAND => {
                let id = (wparam.0 & 0xFFFF) as u16;
                let notification = ((wparam.0 >> 16) & 0xFFFF) as u32;
                // Menu items and shortcuts (0 and 1), and button presses.
                if lparam.0 == 0 || notification == BN_CLICKED || id == app::DEVICE_LIST {
                    app::command(id, notification);
                }
                LRESULT(0)
            }
            WM_INITMENUPOPUP => {
                app::menu_opening(HMENU(wparam.0 as *mut _));
                LRESULT(0)
            }
            WM_HSCROLL => {
                let control = HWND(lparam.0 as *mut _);
                if unsafe { GetDlgCtrlID(control) } == app::VOLUME_SLIDER as i32 {
                    // Only once the slider has settled, not for every step of a drag.
                    let code = (wparam.0 & 0xFFFF) as i32;
                    if code != 5 {
                        // TB_THUMBTRACK
                        app::volume_changed();
                    }
                }
                LRESULT(0)
            }
            WM_DROPFILES => {
                let drop = HDROP(wparam.0 as *mut _);
                let files = ui::dropped_files(drop);
                unsafe { DragFinish(drop) };
                app::files_dropped(files);
                LRESULT(0)
            }
            // Windows puts the focus on the window itself when it's
            // activated, so AAE remembers which control had it and puts it back.
            WM_ACTIVATE => {
                if wparam.0 & 0xFFFF == WA_INACTIVE as usize {
                    let focus = unsafe { windows::Win32::UI::Input::KeyboardAndMouse::GetFocus() };
                    if !focus.is_invalid() && unsafe { IsChild(hwnd, focus) }.as_bool() {
                        LAST_FOCUS.store(focus.0 as isize, Ordering::SeqCst);
                    }
                } else {
                    let last = HWND(LAST_FOCUS.load(Ordering::SeqCst) as *mut _);
                    if !last.is_invalid()
                        && unsafe { IsWindowVisible(last) }.as_bool()
                        && unsafe { IsWindowEnabled(last) }.as_bool()
                    {
                        ui::focus(last);
                    } else {
                        app::focus_start();
                    }
                }
                LRESULT(0)
            }
            WM_SIZE => {
                app::resized();
                LRESULT(0)
            }
            WM_DPICHANGED => {
                let rect = unsafe { &*(lparam.0 as *const windows::Win32::Foundation::RECT) };
                unsafe {
                    let _ = SetWindowPos(
                        hwnd,
                        None,
                        rect.left,
                        rect.top,
                        rect.right - rect.left,
                        rect.bottom - rect.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
                LRESULT(0)
            }
            WM_GETMINMAXINFO => {
                let info = unsafe { &mut *(lparam.0 as *mut MINMAXINFO) };
                info.ptMinTrackSize.x = ui::scaled(hwnd, 560);
                info.ptMinTrackSize.y = ui::scaled(hwnd, 420);
                LRESULT(0)
            }
            WM_NEXTDLGCTL => {
                ui::next_control(hwnd, wparam, lparam);
                LRESULT(0)
            }
            WM_CLOSE => {
                if app::closing() {
                    unsafe {
                        let _ = DestroyWindow(hwnd);
                    }
                }
                LRESULT(0)
            }
            _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::gestures::{GestureAction, GestureKeys};

    const UP: u16 = 0x148;
    const LEFT: u16 = 0x14B;
    const SPACE: u16 = 0x39;
    const TWO: u16 = 0x03;
    const H: u16 = 0x23;

    fn gesture(name: &str) -> GestureAction {
        GestureAction::Gesture(name.into())
    }

    #[test]
    fn arrows_swipe_when_released() {
        let mut keys = GestureKeys::default();
        assert!(keys.handle(UP, true).is_empty());
        assert_eq!(keys.handle(UP, false), vec![gesture("swipe-up")]);
    }

    #[test]
    fn a_second_arrow_makes_a_two_part_swipe_at_once() {
        let mut keys = GestureKeys::default();
        keys.handle(UP, true);
        assert_eq!(keys.handle(LEFT, true), vec![gesture("swipe-up-then-left")]);
        // The first arrow's release does nothing more.
        assert!(keys.handle(UP, false).is_empty());
    }

    #[test]
    fn repeats_are_ignored() {
        let mut keys = GestureKeys::default();
        assert_eq!(keys.handle(SPACE, true), vec![gesture("double-tap")]);
        assert!(keys.handle(SPACE, true).is_empty());
    }

    #[test]
    fn number_keys_add_fingers() {
        let mut keys = GestureKeys::default();
        keys.handle(TWO, true);
        assert_eq!(
            keys.handle(SPACE, true),
            vec![gesture("two-finger-double-tap")]
        );
    }

    #[test]
    fn holds_last_until_released() {
        let mut keys = GestureKeys::default();
        assert_eq!(
            keys.handle(H, true),
            vec![GestureAction::Press("double-tap-hold".into())]
        );
        assert_eq!(keys.handle(H, false), vec![GestureAction::Release]);
    }
}
