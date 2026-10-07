//! Modal forms: questions, prompts and confirmations. Each is a real Windows
//! dialog, so screen readers announce it as one and read its text, with
//! standard controls laid out top to bottom and buttons along the bottom.
//! Tab moves between controls, Enter presses the default button, if there is
//! one, and Escape the cancel button.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::ui;

pub enum Field {
    /// A paragraph.
    Text(String),
    /// A one-line text field.
    Edit {
        label: String,
        value: String,
    },
    /// A multi-line text area, scrolled, read-only if asked.
    Area {
        label: String,
        value: String,
        read_only: bool,
        lines: i32,
    },
    /// A drop-down list.
    Choice {
        label: String,
        items: Vec<String>,
        selected: usize,
    },
    Check {
        label: String,
        checked: bool,
    },
    /// A checkbox that runs the form's action, with `action` as its id,
    /// whenever it's ticked or unticked.
    Toggle {
        label: String,
        checked: bool,
        action: i32,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Closes the form. Enter presses it.
    Default,
    /// Closes the form. Escape presses it.
    Cancel,
    /// Closes the form.
    Close,
    /// Stays open, and runs the form's action.
    Action,
}

pub struct Button {
    pub text: String,
    pub id: i32,
    pub role: Role,
}

impl Button {
    pub fn new(text: &str, id: i32, role: Role) -> Self {
        Button {
            text: text.to_string(),
            id,
            role,
        }
    }
}

pub type Action = Box<dyn FnMut(i32, &Handle)>;

pub struct Form {
    pub title: String,
    pub fields: Vec<Field>,
    pub buttons: Vec<Button>,
    /// Width at 96 dots per inch.
    pub width: i32,
    /// Runs for buttons with the Action role.
    pub action: Option<Action>,
    /// The field to start on, rather than the first control.
    pub focus: Option<usize>,
}

impl Form {
    pub fn new(title: &str) -> Self {
        Form {
            title: title.to_string(),
            fields: Vec::new(),
            buttons: Vec::new(),
            width: 460,
            action: None,
            focus: None,
        }
    }

    pub fn field(mut self, field: Field) -> Self {
        self.fields.push(field);
        self
    }

    pub fn text(self, text: &str) -> Self {
        self.field(Field::Text(text.to_string()))
    }

    pub fn button(mut self, text: &str, id: i32, role: Role) -> Self {
        self.buttons.push(Button::new(text, id, role));
        self
    }
}

/// A field's value when the form closed.
#[derive(Clone, Debug)]
pub enum Value {
    None,
    Text(String),
    Choice(Option<usize>),
    Check(bool),
}

impl Value {
    pub fn text(&self) -> String {
        match self {
            Value::Text(t) => t.clone(),
            _ => String::new(),
        }
    }
    pub fn checked(&self) -> bool {
        matches!(self, Value::Check(true))
    }
    pub fn choice(&self) -> Option<usize> {
        match self {
            Value::Choice(c) => *c,
            _ => None,
        }
    }
}

pub struct Answer {
    /// The id of the button that closed the form.
    pub button: i32,
    pub values: Vec<Value>,
}

/// The open form's controls, for its action to read and change.
pub struct Handle {
    pub dialog: HWND,
    controls: Vec<Option<HWND>>,
}

impl Handle {
    pub fn set_text(&self, field: usize, text: &str) {
        if let Some(Some(hwnd)) = self.controls.get(field) {
            ui::set_text(*hwnd, text);
        }
    }

    pub fn set_checked(&self, field: usize, on: bool) {
        if let Some(Some(hwnd)) = self.controls.get(field) {
            ui::set_checked(*hwnd, on);
        }
    }

    /// A field's control, to change once the form's action has finished.
    pub fn control(&self, field: usize) -> Option<HWND> {
        self.controls.get(field).copied().flatten()
    }

    pub fn checked(&self, field: usize) -> bool {
        matches!(self.controls.get(field), Some(Some(hwnd)) if ui::checked(*hwnd))
    }

    pub fn choice(&self, field: usize) -> Option<usize> {
        match self.controls.get(field) {
            Some(Some(hwnd)) => ui::combo_selection(*hwnd),
            _ => None,
        }
    }

    /// Replaces a drop-down list's choices.
    pub fn set_choices(&self, field: usize, items: &[String], selected: usize) {
        if let Some(Some(hwnd)) = self.controls.get(field) {
            ui::set_combo_items(*hwnd, items, selected);
        }
    }
}

struct State {
    form: Form,
    controls: Vec<Option<HWND>>,
    /// Control id to button index.
    button_ids: HashMap<u16, usize>,
    /// Control id to the action id of a Toggle.
    toggle_ids: HashMap<u16, i32>,
    closed_by: Option<i32>,
}

thread_local! {
    static FORMS: RefCell<HashMap<isize, Rc<RefCell<State>>>> = RefCell::new(HashMap::new());
    static CREATING: RefCell<Option<Rc<RefCell<State>>>> = const { RefCell::new(None) };
}

const FIRST_BUTTON: u16 = 3000;
const FIRST_FIELD: u16 = 2000;
const MARGIN: i32 = 12;

/// Shows a form and waits for it to close.
pub fn run(owner: HWND, form: Form) -> Answer {
    let template = template(&form.title);
    let state = Rc::new(RefCell::new(State {
        form,
        controls: Vec::new(),
        button_ids: HashMap::new(),
        toggle_ids: HashMap::new(),
        closed_by: None,
    }));
    CREATING.with(|c| *c.borrow_mut() = Some(state.clone()));
    let dialog = unsafe {
        CreateDialogIndirectParamW(
            Some(ui::hinstance()),
            template.as_ptr() as *const DLGTEMPLATE,
            Some(owner),
            Some(dialog_proc),
            LPARAM(0),
        )
    };
    CREATING.with(|c| *c.borrow_mut() = None);
    let Ok(dialog) = dialog else {
        return Answer {
            button: cancel_id(&state.borrow().form),
            values: Vec::new(),
        };
    };
    unsafe {
        let _ = EnableWindow(owner, false);
        let _ = ShowWindow(dialog, SW_SHOW);
    }
    let mut quit = None;
    let mut msg = MSG::default();
    while state.borrow().closed_by.is_none() {
        let got = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        if got.0 <= 0 {
            quit = Some(msg.wParam.0 as i32);
            break;
        }
        unsafe {
            if !IsDialogMessageW(dialog, &msg).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
    let answer = {
        let state = state.borrow();
        Answer {
            button: state.closed_by.unwrap_or_else(|| cancel_id(&state.form)),
            values: state
                .form
                .fields
                .iter()
                .zip(&state.controls)
                .map(|(field, hwnd)| match (field, hwnd) {
                    (Field::Edit { .. } | Field::Area { .. }, Some(h)) => Value::Text(ui::text(*h)),
                    (Field::Choice { .. }, Some(h)) => Value::Choice(ui::combo_selection(*h)),
                    (Field::Check { .. } | Field::Toggle { .. }, Some(h)) => {
                        Value::Check(ui::checked(*h))
                    }
                    _ => Value::None,
                })
                .collect(),
        }
    };
    unsafe {
        // The owner comes back before the form goes, so it gets the focus back.
        let _ = EnableWindow(owner, true);
        let _ = DestroyWindow(dialog);
        let _ = SetForegroundWindow(owner);
    }
    FORMS.with(|f| f.borrow_mut().remove(&(dialog.0 as isize)));
    if let Some(code) = quit {
        unsafe { PostQuitMessage(code) };
    }
    answer
}

fn cancel_id(form: &Form) -> i32 {
    form.buttons
        .iter()
        .find(|b| b.role == Role::Cancel)
        .map(|b| b.id)
        .unwrap_or(-1)
}

/// An empty dialog template: the controls are added when it opens.
fn template(title: &str) -> Vec<u32> {
    const DS_MODALFRAME: u32 = 0x80;
    let style = (WS_POPUP | WS_CAPTION | WS_SYSMENU).0 | DS_MODALFRAME;
    let mut words: Vec<u16> = Vec::new();
    words.extend([style as u16, (style >> 16) as u16]);
    let ex = WS_EX_CONTROLPARENT.0;
    words.extend([ex as u16, (ex >> 16) as u16]);
    words.push(0); // no controls in the template
    words.extend([0, 0, 200, 100]); // position and size, set again later
    words.push(0); // no menu
    words.push(0); // the standard dialog class
    words.extend(title.encode_utf16());
    words.push(0);
    if words.len() % 2 == 1 {
        words.push(0);
    }
    words
        .chunks(2)
        .map(|pair| pair[0] as u32 | (pair[1] as u32) << 16)
        .collect()
}

unsafe extern "system" fn dialog_proc(
    dialog: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> isize {
    std::panic::catch_unwind(|| handle(dialog, msg, wparam, lparam)).unwrap_or_else(|panic| {
        crate::log_panic(&panic);
        0
    })
}

fn handle(dialog: HWND, msg: u32, wparam: WPARAM, _: LPARAM) -> isize {
    match msg {
        WM_INITDIALOG => {
            let Some(state) = CREATING.with(|c| c.borrow().clone()) else {
                return 1;
            };
            FORMS.with(|f| f.borrow_mut().insert(dialog.0 as isize, state.clone()));
            let focus = build(dialog, &mut state.borrow_mut());
            match focus {
                Some(control) => {
                    ui::focus(control);
                    0
                }
                None => 1,
            }
        }
        WM_COMMAND => {
            let id = (wparam.0 & 0xFFFF) as u16;
            let notification = ((wparam.0 >> 16) & 0xFFFF) as u32;
            if notification != BN_CLICKED && id != IDCANCEL.0 as u16 {
                return 0;
            }
            let Some(state) = FORMS.with(|f| f.borrow().get(&(dialog.0 as isize)).cloned()) else {
                return 0;
            };
            let run_action = |action_id: i32| {
                // Taken out while it runs, as it may open a file dialog,
                // which runs its own messages.
                let action = state.borrow_mut().form.action.take();
                if let Some(mut action) = action {
                    let handle = Handle {
                        dialog,
                        controls: state.borrow().controls.clone(),
                    };
                    action(action_id, &handle);
                    state.borrow_mut().form.action = Some(action);
                }
            };
            let toggle = state.borrow().toggle_ids.get(&id).copied();
            if let Some(action_id) = toggle {
                run_action(action_id);
                return 1;
            }
            let index = if id == IDCANCEL.0 as u16 {
                // Escape: the cancel button, or a form's only button, or
                // its default, such as Done.
                let s = state.borrow();
                let buttons = &s.form.buttons;
                buttons
                    .iter()
                    .position(|b| b.role == Role::Cancel)
                    .or((buttons.len() == 1).then_some(0))
                    .or_else(|| buttons.iter().position(|b| b.role == Role::Default))
            } else {
                state.borrow().button_ids.get(&id).copied()
            };
            let Some(index) = index else { return 0 };
            let (button_id, role) = {
                let s = state.borrow();
                (s.form.buttons[index].id, s.form.buttons[index].role)
            };
            if role == Role::Action {
                run_action(button_id);
            } else {
                state.borrow_mut().closed_by = Some(button_id);
            }
            1
        }
        WM_CLOSE => {
            if let Some(state) = FORMS.with(|f| f.borrow().get(&(dialog.0 as isize)).cloned()) {
                let cancel = cancel_id(&state.borrow().form);
                state.borrow_mut().closed_by = Some(cancel);
            }
            1
        }
        _ => 0,
    }
}

/// Creates the controls and sizes the dialog. Returns the control to focus.
fn build(dialog: HWND, state: &mut State) -> Option<HWND> {
    let width = state.form.width;
    let inner = width - 2 * MARGIN;
    let mut y = MARGIN;
    let mut first_input = None;
    let mut controls = Vec::new();
    for (i, field) in state.form.fields.iter().enumerate() {
        let id = FIRST_FIELD + i as u16;
        let control = match field {
            Field::Text(text) => {
                let height = ui::text_height(dialog, text, inner).max(16);
                let label = ui::label(dialog, text);
                ui::place(label, MARGIN, y, inner, height);
                y += height + 10;
                // Kept, so an action can change it, but never focused.
                controls.push(Some(label));
                continue;
            }
            Field::Edit { label, value } => {
                let l = ui::label(dialog, label);
                ui::place(l, MARGIN, y, inner, 16);
                y += 19;
                let edit = ui::edit(dialog, value, id);
                ui::place(edit, MARGIN, y, inner, 23);
                y += 33;
                Some(edit)
            }
            Field::Area {
                label,
                value,
                read_only,
                lines,
            } => {
                let l = ui::label(dialog, label);
                ui::place(l, MARGIN, y, inner, 16);
                y += 19;
                let area = ui::text_area(dialog, value, id, *read_only);
                let height = lines * 16 + 8;
                ui::place(area, MARGIN, y, inner, height);
                y += height + 10;
                Some(area)
            }
            Field::Choice {
                label,
                items,
                selected,
            } => {
                let l = ui::label(dialog, label);
                ui::place(l, MARGIN, y, inner, 16);
                y += 19;
                let combo = ui::combo_box(dialog, id);
                ui::set_combo_items(combo, items, *selected);
                // A drop-down list's height includes its open list.
                ui::place(combo, MARGIN, y, inner, 240);
                y += 33;
                Some(combo)
            }
            Field::Check { label, checked } => {
                let check = ui::checkbox(dialog, label, id, *checked);
                ui::place(check, MARGIN, y, inner, 20);
                y += 26;
                Some(check)
            }
            Field::Toggle {
                label,
                checked,
                action,
            } => {
                let check = ui::checkbox(dialog, label, id, *checked);
                ui::place(check, MARGIN, y, inner, 20);
                y += 26;
                state.toggle_ids.insert(id, *action);
                Some(check)
            }
        };
        if first_input.is_none() {
            first_input = control;
        }
        controls.push(control);
    }

    // Buttons along the bottom, right-aligned, in reading order.
    y += 4;
    let widths: Vec<i32> = state
        .form
        .buttons
        .iter()
        .map(|b| (b.text.chars().count() as i32 * 7 + 24).max(84))
        .collect();
    let total: i32 = widths.iter().sum::<i32>() + 8 * (widths.len() as i32 - 1).max(0);
    let mut x = width - MARGIN - total;
    let mut default_button = None;
    for (i, button) in state.form.buttons.iter().enumerate() {
        let id = if button.role == Role::Cancel {
            IDCANCEL.0 as u16
        } else {
            FIRST_BUTTON + i as u16
        };
        let hwnd = ui::button(dialog, &button.text, id);
        if button.role == Role::Default {
            unsafe {
                SetWindowLongW(
                    hwnd,
                    GWL_STYLE,
                    GetWindowLongW(hwnd, GWL_STYLE) | BS_DEFPUSHBUTTON,
                );
            }
            ui::send(dialog, DM_SETDEFID, id as usize, 0);
            default_button = Some(hwnd);
        }
        ui::place(hwnd, x, y, widths[i], 26);
        x += widths[i] + 8;
        state.button_ids.insert(id, i);
    }
    y += 26 + MARGIN;

    // Size the dialog to fit, centred on its owner.
    unsafe {
        let dpi = GetDpiForWindow(dialog).max(96);
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: ui::scaled(dialog, width),
            bottom: ui::scaled(dialog, y),
        };
        let style = WINDOW_STYLE(GetWindowLongW(dialog, GWL_STYLE) as u32);
        let ex = WINDOW_EX_STYLE(GetWindowLongW(dialog, GWL_EXSTYLE) as u32);
        let _ = AdjustWindowRectExForDpi(&mut rect, style, false, ex, dpi);
        let (w, h) = (rect.right - rect.left, rect.bottom - rect.top);
        let mut owner_rect = RECT::default();
        let owner = GetWindow(dialog, GW_OWNER).unwrap_or_default();
        let _ = GetWindowRect(owner, &mut owner_rect);
        let x = owner_rect.left + ((owner_rect.right - owner_rect.left) - w) / 2;
        let y = owner_rect.top + ((owner_rect.bottom - owner_rect.top) - h) / 2;
        let _ = SetWindowPos(
            dialog,
            None,
            x.max(0),
            y.max(0),
            w,
            h,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
    state.controls = controls;
    match state.form.focus {
        Some(field) => state.controls.get(field).copied().flatten(),
        None => first_input.or(default_button),
    }
}

/// Asks a yes-or-no question. The safe answer is the default, so a stray
/// Enter does nothing harmful. Returns true for `action`.
pub fn confirm(owner: HWND, title: &str, detail: &str, action: &str) -> bool {
    let answer = run(
        owner,
        Form::new(title)
            .text(detail)
            .button("Cancel", 0, Role::Cancel)
            .button(action, 1, Role::Close),
    );
    answer.button == 1
}

/// Shows a message with an OK button.
pub fn message(owner: HWND, title: &str, detail: &str) {
    run(
        owner,
        Form::new(title).text(detail).button("OK", 0, Role::Default),
    );
}

/// Asks for a name. Returns it trimmed, or none if cancelled or empty.
pub fn ask_name(
    owner: HWND,
    title: &str,
    label: &str,
    initial: &str,
    action: &str,
) -> Option<String> {
    let answer = run(
        owner,
        Form::new(title)
            .field(Field::Edit {
                label: label.to_string(),
                value: initial.to_string(),
            })
            .button(action, 1, Role::Default)
            .button("Cancel", 0, Role::Cancel),
    );
    let name = answer.values.first()?.text().trim().to_string();
    (answer.button == 1 && !name.is_empty()).then_some(name)
}
