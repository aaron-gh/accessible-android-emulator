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
    type GetProcessId = unsafe extern "system" fn(*mut u32) -> u32;
    /// SSML, symbol level, priority, and whether to return at once.
    type SpeakSsml = unsafe extern "system" fn(PCWSTR, i32, i32, u8) -> u32;
    /// Called with each SSML mark's name as NVDA reaches it.
    pub type MarkReached = unsafe extern "system" fn(PCWSTR) -> u32;
    type SetMarkCallback = unsafe extern "system" fn(Option<MarkReached>) -> u32;

    struct Client {
        test_if_running: NoArguments,
        speak_text: WithText,
        cancel_speech: NoArguments,
        braille_message: WithText,
        // Version 2 of the controller client, in NVDA 2024.1 and later.
        get_process_id: Option<GetProcessId>,
        speak_ssml: Option<SpeakSsml>,
        set_mark_callback: Option<SetMarkCallback>,
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
            get_process_id: find(s!("nvdaController_getProcessId"))
                .map(|f| unsafe { std::mem::transmute::<Found, GetProcessId>(f) }),
            speak_ssml: find(s!("nvdaController_speakSsml"))
                .map(|f| unsafe { std::mem::transmute::<Found, SpeakSsml>(f) }),
            set_mark_callback: find(s!("nvdaController_setOnSsmlMarkReachedCallback"))
                .map(|f| unsafe { std::mem::transmute::<Found, SetMarkCallback>(f) }),
        })
    }

    /// Whether NVDA is running and can speak SSML and say when it reaches
    /// its marks: NVDA 2024.1 or later. Older versions answer the version 2
    /// calls with an error (1717), so asking its process makes no sound.
    pub fn speaks_ssml() -> bool {
        let Some(client) = client() else {
            return false;
        };
        let (Some(get_process_id), Some(_), Some(_)) = (
            client.get_process_id,
            client.speak_ssml,
            client.set_mark_callback,
        ) else {
            return false;
        };
        let mut pid = 0u32;
        unsafe { (client.test_if_running)() == 0 && get_process_id(&mut pid) == 0 && pid != 0 }
    }

    /// Hears each SSML mark NVDA reaches, for this whole program.
    pub fn on_mark(callback: MarkReached) -> bool {
        client()
            .and_then(|c| c.set_mark_callback)
            .is_some_and(|set| unsafe { set(Some(callback)) } == 0)
    }

    /// Speaks SSML at normal priority, returning at once. False if NVDA
    /// didn't take it.
    pub fn speak_ssml(ssml: &str) -> bool {
        let Some(speak) = client().and_then(|c| c.speak_ssml) else {
            return false;
        };
        let ssml = ui::wide(ssml);
        // Symbols as NVDA's settings have them (-1); normal priority (0),
        // as "now" would resume what it interrupted; asynchronous.
        unsafe { speak(PCWSTR(ssml.as_ptr()), -1, 0, 1) == 0 }
    }

    /// Stops whatever NVDA is saying.
    pub fn cancel() {
        if let Some(client) = client() {
            unsafe {
                (client.cancel_speech)();
            }
        }
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
