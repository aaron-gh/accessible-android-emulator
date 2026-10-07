//! Speech bridge output on Windows: NVDA 2024.1 or later (SSML with an end
//! mark for completion), otherwise SAPI 5 with the default voice and rate, or
//! an installed voice for another language. Completion and stops are reported
//! to the device.
//!
//! One thread per device, as SAPI objects are bound to their thread.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use crate::screen_readers::nvda;

use aae_ffi::{Session, SpeechBridgeListener};
use windows::Win32::Globalization::{GetUserDefaultLocaleName, LocaleNameToLCID};
use windows::Win32::Media::Speech::{
    ISpObjectToken, ISpObjectTokenCategory, ISpVoice, SPCAT_VOICES, SPF_ASYNC, SPF_IS_NOT_XML,
    SPF_PURGEBEFORESPEAK, SPRS_DONE, SPVOICESTATUS, SpObjectTokenCategory, SpVoice,
};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::core::{HSTRING, PCWSTR};

enum Command {
    Speak {
        id: u64,
        text: String,
        language: String,
    },
    Stop,
    /// NVDA reached a mark.
    Mark(String),
}

/// Which thread is waiting for each NVDA mark.
static MARKS: Mutex<Option<HashMap<String, mpsc::Sender<Command>>>> = Mutex::new(None);
static NEXT_MARK: AtomicU64 = AtomicU64::new(1);

/// NVDA reached a mark: tells the thread that's waiting for it.
unsafe extern "system" fn mark_reached(name: PCWSTR) -> u32 {
    let name = unsafe { name.to_string() }.unwrap_or_default();
    if let Some(waiting) = MARKS.lock().unwrap().as_mut().and_then(|m| m.remove(&name)) {
        let _ = waiting.send(Command::Mark(name));
    }
    0
}

/// Speaks for one device. Dropping it ends its thread.
pub struct Speaker {
    commands: mpsc::Sender<Command>,
}

impl Speaker {
    pub fn start(session: &Arc<Session>) -> Arc<Speaker> {
        let (commands, received) = mpsc::channel();
        let session = Arc::downgrade(session);
        let own = commands.clone();
        let _ = std::thread::Builder::new()
            .name("AAE speech bridge".into())
            .spawn(move || run(received, own, session));
        Arc::new(Speaker { commands })
    }
}

impl SpeechBridgeListener for Speaker {
    fn speak(&self, id: u64, text: String, language: String, _rate: u32, _pitch: u32) {
        let _ = self.commands.send(Command::Speak { id, text, language });
    }

    fn stop(&self) {
        let _ = self.commands.send(Command::Stop);
    }
}

fn run(commands: mpsc::Receiver<Command>, own: mpsc::Sender<Command>, session: Weak<Session>) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    if let Ok(voice) = unsafe { CoCreateInstance::<_, ISpVoice>(&SpVoice, None, CLSCTX_ALL) } {
        speak_until_closed(&voice, &commands, &own, &session);
    }
    unsafe { CoUninitialize() };
}

fn finished(session: &Weak<Session>, id: u64) {
    if let Some(session) = session.upgrade() {
        session.speech_finished(id);
    }
}

/// How an utterance is being spoken.
enum Speaking {
    /// By SAPI, as its stream number.
    Sapi(u32),
    /// By NVDA, until this mark or this deadline. The mark is never reached
    /// if other speech interrupts it.
    Nvda(String, Instant),
}

fn speak_until_closed(
    voice: &ISpVoice,
    commands: &mpsc::Receiver<Command>,
    own: &mpsc::Sender<Command>,
    session: &Weak<Session>,
) {
    let default_voice = unsafe { voice.GetVoice() }.ok();
    let user_language = user_language();
    let mut listening_for_marks = false;
    // The utterance being spoken: the device's id for it, and how.
    let mut speaking: Option<(u64, Speaking)> = None;
    let stop = |speaking: &mut Option<(u64, Speaking)>| match speaking.take() {
        Some((id, Speaking::Sapi(_))) => {
            purge(voice);
            finished(session, id);
        }
        Some((id, Speaking::Nvda(mark, _))) => {
            nvda::cancel();
            forget_mark(&mark);
            finished(session, id);
        }
        None => {}
    };
    loop {
        // While speaking, look in often to notice the end.
        let wait = if speaking.is_some() {
            Duration::from_millis(15)
        } else {
            Duration::from_secs(3600)
        };
        match commands.recv_timeout(wait) {
            Ok(Command::Speak { id, text, language }) => {
                stop(&mut speaking);
                if text.trim().is_empty() {
                    finished(session, id);
                    continue;
                }
                if nvda::speaks_ssml() {
                    if !listening_for_marks {
                        listening_for_marks = nvda::on_mark(mark_reached);
                    }
                    let mark = format!("aae-{}", NEXT_MARK.fetch_add(1, Ordering::Relaxed));
                    MARKS
                        .lock()
                        .unwrap()
                        .get_or_insert_with(HashMap::new)
                        .insert(mark.clone(), own.clone());
                    nvda::cancel();
                    if listening_for_marks
                        && nvda::speak_ssml(&ssml(&text, &language, &user_language, &mark))
                    {
                        // Far longer than it could take, even slowly.
                        let longest =
                            Duration::from_millis(5000 + text.chars().count() as u64 * 150);
                        speaking = Some((id, Speaking::Nvda(mark, Instant::now() + longest)));
                        continue;
                    }
                    forget_mark(&mark);
                }
                choose_voice(voice, &language, &user_language, default_voice.as_ref());
                let wide = HSTRING::from(text.as_str());
                let mut stream = 0u32;
                let flags = (SPF_ASYNC.0 | SPF_IS_NOT_XML.0 | SPF_PURGEBEFORESPEAK.0) as u32;
                if unsafe { voice.Speak(PCWSTR(wide.as_ptr()), flags, Some(&mut stream)) }.is_ok() {
                    speaking = Some((id, Speaking::Sapi(stream)));
                } else {
                    finished(session, id);
                }
            }
            Ok(Command::Stop) => stop(&mut speaking),
            Ok(Command::Mark(name)) => {
                if matches!(&speaking, Some((_, Speaking::Nvda(mark, _))) if *mark == name)
                    && let Some((id, _)) = speaking.take()
                {
                    finished(session, id);
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                stop(&mut speaking);
                return;
            }
        }
        let done = match &speaking {
            Some((_, Speaking::Sapi(stream))) => {
                let mut status = SPVOICESTATUS::default();
                unsafe { voice.GetStatus(&mut status, std::ptr::null_mut()) }.is_ok()
                    && status.ulCurrentStream >= *stream
                    && status.dwRunningState & SPRS_DONE.0 as u32 != 0
            }
            Some((_, Speaking::Nvda(_, deadline))) => Instant::now() >= *deadline,
            None => false,
        };
        if done && let Some((id, how)) = speaking.take() {
            if let Speaking::Nvda(mark, _) = how {
                forget_mark(&mark);
            }
            finished(session, id);
        }
    }
}

fn forget_mark(mark: &str) {
    if let Some(marks) = MARKS.lock().unwrap().as_mut() {
        marks.remove(mark);
    }
}

/// The utterance as SSML with `mark` at the end, in a `lang` element when
/// it isn't the user's language (for NVDA's automatic language switching).
fn ssml(text: &str, language: &str, user: &str, mark: &str) -> String {
    let escaped = text
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;");
    let body = if !language.is_empty() && primary(language) != user {
        format!("<lang xml:lang=\"{language}\">{escaped}</lang>")
    } else {
        escaped
    };
    format!("<speak>{body}<mark name=\"{mark}\"/></speak>")
}

/// Stops speaking at once.
fn purge(voice: &ISpVoice) {
    unsafe {
        let _ = voice.Speak(PCWSTR::null(), SPF_PURGEBEFORESPEAK.0 as u32, None);
    }
}

/// The user's language, such as "en", from Windows' settings.
fn user_language() -> String {
    let mut name = [0u16; 85];
    let length = unsafe { GetUserDefaultLocaleName(&mut name) };
    let name = String::from_utf16_lossy(&name[..(length.max(1) - 1) as usize]);
    primary(&name)
}

fn primary(tag: &str) -> String {
    tag.split(['-', '_']).next().unwrap_or("").to_lowercase()
}

/// Selects the default voice, or an installed voice for another language.
fn choose_voice(voice: &ISpVoice, language: &str, user: &str, default: Option<&ISpObjectToken>) {
    let wanted = if language.is_empty() || primary(language) == user {
        None
    } else {
        voice_for(language)
    };
    let token = wanted.as_ref().or(default);
    if let Some(token) = token {
        unsafe {
            let _ = voice.SetVoice(token);
        }
    }
}

/// An installed SAPI voice speaking a language, such as "fr-FR".
fn voice_for(language: &str) -> Option<ISpObjectToken> {
    let lcid = unsafe { LocaleNameToLCID(&HSTRING::from(language), 0) };
    if lcid == 0 {
        return None;
    }
    let category: ISpObjectTokenCategory =
        unsafe { CoCreateInstance(&SpObjectTokenCategory, None, CLSCTX_ALL) }.ok()?;
    unsafe { category.SetId(SPCAT_VOICES, false) }.ok()?;
    let query = HSTRING::from(format!("Language={:x}", lcid & 0xffff));
    let tokens = unsafe { category.EnumTokens(&query, PCWSTR::null()) }.ok()?;
    let mut token = None;
    unsafe { tokens.Next(1, &mut token, None) }.ok()?;
    token
}
