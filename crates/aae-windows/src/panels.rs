//! Tool windows that stay open beside the main one, such as the Shell or the
//! Device Log, as the Mac app's windows do. Each is an ordinary resizable
//! window of standard controls laid out top to bottom, with one list or text
//! area taking the space left. Tab moves between controls and Enter presses
//! the window's default button, as in a dialog; Escape closes it.
//!
//! What a window does lives in its [`Handler`]; the window itself only lays
//! out and routes.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

use crate::ui;

/// What a tool window does.
pub trait Handler {
    /// A button was pressed, or a list's selection changed.
    fn command(&mut self, panel: &Panel, id: u16);
    /// The window is closing.
    fn closed(&mut self) {}
    /// The window's timer fired; see [`Panel::every`].
    fn tick(&mut self, _panel: &Panel) {}
}

/// One row of a tool window.
enum Row {
    /// A heading or a sentence.
    Text(HWND),
    /// A label above a control.
    Labelled {
        label: HWND,
        control: HWND,
        height: i32,
    },
    /// A control on its own, such as a checkbox.
    Single { control: HWND, height: i32 },
    /// Buttons side by side.
    Buttons(Vec<(HWND, i32)>),
}

/// A tool window being built, then shown.
pub struct Panel {
    pub hwnd: HWND,
    rows: Vec<Row>,
    /// The row that takes the space left, if any.
    grow: Option<usize>,
    default_button: Option<u16>,
    next_id: u16,
}

struct State {
    panel: Panel,
    handler: Option<Box<dyn Handler>>,
}

thread_local! {
    static PANELS: RefCell<HashMap<isize, Rc<RefCell<State>>>> = RefCell::new(HashMap::new());
    /// Open windows by kind, such as "shell", so each kind opens once.
    static KINDS: RefCell<HashMap<&'static str, isize>> = RefCell::new(HashMap::new());
}

const CLASS: PCWSTR = w!("AAEPanel");
const MARGIN: i32 = 12;
const TIMER: usize = 7;

fn register() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        let class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(panel_proc),
            hInstance: ui::hinstance(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hbrBackground: windows::Win32::Graphics::Gdi::HBRUSH(
                (windows::Win32::Graphics::Gdi::COLOR_BTNFACE.0 + 1) as usize as *mut _,
            ),
            lpszClassName: CLASS,
            ..Default::default()
        };
        RegisterClassExW(&class);
    });
}

/// The open window of a kind, brought to the front, if there is one.
pub fn bring_forward(kind: &'static str) -> Option<HWND> {
    let hwnd = KINDS.with(|k| k.borrow().get(kind).copied())?;
    let hwnd = HWND(hwnd as *mut _);
    unsafe {
        if !IsWindow(Some(hwnd)).as_bool() {
            return None;
        }
        let _ = ShowWindow(hwnd, SW_RESTORE);
        let _ = SetForegroundWindow(hwnd);
    }
    Some(hwnd)
}

/// The open window of a kind, if there is one.
pub fn open_window(kind: &'static str) -> Option<HWND> {
    let hwnd = HWND(KINDS.with(|k| k.borrow().get(kind).copied())? as *mut _);
    unsafe { IsWindow(Some(hwnd)) }.as_bool().then_some(hwnd)
}

/// True for AAE's tool windows, so the message loop can give them dialog keys.
pub fn is_panel(hwnd: HWND) -> bool {
    PANELS.with(|p| p.borrow().contains_key(&(hwnd.0 as isize)))
}

impl Panel {
    /// Starts a tool window of a kind, with a title and a size at 96 dots per inch.
    pub fn new(kind: &'static str, title: &str, width: i32, height: i32) -> Panel {
        register();
        let title = ui::wide(title);
        let dpi = unsafe { windows::Win32::UI::HiDpi::GetDpiForSystem() } as i32;
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_CONTROLPARENT,
                CLASS,
                PCWSTR(title.as_ptr()),
                WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                width * dpi / 96,
                height * dpi / 96,
                None,
                None,
                Some(ui::hinstance()),
                None,
            )
            .unwrap_or_default()
        };
        KINDS.with(|k| k.borrow_mut().insert(kind, hwnd.0 as isize));
        Panel {
            hwnd,
            rows: Vec::new(),
            grow: None,
            default_button: None,
            next_id: 100,
        }
    }

    fn id(&mut self) -> u16 {
        self.next_id += 1;
        self.next_id
    }

    /// A heading or a sentence.
    pub fn text(&mut self, text: &str) -> HWND {
        let label = ui::label(self.hwnd, text);
        self.rows.push(Row::Text(label));
        label
    }

    /// A one-line text field, with its label above it.
    pub fn edit(&mut self, label: &str, value: &str) -> HWND {
        let label = ui::label(self.hwnd, label);
        let id = self.id();
        let control = ui::edit(self.hwnd, value, id);
        self.rows.push(Row::Labelled {
            label,
            control,
            height: 23,
        });
        control
    }

    /// A multi-line text area. With `grow`, it takes the space left.
    pub fn area(
        &mut self,
        label: &str,
        value: &str,
        read_only: bool,
        lines: i32,
        grow: bool,
    ) -> HWND {
        let label = ui::label(self.hwnd, label);
        let id = self.id();
        let control = ui::text_area(self.hwnd, value, id, read_only);
        self.push_growing(
            Row::Labelled {
                label,
                control,
                height: lines * 16 + 8,
            },
            grow,
        );
        control
    }

    /// A list. With `grow`, it takes the space left. Its id is the one its
    /// selection changes are reported with.
    pub fn list(&mut self, label: &str, id: u16, lines: i32, grow: bool) -> HWND {
        let label = ui::label(self.hwnd, label);
        let control = ui::list_box(self.hwnd, id);
        self.push_growing(
            Row::Labelled {
                label,
                control,
                height: lines * 16 + 8,
            },
            grow,
        );
        control
    }

    /// A drop-down list. Its id is the one its selection changes are reported with.
    pub fn choice(&mut self, label: &str, id: u16, items: &[String], selected: usize) -> HWND {
        let label = ui::label(self.hwnd, label);
        let control = ui::combo_box(self.hwnd, id);
        ui::set_combo_items(control, items, selected);
        self.rows.push(Row::Labelled {
            label,
            control,
            height: 23,
        });
        control
    }

    /// A checkbox. Its id is reported when it's ticked or unticked.
    pub fn check(&mut self, label: &str, id: u16, checked: bool) -> HWND {
        let control = ui::checkbox(self.hwnd, label, id, checked);
        self.rows.push(Row::Single {
            control,
            height: 20,
        });
        control
    }

    /// Buttons side by side, by label and id. The one given as `default`
    /// is pressed by Enter.
    pub fn buttons(&mut self, buttons: &[(&str, u16)], default: Option<u16>) -> Vec<HWND> {
        let mut row = Vec::new();
        let mut handles = Vec::new();
        for &(text, id) in buttons {
            let hwnd = ui::button(self.hwnd, text, id);
            if default == Some(id) {
                unsafe {
                    SetWindowLongW(
                        hwnd,
                        GWL_STYLE,
                        GetWindowLongW(hwnd, GWL_STYLE) | BS_DEFPUSHBUTTON,
                    );
                }
                self.default_button = Some(id);
            }
            let width = (text.chars().count() as i32 * 7 + 24).max(84);
            row.push((hwnd, width));
            handles.push(hwnd);
        }
        self.rows.push(Row::Buttons(row));
        handles
    }

    fn push_growing(&mut self, row: Row, grow: bool) {
        if grow {
            self.grow = Some(self.rows.len());
        }
        self.rows.push(row);
    }

    /// Shows the window, run by `handler`, with the focus on `focus`.
    pub fn show(self, handler: impl Handler + 'static, focus: Option<HWND>) {
        let hwnd = self.hwnd;
        let state = Rc::new(RefCell::new(State {
            panel: self,
            handler: Some(Box::new(handler)),
        }));
        PANELS.with(|p| p.borrow_mut().insert(hwnd.0 as isize, state.clone()));
        layout(&state.borrow().panel);
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetForegroundWindow(hwnd);
        }
        if let Some(control) = focus {
            ui::focus(control);
        }
    }

    pub fn set_title(&self, title: &str) {
        ui::set_text(self.hwnd, title);
    }

    /// Asks for [`Handler::tick`] every `ms` milliseconds, for windows that
    /// keep up with the device, such as the logs.
    pub fn every(&self, ms: u32) {
        unsafe {
            SetTimer(Some(self.hwnd), TIMER, ms, None);
        }
    }

    pub fn close(&self) {
        unsafe {
            let _ = PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }
}

/// Runs code on a kind's open window, if it's open, such as to show new
/// results that arrived in the background.
pub fn with_open<R>(
    kind: &'static str,
    work: impl FnOnce(&Panel, &mut dyn Handler) -> R,
) -> Option<R> {
    let hwnd = KINDS.with(|k| k.borrow().get(kind).copied())?;
    let state = PANELS.with(|p| p.borrow().get(&hwnd).cloned())?;
    let mut state = state.try_borrow_mut().ok()?;
    let State { panel, handler } = &mut *state;
    let handler = handler.as_mut()?;
    Some(work(panel, handler.as_mut()))
}

fn layout(panel: &Panel) {
    let mut rect = RECT::default();
    unsafe {
        let _ = GetClientRect(panel.hwnd, &mut rect);
    }
    let dpi = unsafe { GetDpiForWindow(panel.hwnd) }.max(96) as i32;
    let width = rect.right * 96 / dpi;
    let height = rect.bottom * 96 / dpi;
    let inner = (width - 2 * MARGIN).max(150);
    let row_height = |row: &Row| match row {
        Row::Text(label) => ui::text_height(*label, &ui::text(*label), inner).max(16) + 8,
        Row::Labelled { height, .. } => 19 + height + 10,
        Row::Single { height, .. } => height + 6,
        Row::Buttons(_) => 34,
    };
    let fixed: i32 = panel.rows.iter().map(row_height).sum();
    let spare = (height - 2 * MARGIN - fixed).max(0);
    let mut y = MARGIN;
    for (i, row) in panel.rows.iter().enumerate() {
        match row {
            Row::Text(label) => {
                let h = row_height(row) - 8;
                ui::place(*label, MARGIN, y, inner, h);
            }
            Row::Labelled {
                label,
                control,
                height,
            } => {
                ui::place(*label, MARGIN, y, inner, 16);
                let extra = if Some(i) == panel.grow { spare } else { 0 };
                // A drop-down's height includes its open list.
                let class = class_name(*control);
                let h = if class.eq_ignore_ascii_case("ComboBox") {
                    240
                } else {
                    height + extra
                };
                ui::place(*control, MARGIN, y + 19, inner, h);
                y += extra;
            }
            Row::Single { control, height } => ui::place(*control, MARGIN, y, inner, *height),
            Row::Buttons(buttons) => {
                let mut x = MARGIN;
                for &(button, w) in buttons {
                    ui::place(button, x, y, w, 26);
                    x += w + 8;
                }
            }
        }
        y += row_height(row);
    }
}

fn class_name(hwnd: HWND) -> String {
    let mut buffer = [0u16; 64];
    let length = unsafe { GetClassNameW(hwnd, &mut buffer) };
    String::from_utf16_lossy(&buffer[..length.max(0) as usize])
}

unsafe extern "system" fn panel_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    std::panic::catch_unwind(|| unsafe { handle(hwnd, msg, wparam, lparam) }).unwrap_or_else(
        |panic| {
            crate::log_panic(&panic);
            LRESULT(0)
        },
    )
}

unsafe fn handle(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let state = || PANELS.with(|p| p.borrow().get(&(hwnd.0 as isize)).cloned());
    match msg {
        WM_COMMAND => {
            let id = (wparam.0 & 0xFFFF) as u16;
            let notification = ((wparam.0 >> 16) & 0xFFFF) as u32;
            // Escape, through the dialog keys.
            if id == IDCANCEL.0 as u16 && lparam.0 == 0 {
                unsafe {
                    let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
                return LRESULT(0);
            }
            // Enter, through the dialog keys, presses the default button.
            // Without one, the handler gets IDOK, to act on the field Enter
            // was pressed in.
            let id = if id == IDOK.0 as u16 && lparam.0 == 0 {
                state()
                    .and_then(|s| s.try_borrow().ok().and_then(|s| s.panel.default_button))
                    .unwrap_or(id)
            } else {
                id
            };
            // Button presses, and list selections changing (LBN_SELCHANGE
            // and CBN_SELCHANGE are both 1).
            if lparam.0 == 0 || notification == BN_CLICKED || notification == 1 {
                dispatch(hwnd, |panel, handler| handler.command(panel, id));
            }
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TIMER => {
            dispatch(hwnd, |panel, handler| handler.tick(panel));
            LRESULT(0)
        }
        // IsDialogMessage asks for the default button.
        DM_GETDEFID => {
            match state().and_then(|s| s.try_borrow().ok().and_then(|s| s.panel.default_button)) {
                Some(id) => LRESULT((DC_HASDEFID as isize) << 16 | id as isize),
                None => LRESULT(0),
            }
        }
        WM_SIZE => {
            if let Some(state) = state()
                && let Ok(state) = state.try_borrow()
            {
                layout(&state.panel);
            }
            LRESULT(0)
        }
        WM_GETMINMAXINFO => {
            let info = unsafe { &mut *(lparam.0 as *mut MINMAXINFO) };
            info.ptMinTrackSize.x = ui::scaled(hwnd, 360);
            info.ptMinTrackSize.y = ui::scaled(hwnd, 240);
            LRESULT(0)
        }
        WM_CLOSE => {
            dispatch(hwnd, |_, handler| handler.closed());
            unsafe {
                let _ = KillTimer(Some(hwnd), TIMER);
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            PANELS.with(|p| p.borrow_mut().remove(&(hwnd.0 as isize)));
            KINDS.with(|k| k.borrow_mut().retain(|_, h| *h != hwnd.0 as isize));
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// Runs the window's handler, unless it's already running, as when it opened
/// a form whose messages arrive here.
fn dispatch(hwnd: HWND, work: impl FnOnce(&Panel, &mut dyn Handler)) {
    let Some(state) = PANELS.with(|p| p.borrow().get(&(hwnd.0 as isize)).cloned()) else {
        return;
    };
    // The handler is taken out while it runs, so it can reach the window.
    let Some(mut handler) = state
        .try_borrow_mut()
        .ok()
        .and_then(|mut s| s.handler.take())
    else {
        return;
    };
    if let Ok(s) = state.try_borrow() {
        work(&s.panel, handler.as_mut());
    }
    if let Ok(mut s) = state.try_borrow_mut() {
        s.handler = Some(handler);
    }
}
