//! A thin layer over Windows' standard controls: creating them, reading and
//! setting them, and running work on the window thread. Standard controls are
//! what Windows screen readers know best, so AAE uses nothing else.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicIsize, Ordering};

use windows::Win32::Foundation::{HGLOBAL, HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateFontIndirectW, DT_CALCRECT, DT_WORDBREAK, DrawTextW, GetDC, HFONT, ReleaseDC,
    SelectObject,
};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::{CF_HDROP, CF_UNICODETEXT};
use windows::Win32::UI::Controls::Dialogs::{
    GetOpenFileNameW, GetSaveFileNameW, OFN_ALLOWMULTISELECT, OFN_EXPLORER, OFN_FILEMUSTEXIST,
    OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST, OPENFILENAMEW,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, IsWindowEnabled, SetFocus};
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

/// A null-terminated UTF-16 copy of a string, for Windows.
pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

pub fn hinstance() -> windows::Win32::Foundation::HINSTANCE {
    unsafe { GetModuleHandleW(None).unwrap_or_default().into() }
}

/// The message to run queued work on the window thread.
pub const WM_RUN: u32 = WM_APP + 1;

static MAIN_WINDOW: AtomicIsize = AtomicIsize::new(0);

type Work = Box<dyn FnOnce() + Send>;
static QUEUE: Mutex<VecDeque<Work>> = Mutex::new(VecDeque::new());

/// Sets the window that queued work is run by.
pub fn set_main_window(hwnd: HWND) {
    MAIN_WINDOW.store(hwnd.0 as isize, Ordering::SeqCst);
}

pub fn main_window() -> HWND {
    HWND(MAIN_WINDOW.load(Ordering::SeqCst) as *mut _)
}

/// Runs work on the window thread, from any thread. This is how background
/// work reports back.
pub fn run_on_ui(work: impl FnOnce() + Send + 'static) {
    QUEUE.lock().unwrap().push_back(Box::new(work));
    unsafe {
        let _ = PostMessageW(Some(main_window()), WM_RUN, WPARAM(0), LPARAM(0));
    }
}

/// Runs the work queued for the window thread. Called on `WM_RUN`.
pub fn run_queued() {
    loop {
        let next = QUEUE.lock().unwrap().pop_front();
        match next {
            Some(work) => work(),
            None => break,
        }
    }
}

thread_local! {
    static FONT: RefCell<Option<(u32, HFONT)>> = const { RefCell::new(None) };
}

/// Windows' message font at a window's scale, as dialogs use.
pub fn font(hwnd: HWND) -> HFONT {
    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
    FONT.with(|cell| {
        if let Some((for_dpi, font)) = *cell.borrow()
            && for_dpi == dpi
        {
            return font;
        }
        let mut metrics = NONCLIENTMETRICSW {
            cbSize: std::mem::size_of::<NONCLIENTMETRICSW>() as u32,
            ..Default::default()
        };
        let font = unsafe {
            let _ = windows::Win32::UI::HiDpi::SystemParametersInfoForDpi(
                SPI_GETNONCLIENTMETRICS.0,
                metrics.cbSize,
                Some(&mut metrics as *mut _ as *mut _),
                0,
                dpi,
            );
            CreateFontIndirectW(&metrics.lfMessageFont)
        };
        *cell.borrow_mut() = Some((dpi, font));
        font
    })
}

/// A length in pixels at 96 dots per inch, at a window's scale.
pub fn scaled(hwnd: HWND, length: i32) -> i32 {
    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96) as i32;
    length * dpi / 96
}

/// Creates a child control.
pub fn control(
    parent: HWND,
    class: PCWSTR,
    text: &str,
    style: WINDOW_STYLE,
    ex_style: WINDOW_EX_STYLE,
    id: u16,
) -> HWND {
    let text = wide(text);
    let hwnd = unsafe {
        CreateWindowExW(
            ex_style,
            class,
            PCWSTR(text.as_ptr()),
            WS_CHILD | WS_VISIBLE | style,
            0,
            0,
            10,
            10,
            Some(parent),
            Some(HMENU(id as usize as *mut _)),
            Some(hinstance()),
            None,
        )
        .unwrap_or_default()
    };
    unsafe {
        SendMessageW(
            hwnd,
            WM_SETFONT,
            Some(WPARAM(font(parent).0 as usize)),
            Some(LPARAM(1)),
        );
    }
    hwnd
}

/// A label. Windows screen readers name the control after it from it, so
/// it's created just before the control it labels.
pub fn label(parent: HWND, text: &str) -> HWND {
    control(
        parent,
        w!("STATIC"),
        text,
        WINDOW_STYLE(0),
        WINDOW_EX_STYLE(0),
        0xFFFF,
    )
}

pub fn button(parent: HWND, text: &str, id: u16) -> HWND {
    control(
        parent,
        w!("BUTTON"),
        text,
        WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON as u32),
        WINDOW_EX_STYLE(0),
        id,
    )
}

pub fn checkbox(parent: HWND, text: &str, id: u16, checked: bool) -> HWND {
    let hwnd = control(
        parent,
        w!("BUTTON"),
        text,
        WS_TABSTOP | WINDOW_STYLE(BS_AUTOCHECKBOX as u32),
        WINDOW_EX_STYLE(0),
        id,
    );
    set_checked(hwnd, checked);
    hwnd
}

/// A single-line text field.
pub fn edit(parent: HWND, text: &str, id: u16) -> HWND {
    control(
        parent,
        w!("EDIT"),
        text,
        WS_TABSTOP | WS_BORDER | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
        WINDOW_EX_STYLE(0),
        id,
    )
}

/// A multi-line text area, read-only for reading long text line by line.
pub fn text_area(parent: HWND, text: &str, id: u16, read_only: bool) -> HWND {
    let mut style = WS_TABSTOP
        | WS_BORDER
        | WS_VSCROLL
        | WINDOW_STYLE((ES_MULTILINE | ES_AUTOVSCROLL | ES_WANTRETURN) as u32);
    if read_only {
        style |= WINDOW_STYLE(ES_READONLY as u32);
    }
    let hwnd = control(parent, w!("EDIT"), "", style, WINDOW_EX_STYLE(0), id);
    set_text(hwnd, text);
    hwnd
}

pub fn list_box(parent: HWND, id: u16) -> HWND {
    control(
        parent,
        w!("LISTBOX"),
        "",
        WS_TABSTOP
            | WS_BORDER
            | WS_VSCROLL
            | WINDOW_STYLE((LBS_NOTIFY | LBS_NOINTEGRALHEIGHT) as u32),
        WINDOW_EX_STYLE(0),
        id,
    )
}

pub fn combo_box(parent: HWND, id: u16) -> HWND {
    control(
        parent,
        w!("COMBOBOX"),
        "",
        WS_TABSTOP | WS_VSCROLL | WINDOW_STYLE(CBS_DROPDOWNLIST as u32),
        WINDOW_EX_STYLE(0),
        id,
    )
}

/// Places a control, in pixels at 96 dots per inch.
pub fn place(hwnd: HWND, x: i32, y: i32, width: i32, height: i32) {
    let parent = unsafe { GetParent(hwnd) }.unwrap_or(hwnd);
    let s = |v| scaled(parent, v);
    unsafe {
        let _ = MoveWindow(hwnd, s(x), s(y), s(width), s(height), true);
    }
}

pub fn set_text(hwnd: HWND, text: &str) {
    // Edit controls need Windows line endings.
    let text = text.replace("\r\n", "\n").replace('\n', "\r\n");
    let text = wide(&text);
    unsafe {
        let _ = SetWindowTextW(hwnd, PCWSTR(text.as_ptr()));
    }
}

pub fn text(hwnd: HWND) -> String {
    unsafe {
        let length = GetWindowTextLengthW(hwnd);
        let mut buffer = vec![0u16; length as usize + 1];
        let read = GetWindowTextW(hwnd, &mut buffer);
        String::from_utf16_lossy(&buffer[..read as usize])
    }
}

pub fn enable(hwnd: HWND, on: bool) {
    unsafe {
        if IsWindowEnabled(hwnd).as_bool() != on {
            let _ = EnableWindow(hwnd, on);
        }
    }
}

pub fn show(hwnd: HWND, on: bool) {
    unsafe {
        let _ = ShowWindow(hwnd, if on { SW_SHOW } else { SW_HIDE });
    }
}

pub fn focus(hwnd: HWND) {
    unsafe {
        let _ = SetFocus(Some(hwnd));
    }
}

pub fn send(hwnd: HWND, message: u32, wparam: usize, lparam: isize) -> isize {
    unsafe { SendMessageW(hwnd, message, Some(WPARAM(wparam)), Some(LPARAM(lparam))).0 }
}

pub fn checked(hwnd: HWND) -> bool {
    send(hwnd, BM_GETCHECK, 0, 0) == 1
}

pub fn set_checked(hwnd: HWND, on: bool) {
    send(hwnd, BM_SETCHECK, on as usize, 0);
}

/// Replaces a list box's items, keeping the selection where it can.
pub fn set_list_items(hwnd: HWND, items: &[String], selected: Option<usize>) {
    send(hwnd, WM_SETREDRAW, 0, 0);
    send(hwnd, LB_RESETCONTENT, 0, 0);
    for item in items {
        let item = wide(item);
        send(hwnd, LB_ADDSTRING, 0, item.as_ptr() as isize);
    }
    if let Some(index) = selected {
        send(hwnd, LB_SETCURSEL, index, 0);
    }
    send(hwnd, WM_SETREDRAW, 1, 0);
}

/// Changes one list box item's text, keeping the selection.
pub fn set_list_item(hwnd: HWND, index: usize, item: &str) {
    let selected = list_selection(hwnd);
    send(hwnd, LB_DELETESTRING, index, 0);
    let item = wide(item);
    send(hwnd, LB_INSERTSTRING, index, item.as_ptr() as isize);
    if let Some(selected) = selected {
        send(hwnd, LB_SETCURSEL, selected, 0);
    }
}

/// Adds an item to the end of a list box, keeping the selection.
pub fn add_list_item(hwnd: HWND, item: &str) {
    let item = wide(item);
    send(hwnd, LB_ADDSTRING, 0, item.as_ptr() as isize);
}

/// Replaces a text area's text, showing its end, as a transcript does.
/// Windows' text controls want their own line endings.
pub fn set_transcript(hwnd: HWND, text: &str) {
    let text = text.replace("\r\n", "\n").replace('\n', "\r\n");
    set_text(hwnd, &text);
    let end = text.encode_utf16().count();
    send(
        hwnd,
        windows::Win32::UI::Controls::EM_SETSEL,
        end,
        end as isize,
    );
    send(hwnd, windows::Win32::UI::Controls::EM_SCROLLCARET, 0, 0);
}

pub fn list_selection(hwnd: HWND) -> Option<usize> {
    let index = send(hwnd, LB_GETCURSEL, 0, 0);
    (index >= 0).then_some(index as usize)
}

pub fn set_combo_items(hwnd: HWND, items: &[String], selected: usize) {
    send(hwnd, CB_RESETCONTENT, 0, 0);
    for item in items {
        let item = wide(item);
        send(hwnd, CB_ADDSTRING, 0, item.as_ptr() as isize);
    }
    send(hwnd, CB_SETCURSEL, selected, 0);
}

pub fn combo_selection(hwnd: HWND) -> Option<usize> {
    let index = send(hwnd, CB_GETCURSEL, 0, 0);
    (index >= 0).then_some(index as usize)
}

/// The height text needs at a width, both at 96 dots per inch.
pub fn text_height(hwnd: HWND, text: &str, width: i32) -> i32 {
    let mut text: Vec<u16> = text.encode_utf16().collect();
    unsafe {
        let dc = GetDC(Some(hwnd));
        let old = SelectObject(dc, font(hwnd).into());
        let mut rect = windows::Win32::Foundation::RECT {
            left: 0,
            top: 0,
            right: scaled(hwnd, width),
            bottom: 0,
        };
        DrawTextW(dc, &mut text, &mut rect, DT_CALCRECT | DT_WORDBREAK);
        SelectObject(dc, old);
        ReleaseDC(Some(hwnd), dc);
        let dpi = GetDpiForWindow(hwnd).max(96) as i32;
        rect.bottom * 96 / dpi
    }
}

/// Asks for files to open. Returns their paths, or none if cancelled.
pub fn open_files(owner: HWND, title: &str, filter: &[(&str, &str)], many: bool) -> Vec<String> {
    let mut buffer = vec![0u16; 32 * 1024];
    let filter = filter_string(filter);
    let title = wide(title);
    let mut flags = OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST;
    if many {
        flags |= OFN_ALLOWMULTISELECT;
    }
    let mut ofn = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: owner,
        lpstrFilter: PCWSTR(filter.as_ptr()),
        lpstrFile: windows::core::PWSTR(buffer.as_mut_ptr()),
        nMaxFile: buffer.len() as u32,
        lpstrTitle: PCWSTR(title.as_ptr()),
        Flags: flags,
        ..Default::default()
    };
    if !unsafe { GetOpenFileNameW(&mut ofn) }.as_bool() {
        return Vec::new();
    }
    // One file is a full path; several are the folder, then each name, all
    // separated by nulls.
    let parts: Vec<String> = buffer
        .split(|&c| c == 0)
        .take_while(|part| !part.is_empty())
        .map(String::from_utf16_lossy)
        .collect();
    match parts.as_slice() {
        [] => Vec::new(),
        [one] => vec![one.clone()],
        [folder, names @ ..] => names
            .iter()
            .map(|name| format!("{}\\{name}", folder.trim_end_matches('\\')))
            .collect(),
    }
}

/// Asks where to save a file. Returns its path, or none if cancelled.
pub fn save_file(
    owner: HWND,
    title: &str,
    name: &str,
    filter: &[(&str, &str)],
    extension: &str,
) -> Option<String> {
    let mut buffer = vec![0u16; 4096];
    let name: Vec<u16> = name
        .chars()
        .map(|c| if r#"\/:*?"<>|"#.contains(c) { '-' } else { c })
        .collect::<String>()
        .encode_utf16()
        .collect();
    buffer[..name.len()].copy_from_slice(&name);
    let filter = filter_string(filter);
    let title = wide(title);
    let extension = wide(extension);
    let mut ofn = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: owner,
        lpstrFilter: PCWSTR(filter.as_ptr()),
        lpstrFile: windows::core::PWSTR(buffer.as_mut_ptr()),
        nMaxFile: buffer.len() as u32,
        lpstrTitle: PCWSTR(title.as_ptr()),
        lpstrDefExt: PCWSTR(extension.as_ptr()),
        Flags: OFN_EXPLORER | OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST,
        ..Default::default()
    };
    if !unsafe { GetSaveFileNameW(&mut ofn) }.as_bool() {
        return None;
    }
    let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    Some(String::from_utf16_lossy(&buffer[..end]))
}

fn filter_string(filter: &[(&str, &str)]) -> Vec<u16> {
    let mut out = String::new();
    for (name, pattern) in filter {
        out.push_str(name);
        out.push('\0');
        out.push_str(pattern);
        out.push('\0');
    }
    out.push('\0');
    out.encode_utf16().collect()
}

/// The clipboard's text, if it has any.
pub fn clipboard_text(owner: HWND) -> Option<String> {
    unsafe {
        OpenClipboard(Some(owner)).ok()?;
        let text = GetClipboardData(CF_UNICODETEXT.0 as u32)
            .ok()
            .and_then(|handle| {
                let global = HGLOBAL(handle.0);
                let pointer = GlobalLock(global) as *const u16;
                if pointer.is_null() {
                    return None;
                }
                let mut length = 0;
                while *pointer.add(length) != 0 {
                    length += 1;
                }
                let text = String::from_utf16_lossy(std::slice::from_raw_parts(pointer, length));
                let _ = GlobalUnlock(global);
                Some(text)
            });
        let _ = CloseClipboard();
        text
    }
}

/// Puts text on the clipboard.
pub fn set_clipboard_text(owner: HWND, text: &str) -> bool {
    let text = text.replace("\r\n", "\n").replace('\n', "\r\n");
    let text = wide(&text);
    unsafe {
        if OpenClipboard(Some(owner)).is_err() {
            return false;
        }
        let _ = EmptyClipboard();
        let done = (|| {
            let global = GlobalAlloc(GMEM_MOVEABLE, text.len() * 2).ok()?;
            let pointer = GlobalLock(global) as *mut u16;
            if pointer.is_null() {
                return None;
            }
            std::ptr::copy_nonoverlapping(text.as_ptr(), pointer, text.len());
            let _ = GlobalUnlock(global);
            SetClipboardData(
                CF_UNICODETEXT.0 as u32,
                Some(windows::Win32::Foundation::HANDLE(global.0)),
            )
            .ok()
        })()
        .is_some();
        let _ = CloseClipboard();
        done
    }
}

/// Files copied in File Explorer, on the clipboard.
pub fn clipboard_files(owner: HWND) -> Vec<String> {
    unsafe {
        if OpenClipboard(Some(owner)).is_err() {
            return Vec::new();
        }
        let files = GetClipboardData(CF_HDROP.0 as u32)
            .map(|handle| dropped_files(HDROP(handle.0)))
            .unwrap_or_default();
        let _ = CloseClipboard();
        files
    }
}

/// The paths of files dropped on a window, or copied.
pub fn dropped_files(drop: HDROP) -> Vec<String> {
    unsafe {
        let count = DragQueryFileW(drop, u32::MAX, None);
        (0..count)
            .map(|i| {
                let length = DragQueryFileW(drop, i, None) as usize;
                let mut buffer = vec![0u16; length + 1];
                DragQueryFileW(drop, i, Some(&mut buffer));
                String::from_utf16_lossy(&buffer[..length])
            })
            .collect()
    }
}
