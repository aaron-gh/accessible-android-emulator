//! AAE's announcements and tones.
//!
//! Announcements go to the user's screen reader, in their own voice and
//! settings: straight to NVDA or JAWS when one is running, and otherwise, for
//! Narrator and the rest, as UI Automation notifications. With no screen
//! reader, Windows' own voice speaks them, so news like "Windows keyboard on"
//! is never lost.
//!
//! Each announcement waits a moment first, as on the Mac, so the screen reader
//! finishes reading the menu item or button that caused it. Otherwise its
//! reading of the newly focused control would cut the announcement off.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::OnceLock;

use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};
use windows::Win32::Media::Speech::{
    ISpVoice, SPF_ASYNC, SPF_IS_NOT_XML, SPF_PURGEBEFORESPEAK, SpVoice,
};
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance};
use windows::Win32::UI::Accessibility::{
    NotificationKind_Other, NotificationProcessing_All, NotificationProcessing_ImportantMostRecent,
    UiaHostProviderFromHwnd, UiaRaiseNotificationEvent,
};
use windows::Win32::UI::WindowsAndMessaging::{
    KillTimer, SPI_GETSCREENREADER, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SetTimer,
    SystemParametersInfoW,
};
use windows::core::{BOOL, BSTR, PCWSTR};

use crate::screen_readers::{jaws, nvda};
use crate::ui;

/// The timer that sends waiting announcements.
pub const ANNOUNCE_TIMER: usize = 1;
const DELAY_MS: u32 = 250;

thread_local! {
    static WAITING: RefCell<VecDeque<(String, bool)>> = const { RefCell::new(VecDeque::new()) };
    static VOICE: RefCell<Option<ISpVoice>> = const { RefCell::new(None) };
}

/// Says something, after a moment. `interrupt` is for failures, which cut
/// off whatever else is being said.
pub fn announce(text: &str, interrupt: bool) {
    WAITING.with(|w| w.borrow_mut().push_back((text.to_string(), interrupt)));
    unsafe {
        SetTimer(Some(ui::main_window()), ANNOUNCE_TIMER, DELAY_MS, None);
    }
}

/// Sends the waiting announcements. Called when the timer fires.
pub fn on_timer() {
    unsafe {
        let _ = KillTimer(Some(ui::main_window()), ANNOUNCE_TIMER);
    }
    let waiting: Vec<(String, bool)> = WAITING.with(|w| w.borrow_mut().drain(..).collect());
    for (text, interrupt) in waiting {
        speak_now(&text, interrupt);
    }
}

fn speak_now(text: &str, interrupt: bool) {
    if nvda::say(text, interrupt) || jaws::say(text, interrupt) {
        return;
    }
    if screen_reader_running() && notify(text, interrupt).is_ok() {
        return;
    }
    speak_with_windows_voice(text, interrupt);
}

/// True when a screen reader has told Windows it's running, as NVDA, JAWS
/// and Narrator do.
pub fn screen_reader_running() -> bool {
    let mut on = BOOL(0);
    unsafe {
        SystemParametersInfoW(
            SPI_GETSCREENREADER,
            0,
            Some(&mut on as *mut BOOL as *mut _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
        .is_ok()
            && on.as_bool()
    }
}

/// Raises a UI Automation notification on AAE's main window.
fn notify(text: &str, interrupt: bool) -> windows::core::Result<()> {
    unsafe {
        let provider = UiaHostProviderFromHwnd(ui::main_window())?;
        UiaRaiseNotificationEvent(
            &provider,
            NotificationKind_Other,
            if interrupt {
                NotificationProcessing_ImportantMostRecent
            } else {
                NotificationProcessing_All
            },
            &BSTR::from(text),
            &BSTR::from("AAE"),
        )
    }
}

fn speak_with_windows_voice(text: &str, interrupt: bool) {
    VOICE.with(|cell| {
        let mut voice = cell.borrow_mut();
        if voice.is_none() {
            *voice = unsafe { CoCreateInstance::<_, ISpVoice>(&SpVoice, None, CLSCTX_ALL) }.ok();
        }
        if let Some(voice) = voice.as_ref() {
            let mut flags = (SPF_ASYNC.0 | SPF_IS_NOT_XML.0) as u32;
            if interrupt {
                flags |= SPF_PURGEBEFORESPEAK.0 as u32;
            }
            let text = ui::wide(text);
            unsafe {
                let _ = voice.Speak(PCWSTR(text.as_ptr()), flags, None);
            }
        }
    });
}

/// A short sound before each announcement, so the result is heard before the
/// words. AAE makes its own, as Windows' sounds may be the user's error sound.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Info,
    Success,
    Failure,
    Progress,
}

impl Tone {
    pub fn play(self) {
        if !crate::settings::get().play_sounds {
            return;
        }
        static SOUNDS: OnceLock<[Vec<u8>; 4]> = OnceLock::new();
        let sounds = SOUNDS.get_or_init(|| {
            [Tone::Info, Tone::Success, Tone::Failure, Tone::Progress].map(|t| wave(t.notes()))
        });
        let sound = &sounds[self as usize];
        unsafe {
            let _ = PlaySoundW(
                PCWSTR(sound.as_ptr() as *const u16),
                None,
                SND_MEMORY | SND_ASYNC | SND_NODEFAULT,
            );
        }
    }

    /// Frequency in hertz and length in seconds of each note, as on the Mac.
    fn notes(self) -> &'static [(f64, f64)] {
        match self {
            Tone::Info => &[(880.0, 0.06)],
            Tone::Success => &[(660.0, 0.06), (990.0, 0.08)],
            Tone::Failure => &[(330.0, 0.09), (220.0, 0.14)],
            Tone::Progress => &[(1320.0, 0.03)],
        }
    }
}

/// Sine waves with a quick fade in and out, so they click less and sound
/// soft, as a WAV file in memory.
fn wave(notes: &[(f64, f64)]) -> Vec<u8> {
    const RATE: f64 = 44_100.0;
    const VOLUME: f64 = 0.35;
    let gap = (RATE * 0.015) as usize;
    let mut samples: Vec<i16> = Vec::new();
    for &(frequency, length) in notes {
        let count = (RATE * length) as usize;
        let fade = (count / 4).min((RATE * 0.01) as usize).max(1);
        for i in 0..count {
            let envelope = (i.min(count - 1 - i).min(fade) as f64 / fade as f64).min(1.0);
            let value = (2.0 * std::f64::consts::PI * frequency * i as f64 / RATE).sin();
            samples.push((value * envelope * VOLUME * i16::MAX as f64) as i16);
        }
        samples.extend(std::iter::repeat_n(0, gap));
    }
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&(RATE as u32).to_le_bytes());
    out.extend_from_slice(&(RATE as u32 * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}
