//! Windows' side of the touchpad as the device's screen (see `touchpad`):
//! Raw Input for a Precision Touchpad's reports, and a mouse hook that
//! keeps Windows from acting on the fingers itself.
//!
//! - Touches go to Android only while AAE's window is in front, as keys do.
//!   Fingers down when another window comes in front are lifted.
//! - While fingers are on the touchpad, and for a moment after the last
//!   lifts, as Windows' tap to click and scrolling's glide come after, the
//!   pointer doesn't move, click or scroll. A mouse works as usual otherwise.
//! - Windows' own three- and four-finger gestures, such as Task View, belong
//!   to the shell and can't be held back. To use the screen reader's, set
//!   them to Nothing under Settings, Bluetooth & devices, Touchpad.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

use windows::Win32::Devices::HumanInterfaceDevice::*;
use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Input::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::w;

use crate::touchpad::{self, Assembler, CONFIDENCE, CONTACT_ID, Fingers, Out, Pad, Recogniser, Values};
use crate::ui;

/// A Precision Touchpad's top-level collection: digitizer, touch pad.
const DIGITIZER: u16 = 0x0D;
const TOUCH_PAD: u16 = 0x05;

/// How long the pointer stays still after the last finger lifts.
const QUIET_MS: u64 = 1500;

/// The timer that decides about a touch held back, and how often it runs.
const DECIDE_TIMER: usize = 1;
const DECIDE_EVERY_MS: u32 = 20;

struct Device {
    /// Its preparsed data, for the HID parser, in a buffer aligned for it.
    prepared: Vec<u64>,
    /// The most values one report can hold.
    data_length: u32,
    /// Data indices to (link collection, usage page, usage).
    usages: HashMap<u16, (u16, u16, u16)>,
    /// The link collections holding a finger each.
    links: Vec<u16>,
    pad: Pad,
    assembler: Assembler,
}

impl Device {
    fn prepared(&self) -> PHIDP_PREPARSED_DATA {
        PHIDP_PREPARSED_DATA(self.prepared.as_ptr() as isize)
    }
}

struct State {
    window: HWND,
    mouse_hook: HHOOK,
    /// Touchpads, by device handle.
    devices: HashMap<isize, Device>,
    fingers: Fingers,
    recogniser: Recogniser,
    on_touches: fn(Vec<Out>),
}

impl State {
    /// Runs the timer while a touch is held back.
    fn time(&self) {
        unsafe {
            if self.recogniser.waiting() {
                SetTimer(Some(self.window), DECIDE_TIMER, DECIDE_EVERY_MS, None);
            } else {
                let _ = KillTimer(Some(self.window), DECIDE_TIMER);
            }
        }
    }

    /// Lifts every finger, telling Android if it had them.
    fn lift_all(&mut self) -> Vec<Out> {
        let lifts = self.fingers.lift_all();
        let out = self.recogniser.cancel(lifts);
        self.time();
        out
    }
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// For the mouse hook, which mustn't wait on anything.
static ACTIVE: AtomicBool = AtomicBool::new(false);
static TOUCHING: AtomicBool = AtomicBool::new(false);
static LIFTED_AT: AtomicU64 = AtomicU64::new(0);

fn now_ms() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

/// Makes the touchpad the device's screen, sending touches to `on_touches`
/// on the window thread, with fingers taken as moving once they've gone
/// `settle_mm`. Returns how many touchpads there are; with none, nothing
/// changes.
pub fn start(on_touches: fn(Vec<Out>), settle_mm: u8) -> usize {
    let running = STATE.with(|cell| cell.borrow().as_ref().map(|s| s.devices.len()));
    if let Some(count) = running {
        return count;
    }
    let devices = touchpads();
    if devices.is_empty() {
        return 0;
    }
    let count = devices.len();
    unsafe {
        static CLASS: OnceLock<()> = OnceLock::new();
        CLASS.get_or_init(|| {
            let class = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(window_proc),
                hInstance: ui::hinstance(),
                lpszClassName: w!("AAETouchpad"),
                ..Default::default()
            };
            RegisterClassExW(&class);
        });
        let Ok(window) = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("AAETouchpad"),
            w!("AAE touchpad"),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(ui::hinstance()),
            None,
        ) else {
            return 0;
        };
        let device = RAWINPUTDEVICE {
            usUsagePage: DIGITIZER,
            usUsage: TOUCH_PAD,
            // Reports come whichever window is in front; touches go only
            // while AAE's is.
            dwFlags: RIDEV_INPUTSINK | RIDEV_DEVNOTIFY,
            hwndTarget: window,
        };
        if RegisterRawInputDevices(&[device], std::mem::size_of::<RAWINPUTDEVICE>() as u32).is_err()
        {
            let _ = DestroyWindow(window);
            return 0;
        }
        let mouse_hook = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), Some(ui::hinstance()), 0)
            .unwrap_or_default();
        STATE.with(|cell| {
            *cell.borrow_mut() = Some(State {
                window,
                mouse_hook,
                devices,
                fingers: Fingers::new(settle_mm),
                recogniser: Recogniser::default(),
                on_touches,
            })
        });
    }
    TOUCHING.store(false, Ordering::SeqCst);
    LIFTED_AT.store(0, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    count
}

/// Gives the touchpad back to Windows. Returns the touches that lift any
/// fingers still down on the device.
pub fn stop() -> Vec<Out> {
    ACTIVE.store(false, Ordering::SeqCst);
    TOUCHING.store(false, Ordering::SeqCst);
    let Some(mut state) = STATE.with(|cell| cell.borrow_mut().take()) else {
        return Vec::new();
    };
    unsafe {
        let device = RAWINPUTDEVICE {
            usUsagePage: DIGITIZER,
            usUsage: TOUCH_PAD,
            dwFlags: RIDEV_REMOVE,
            hwndTarget: HWND::default(),
        };
        let _ = RegisterRawInputDevices(&[device], std::mem::size_of::<RAWINPUTDEVICE>() as u32);
        if !state.mouse_hook.is_invalid() {
            let _ = UnhookWindowsHookEx(state.mouse_hook);
        }
        let lifted = state.lift_all();
        let _ = DestroyWindow(state.window);
        lifted
    }
}

/// Every Precision Touchpad, by device handle.
fn touchpads() -> HashMap<isize, Device> {
    let size = std::mem::size_of::<RAWINPUTDEVICELIST>() as u32;
    let mut count = 0u32;
    unsafe { GetRawInputDeviceList(None, &mut count, size) };
    if count == 0 {
        return HashMap::new();
    }
    let mut list = vec![RAWINPUTDEVICELIST::default(); count as usize];
    let got = unsafe { GetRawInputDeviceList(Some(list.as_mut_ptr()), &mut count, size) };
    if got == u32::MAX {
        return HashMap::new();
    }
    list.truncate(got as usize);
    list.iter()
        .filter(|d| d.dwType == RIM_TYPEHID)
        .filter_map(|d| Some((d.hDevice.0 as isize, device(d.hDevice)?)))
        .collect()
}

/// Reads what a HID device reports, if it's a Precision Touchpad.
fn device(handle: HANDLE) -> Option<Device> {
    unsafe {
        let mut info = RID_DEVICE_INFO {
            cbSize: std::mem::size_of::<RID_DEVICE_INFO>() as u32,
            ..Default::default()
        };
        let mut size = info.cbSize;
        let read = GetRawInputDeviceInfoW(
            Some(handle),
            RIDI_DEVICEINFO,
            Some((&mut info as *mut RID_DEVICE_INFO).cast()),
            &mut size,
        );
        if read == u32::MAX {
            return None;
        }
        let hid = info.Anonymous.hid;
        if hid.usUsagePage != DIGITIZER || hid.usUsage != TOUCH_PAD {
            return None;
        }
        let mut size = 0u32;
        GetRawInputDeviceInfoW(Some(handle), RIDI_PREPARSEDDATA, None, &mut size);
        if size == 0 {
            return None;
        }
        let mut prepared = vec![0u64; (size as usize).div_ceil(8)];
        let read = GetRawInputDeviceInfoW(
            Some(handle),
            RIDI_PREPARSEDDATA,
            Some(prepared.as_mut_ptr().cast()),
            &mut size,
        );
        if read == u32::MAX {
            return None;
        }
        let data = PHIDP_PREPARSED_DATA(prepared.as_ptr() as isize);
        let mut caps: HIDP_CAPS = std::mem::zeroed();
        if HidP_GetCaps(data, &mut caps).0 < 0 {
            return None;
        }
        let mut buttons = vec![HIDP_BUTTON_CAPS::default(); caps.NumberInputButtonCaps as usize];
        let mut length = caps.NumberInputButtonCaps;
        if length > 0
            && HidP_GetButtonCaps(HidP_Input, buttons.as_mut_ptr(), &mut length, data).0 < 0
        {
            return None;
        }
        buttons.truncate(length as usize);
        let mut values = vec![HIDP_VALUE_CAPS::default(); caps.NumberInputValueCaps as usize];
        let mut length = caps.NumberInputValueCaps;
        if length > 0 && HidP_GetValueCaps(HidP_Input, values.as_mut_ptr(), &mut length, data).0 < 0
        {
            return None;
        }
        values.truncate(length as usize);

        // Which usage each data index is.
        let mut usages = HashMap::new();
        let mut add = |link: u16, page: u16, range: (u16, u16), first_index: u16| {
            for (i, usage) in (range.0..=range.1).enumerate() {
                usages.insert(first_index + i as u16, (link, page, usage));
            }
        };
        for cap in &buttons {
            if cap.IsRange {
                let r = cap.Anonymous.Range;
                add(
                    cap.LinkCollection,
                    cap.UsagePage,
                    (r.UsageMin, r.UsageMax),
                    r.DataIndexMin,
                );
            } else {
                let n = cap.Anonymous.NotRange;
                add(
                    cap.LinkCollection,
                    cap.UsagePage,
                    (n.Usage, n.Usage),
                    n.DataIndex,
                );
            }
        }
        for cap in &values {
            if cap.IsRange {
                let r = cap.Anonymous.Range;
                add(
                    cap.LinkCollection,
                    cap.UsagePage,
                    (r.UsageMin, r.UsageMax),
                    r.DataIndexMin,
                );
            } else {
                let n = cap.Anonymous.NotRange;
                add(
                    cap.LinkCollection,
                    cap.UsagePage,
                    (n.Usage, n.Usage),
                    n.DataIndex,
                );
            }
        }

        let is = |page: u16, is_range: bool, min: u16, max: u16, usage: (u16, u16)| {
            page == usage.0
                && if is_range {
                    (min..=max).contains(&usage.1)
                } else {
                    min == usage.1
                }
        };
        let value_is = |cap: &HIDP_VALUE_CAPS, usage: (u16, u16)| {
            let (min, max) = if cap.IsRange {
                (cap.Anonymous.Range.UsageMin, cap.Anonymous.Range.UsageMax)
            } else {
                (cap.Anonymous.NotRange.Usage, cap.Anonymous.NotRange.Usage)
            };
            is(cap.UsagePage, cap.IsRange, min, max, usage)
        };
        let button_is = |cap: &HIDP_BUTTON_CAPS, usage: (u16, u16)| {
            let (min, max) = if cap.IsRange {
                (cap.Anonymous.Range.UsageMin, cap.Anonymous.Range.UsageMax)
            } else {
                (cap.Anonymous.NotRange.Usage, cap.Anonymous.NotRange.Usage)
            };
            is(cap.UsagePage, cap.IsRange, min, max, usage)
        };
        let x = values.iter().find(|c| value_is(c, touchpad::X))?;
        let y = values.iter().find(|c| value_is(c, touchpad::Y))?;
        let mut links: Vec<u16> = values
            .iter()
            .filter(|c| value_is(c, CONTACT_ID))
            .map(|c| c.LinkCollection)
            .collect();
        links.sort_unstable();
        links.dedup();
        if links.is_empty() {
            return None;
        }
        let reports_confidence = buttons.iter().any(|c| button_is(c, CONFIDENCE))
            || values.iter().any(|c| value_is(c, CONFIDENCE));
        // A maximum such as 0xFFFF in two bytes reads as negative; positions
        // come unsigned, so it's meant unsigned.
        let logical_max = |c: &HIDP_VALUE_CAPS| {
            if c.LogicalMax >= c.LogicalMin || c.BitSize == 0 || c.BitSize >= 32 {
                c.LogicalMax
            } else {
                (c.LogicalMax as u32 & ((1u32 << c.BitSize) - 1)) as i32
            }
        };
        let per_mm = |c: &HIDP_VALUE_CAPS| {
            touchpad::per_mm(
                (c.LogicalMin, logical_max(c)),
                (c.PhysicalMin, c.PhysicalMax),
                c.Units,
                c.UnitsExp,
            )
        };
        Some(Device {
            data_length: caps.NumberInputDataIndices as u32,
            usages,
            links,
            pad: Pad {
                x_min: x.LogicalMin,
                x_max: logical_max(x),
                y_min: y.LogicalMin,
                y_max: logical_max(y),
                per_mm: (per_mm(x), per_mm(y)),
                reports_confidence,
            },
            assembler: Assembler::default(),
            prepared,
        })
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_INPUT => {
            if let Err(panic) = std::panic::catch_unwind(|| on_input(lparam)) {
                crate::log_panic(&panic);
            }
            // Windows tidies up the input after this.
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        WM_INPUT_DEVICE_CHANGE => {
            if let Err(panic) = std::panic::catch_unwind(on_device_change) {
                crate::log_panic(&panic);
            }
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == DECIDE_TIMER => {
            if let Err(panic) = std::panic::catch_unwind(on_timer) {
                crate::log_panic(&panic);
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// A touchpad came or went: reads them again, lifting fingers down.
fn on_device_change() {
    let lifted = STATE.with(|cell| {
        let mut state = cell.try_borrow_mut().ok()?;
        let state = state.as_mut()?;
        state.devices = touchpads();
        Some((state.lift_all(), state.on_touches))
    });
    // A touchpad taken away mid-touch never says its fingers lifted.
    if TOUCHING.swap(false, Ordering::SeqCst) {
        LIFTED_AT.store(now_ms().max(1), Ordering::SeqCst);
    }
    if let Some((touches, on_touches)) = lifted
        && !touches.is_empty()
    {
        on_touches(touches);
    }
}

/// Time to decide about a touch held back, perhaps.
fn on_timer() {
    let decided = STATE.with(|cell| {
        let mut state = cell.try_borrow_mut().ok()?;
        let state = state.as_mut()?;
        let out = state.recogniser.tick(now_ms());
        state.time();
        Some((out, state.on_touches))
    });
    if let Some((out, on_touches)) = decided
        && !out.is_empty()
    {
        on_touches(out);
    }
}

fn on_input(lparam: LPARAM) {
    let input = HRAWINPUT(lparam.0 as *mut _);
    let header_size = std::mem::size_of::<RAWINPUTHEADER>() as u32;
    let mut size = 0u32;
    unsafe { GetRawInputData(input, RID_INPUT, None, &mut size, header_size) };
    if size == 0 {
        return;
    }
    let mut buffer = vec![0u64; (size as usize).div_ceil(8)];
    let read = unsafe {
        GetRawInputData(
            input,
            RID_INPUT,
            Some(buffer.as_mut_ptr().cast()),
            &mut size,
            header_size,
        )
    };
    if read == u32::MAX {
        return;
    }
    let bytes: &mut [u8] =
        unsafe { std::slice::from_raw_parts_mut(buffer.as_mut_ptr().cast(), size as usize) };
    let header: RAWINPUTHEADER = unsafe { std::ptr::read_unaligned(bytes.as_ptr().cast()) };
    if header.dwType != RIM_TYPEHID.0 {
        return;
    }
    // RAWHID follows the header: each report's size, how many, then them.
    let at = header_size as usize;
    if bytes.len() < at + 8 {
        return;
    }
    let word = |i: usize| u32::from_ne_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
    let (report_size, report_count) = (word(at) as usize, word(at + 4) as usize);
    let reports = &mut bytes[at + 8..];
    if report_size == 0 || reports.len() < report_size * report_count {
        return;
    }

    let mut out = Vec::new();
    let mut on_touches = None;
    STATE.with(|cell| {
        let Ok(mut state) = cell.try_borrow_mut() else {
            return;
        };
        let Some(state) = state.as_mut() else {
            return;
        };
        on_touches = Some(state.on_touches);
        let Some(device) = state.devices.get_mut(&(header.hDevice.0 as isize)) else {
            return;
        };
        for report in reports.chunks_exact_mut(report_size).take(report_count) {
            let Some(values) = read_values(device, report) else {
                continue;
            };
            let Some(frame) = device.assembler.report(&device.links, &values, &device.pad) else {
                continue;
            };
            let touching = frame.iter().any(|c| c.touching);
            if TOUCHING.swap(touching, Ordering::SeqCst) && !touching {
                // 0 is never.
                LIFTED_AT.store(now_ms().max(1), Ordering::SeqCst);
            }
            if in_front() {
                let touches = state.fingers.frame(&device.pad, &frame);
                out.extend(state.recogniser.touches(now_ms(), &device.pad, touches));
            } else {
                // Another window is in front: its fingers aren't Android's.
                let lifts = state.fingers.lift_all();
                out.extend(state.recogniser.cancel(lifts));
            }
        }
        state.time();
    });
    if let Some(on_touches) = on_touches
        && !out.is_empty()
    {
        on_touches(out);
    }
}

/// The values in one report.
fn read_values(device: &Device, report: &mut [u8]) -> Option<Values> {
    let mut data = vec![HIDP_DATA::default(); device.data_length.max(1) as usize];
    let mut length = data.len() as u32;
    let status = unsafe {
        HidP_GetData(
            HidP_Input,
            data.as_mut_ptr(),
            &mut length,
            device.prepared(),
            report,
        )
    };
    if status.0 < 0 {
        return None;
    }
    Some(
        data[..(length as usize).min(data.len())]
            .iter()
            .filter_map(|d| {
                let usage = *device.usages.get(&d.DataIndex)?;
                Some((usage, unsafe { d.Anonymous.RawValue }))
            })
            .collect(),
    )
}

/// Keeps the touchpad's pointer movement, clicks and scrolling from
/// Windows while fingers are on it, or just were.
unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && ACTIVE.load(Ordering::SeqCst) {
        let info = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
        let lifted_at = LIFTED_AT.load(Ordering::SeqCst);
        let recent = TOUCHING.load(Ordering::SeqCst)
            || (lifted_at != 0 && now_ms().saturating_sub(lifted_at) < QUIET_MS);
        if info.flags & LLMHF_INJECTED == 0 && recent && in_front() {
            return LRESULT(1);
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// True when AAE's main window is the one in front.
fn in_front() -> bool {
    unsafe { GetForegroundWindow() == ui::main_window() }
}
