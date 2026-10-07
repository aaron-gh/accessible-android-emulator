//! Speaking straight through NVDA and JAWS, the way each one offers to other
//! programs, as Tolk does. This is more dependable than UI Automation
//! notifications, which each screen reader treats in its own way.

/// NVDA, through NV Access's controller client, `nvdaControllerClient.dll`,
/// shipped next to AAE. It's loaded when first needed, so AAE works without
/// it, and without NVDA.
pub mod nvda {
    use std::sync::OnceLock;

    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    use windows::core::{PCSTR, PCWSTR, s};

    use crate::ui;

    type NoArguments = unsafe extern "system" fn() -> u32;
    type WithText = unsafe extern "system" fn(PCWSTR) -> u32;

    struct Client {
        test_if_running: NoArguments,
        speak_text: WithText,
        cancel_speech: NoArguments,
        braille_message: WithText,
    }

    fn client() -> Option<&'static Client> {
        static CLIENT: OnceLock<Option<Client>> = OnceLock::new();
        CLIENT.get_or_init(load).as_ref()
    }

    fn load() -> Option<Client> {
        let path = std::env::current_exe()
            .ok()?
            .parent()?
            .join("nvdaControllerClient.dll");
        let path = ui::wide(&path.to_string_lossy());
        let module = unsafe { LoadLibraryW(PCWSTR(path.as_ptr())) }.ok()?;
        let find = |name: PCSTR| unsafe { GetProcAddress(module, name) };
        // Each is looked up as a general function, then called as what it is.
        type Found = unsafe extern "system" fn() -> isize;
        let none = |f: Found| unsafe { std::mem::transmute::<Found, NoArguments>(f) };
        let text = |f: Found| unsafe { std::mem::transmute::<Found, WithText>(f) };
        Some(Client {
            test_if_running: none(find(s!("nvdaController_testIfRunning"))?),
            speak_text: text(find(s!("nvdaController_speakText"))?),
            cancel_speech: none(find(s!("nvdaController_cancelSpeech"))?),
            braille_message: text(find(s!("nvdaController_brailleMessage"))?),
        })
    }

    /// Speaks, and shows on a braille display, if NVDA is running. Returns
    /// false if it isn't, or didn't take it.
    pub fn say(text: &str, interrupt: bool) -> bool {
        let Some(client) = client() else {
            return false;
        };
        let text = ui::wide(text);
        unsafe {
            if (client.test_if_running)() != 0 {
                return false;
            }
            if interrupt {
                (client.cancel_speech)();
            }
            let spoken = (client.speak_text)(PCWSTR(text.as_ptr())) == 0;
            (client.braille_message)(PCWSTR(text.as_ptr()));
            spoken
        }
    }
}

/// JAWS, through the speech interface JAWS installs, `FreedomSci.JawsApi`.
pub mod jaws {
    use std::cell::RefCell;

    use windows::Win32::Foundation::VARIANT_BOOL;
    use windows::Win32::System::Com::{
        CLSCTX_ALL, CLSIDFromProgID, CoCreateInstance, DISPATCH_METHOD, DISPPARAMS, IDispatch,
    };
    use windows::Win32::System::Variant::{VARIANT, VT_BOOL};
    use windows::Win32::UI::WindowsAndMessaging::FindWindowW;
    use windows::core::{GUID, PCWSTR, w};

    thread_local! {
        static API: RefCell<Option<(IDispatch, i32)>> = const { RefCell::new(None) };
    }

    /// True when JAWS is running: its window is there.
    fn running() -> bool {
        unsafe { FindWindowW(w!("JFWUI2"), PCWSTR::null()) }.is_ok_and(|h| !h.is_invalid())
    }

    /// JAWS's speech interface, and the number of its SayString method.
    fn api() -> Option<(IDispatch, i32)> {
        API.with(|cell| {
            if let Some(api) = cell.borrow().as_ref() {
                return Some(api.clone());
            }
            let api = unsafe {
                let class = CLSIDFromProgID(w!("FreedomSci.JawsApi")).ok()?;
                let dispatch: IDispatch = CoCreateInstance(&class, None, CLSCTX_ALL).ok()?;
                let name = w!("SayString");
                let mut id = 0;
                dispatch
                    .GetIDsOfNames(&GUID::zeroed(), &name, 1, 0x0400, &mut id)
                    .ok()?;
                (dispatch, id)
            };
            *cell.borrow_mut() = Some(api.clone());
            Some(api)
        })
    }

    /// Speaks, if JAWS is running. Returns false if it isn't, or didn't take it.
    pub fn say(text: &str, interrupt: bool) -> bool {
        if !running() {
            return false;
        }
        let Some((dispatch, id)) = api() else {
            return false;
        };
        // SayString(text, flush): arguments go in last first.
        let mut flush = VARIANT::default();
        unsafe {
            let inner = &mut *flush.Anonymous.Anonymous;
            inner.vt = VT_BOOL;
            inner.Anonymous.boolVal = VARIANT_BOOL::from(interrupt);
        }
        let mut arguments = [flush, VARIANT::from(text)];
        let parameters = DISPPARAMS {
            rgvarg: arguments.as_mut_ptr(),
            rgdispidNamedArgs: std::ptr::null_mut(),
            cArgs: 2,
            cNamedArgs: 0,
        };
        let called = unsafe {
            dispatch.Invoke(
                id,
                &GUID::zeroed(),
                0x0400,
                DISPATCH_METHOD,
                &parameters,
                None,
                None,
                None,
            )
        };
        if called.is_err() {
            // JAWS may have restarted; ask for its interface again next time.
            API.with(|cell| *cell.borrow_mut() = None);
            return false;
        }
        true
    }
}
