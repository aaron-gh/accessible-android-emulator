//! Using a device from the terminal: its audio plays here and the keyboard goes to it.
//!
//! This is the command-line stand-in for the Mac app's device mode. A terminal
//! only sees the keys it is sent, so Command combinations and some Option
//! combinations never reach it, and VoiceOver may echo keys as you type. Turn
//! VoiceOver off while attached to hear only the device's screen reader.
//!
//! Terminals that support the kitty keyboard protocol (iTerm2, Ghostty, kitty,
//! WezTerm) report every key with its modifiers, and AAE turns that on when it
//! can. macOS Terminal doesn't: it turns Option-Left and Option-Right into
//! "word back" and "word forward", which look the same as Option-B and Option-F.

use std::time::Duration;

use aae_core::audio::AudioPlayer;
use aae_core::device::Device;
use aae_core::emulator;
use aae_core::keys::Key;
use aae_core::sdk::Sdk;
use anyhow::Result;
use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, KeyboardEnhancementFlags,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::terminal;

/// Sends the terminal's keyboard to the device and plays its audio until
/// Control-] is pressed. Plain Escape goes to the device.
///
/// A terminal never sees the Command key, so by default Option is sent as
/// Android's Meta key, which TalkBack's current keymap uses as its modifier.
/// With `keep_alt`, Option is sent as Alt.
pub async fn run(sdk: &Sdk, device: &Device, keep_alt: bool, correct_pitch: bool) -> Result<()> {
    let (_, controller, _) = emulator::attach(sdk, device).await?;
    let audio =
        AudioPlayer::start_with_speed(&controller, playback_speed(device, correct_pitch)).await?;
    audio.set_volume(device.meta.playback_volume.unwrap_or(1.0));
    println!(
        "Keyboard is in {}. Press Control-right bracket to return to the terminal.",
        device.meta.name
    );
    if !keep_alt {
        println!(
            "Option acts as Android's Meta key, the screen reader's modifier. \
             Terminal must have Use Option as Meta key turned on."
        );
    }

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let raw = RawMode::enable()?;
    if !raw.enhanced {
        // Raw mode needs a carriage return to start a new line.
        print!(
            "This terminal can't report every key, so Option with the arrow keys may not reach the device.\r\n\
             In macOS Terminal, open Settings, Profiles, Keyboard, and set Option-Left to send \\033[1;3D \
             and Option-Right to send \\033[1;3C. Or use iTerm2, Ghostty or kitty.\r\n"
        );
    }
    let reader = std::thread::spawn(move || {
        loop {
            match event::poll(Duration::from_millis(100)) {
                Ok(true) => match event::read() {
                    Ok(Event::Key(key)) => {
                        let detach = is_detach(&key);
                        if tx.send(key).is_err() || detach {
                            break;
                        }
                    }
                    Ok(_) => {}
                    Err(_) => break,
                },
                Ok(false) => {
                    if tx.is_closed() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let mut failure = None;
    while let Some(event) = rx.recv().await {
        if is_detach(&event) {
            break;
        }
        if event.kind == KeyEventKind::Release {
            continue;
        }
        if let Some(mut key) = to_device_key(&event) {
            if !keep_alt {
                for modifier in &mut key.modifiers {
                    if modifier == "Alt" {
                        *modifier = "Meta".to_string();
                    }
                }
            }
            if let Err(e) = controller.press(&key).await {
                failure = Some(e);
                break;
            }
        }
    }
    drop(rx);
    let _ = reader.join();
    drop(raw);
    drop(audio);
    match failure {
        Some(e) => Err(anyhow::anyhow!(
            "The keyboard connection to {} stopped: {e}",
            device.meta.name
        )),
        None => {
            println!("Keyboard is back in the terminal.");
            Ok(())
        }
    }
}

/// The speed to correct the device's audio for: its measured speed, or none
/// when the user turned pitch correction off.
fn playback_speed(device: &Device, correct_pitch: bool) -> f64 {
    if correct_pitch {
        device.meta.audio_speed.unwrap_or(1.0)
    } else {
        1.0
    }
}

/// Plays the device's audio until Control-C, then reports how it went.
pub async fn listen(sdk: &Sdk, device: &Device, correct_pitch: bool) -> Result<()> {
    let (_, controller, _) = emulator::attach(sdk, device).await?;
    let audio =
        AudioPlayer::start_with_speed(&controller, playback_speed(device, correct_pitch)).await?;
    audio.set_volume(device.meta.playback_volume.unwrap_or(1.0));
    println!(
        "Playing {}'s audio at {} hertz. Press Control-C to stop.",
        device.meta.name, audio.output_rate
    );
    tokio::signal::ctrl_c().await?;
    use std::sync::atomic::Ordering::Relaxed;
    let delay = match audio.stats.average_delay() {
        Some(d) => format!(
            " Audio took {} milliseconds on average to arrive from the device.",
            d.as_millis()
        ),
        None => String::new(),
    };
    println!(
        "Stopped. {} packets received, loudest {:.0}%, {} gaps, {} samples dropped to keep up.{delay}",
        audio.stats.packets.load(Relaxed),
        audio.stats.peak() * 100.0,
        audio.stats.underruns.load(Relaxed),
        audio.stats.dropped.load(Relaxed)
    );
    Ok(())
}

/// Control-] returns to the terminal. Terminals report it as Control-] or,
/// because it is the same control character, Control-5.
fn is_detach(key: &KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char(']') | KeyCode::Char('5'))
}

fn to_device_key(event: &KeyEvent) -> Option<Key> {
    let mods = event.modifiers;
    let mut modifiers = Vec::new();
    if mods.contains(KeyModifiers::CONTROL) {
        modifiers.push("Control".to_string());
    }
    if mods.contains(KeyModifiers::ALT) {
        modifiers.push("Alt".to_string());
    }
    if mods.contains(KeyModifiers::SUPER) || mods.contains(KeyModifiers::META) {
        modifiers.push("Meta".to_string());
    }
    let name = match event.code {
        // A shifted character arrives as the character itself, such as "A".
        KeyCode::Char(c) if modifiers.is_empty() => c.to_string(),
        KeyCode::Char(c) => {
            if c.is_ascii_uppercase() {
                modifiers.push("Shift".to_string());
            }
            c.to_ascii_lowercase().to_string()
        }
        KeyCode::BackTab => {
            modifiers.push("Shift".to_string());
            "Tab".to_string()
        }
        other => {
            if mods.contains(KeyModifiers::SHIFT) {
                modifiers.push("Shift".to_string());
            }
            match other {
                KeyCode::Enter => "Enter",
                KeyCode::Esc => "Escape",
                KeyCode::Tab => "Tab",
                KeyCode::Backspace => "Backspace",
                KeyCode::Delete => "Delete",
                KeyCode::Up => "ArrowUp",
                KeyCode::Down => "ArrowDown",
                KeyCode::Left => "ArrowLeft",
                KeyCode::Right => "ArrowRight",
                KeyCode::Home => "Home",
                KeyCode::End => "End",
                KeyCode::PageUp => "PageUp",
                KeyCode::PageDown => "PageDown",
                KeyCode::Insert => "Insert",
                KeyCode::F(n) => {
                    return Some(Key {
                        name: format!("F{n}"),
                        modifiers,
                    });
                }
                _ => return None,
            }
            .to_string()
        }
    };
    Some(Key { name, modifiers })
}

/// Puts the terminal in raw mode, with full key reporting where the terminal
/// supports it, and restores it when dropped, even on error.
struct RawMode {
    enhanced: bool,
}

impl RawMode {
    fn enable() -> Result<Self> {
        terminal::enable_raw_mode()?;
        let enhanced = terminal::supports_keyboard_enhancement().unwrap_or(false)
            && crossterm::execute!(
                std::io::stdout(),
                PushKeyboardEnhancementFlags(
                    KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                        | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
                )
            )
            .is_ok();
        Ok(RawMode { enhanced })
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        if self.enhanced {
            let _ = crossterm::execute!(std::io::stdout(), PopKeyboardEnhancementFlags);
        }
        let _ = terminal::disable_raw_mode();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn escape_goes_to_the_device() {
        let k = key(KeyCode::Esc, KeyModifiers::NONE);
        assert!(!is_detach(&k));
        assert_eq!(to_device_key(&k).unwrap().name, "Escape");
    }

    #[test]
    fn control_bracket_detaches() {
        assert!(is_detach(&key(KeyCode::Char(']'), KeyModifiers::CONTROL)));
        assert!(is_detach(&key(KeyCode::Char('5'), KeyModifiers::CONTROL)));
        assert!(!is_detach(&key(KeyCode::Char(']'), KeyModifiers::NONE)));
    }

    #[test]
    fn maps_modified_keys() {
        let k = to_device_key(&key(KeyCode::Right, KeyModifiers::ALT)).unwrap();
        assert_eq!(k.to_string(), "Alt+ArrowRight");
        let k = to_device_key(&key(KeyCode::BackTab, KeyModifiers::SHIFT)).unwrap();
        assert_eq!(k.to_string(), "Shift+Tab");
        let k = to_device_key(&key(KeyCode::Char('A'), KeyModifiers::SHIFT)).unwrap();
        assert_eq!(k.to_string(), "A");
    }
}
