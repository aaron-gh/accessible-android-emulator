//! Key names, for sending keys from the command line and from shortcuts.
//!
//! The Mac app forwards physical keys by key code (see
//! [`crate::control::Controller::mac_key`]). This module is for naming keys in
//! text, such as `aae key "Pixel" back` or `meta+right`. TalkBack's current
//! keymap uses Meta (the Search key) as its modifier; the older one used Alt.

use std::fmt;

/// A key with the modifiers held while it is pressed. Names are W3C key values,
/// which is what the emulator understands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Key {
    pub name: String,
    pub modifiers: Vec<String>,
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for modifier in &self.modifiers {
            write!(f, "{modifier}+")?;
        }
        write!(f, "{}", self.name)
    }
}

/// Parses a key such as `back`, `enter`, `meta+right`, `ctrl+shift+a` or `F5`.
/// Names are not case sensitive. Returns `None` for a name it doesn't know.
pub fn parse(text: &str) -> Option<Key> {
    let parts: Vec<&str> = text.split('+').map(str::trim).collect();
    let (last, mods) = parts.split_last()?;
    let modifiers = mods
        .iter()
        .map(|m| modifier(m))
        .collect::<Option<Vec<_>>>()?;
    Some(Key {
        name: key_name(last)?,
        modifiers,
    })
}

fn modifier(name: &str) -> Option<String> {
    Some(
        match name.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => "Control",
            "alt" | "option" | "opt" => "Alt",
            "shift" => "Shift",
            "meta" | "cmd" | "command" | "search" | "win" | "super" => "Meta",
            _ => return None,
        }
        .to_string(),
    )
}

fn key_name(name: &str) -> Option<String> {
    let lower = name.to_ascii_lowercase();
    let named = match lower.as_str() {
        // Android's own buttons.
        "back" => "GoBack",
        "home" => "GoHome",
        "recents" | "overview" | "app-switch" => "AppSwitch",
        "power" => "Power",
        "volume-up" | "volup" => "AudioVolumeUp",
        "volume-down" | "voldown" => "AudioVolumeDown",
        "mute" => "AudioVolumeMute",
        // Keyboard keys.
        "enter" | "return" => "Enter",
        "escape" | "esc" => "Escape",
        "tab" => "Tab",
        "space" => " ",
        "backspace" => "Backspace",
        "delete" | "del" | "forward-delete" => "Delete",
        "up" => "ArrowUp",
        "down" => "ArrowDown",
        "left" => "ArrowLeft",
        "right" => "ArrowRight",
        "home-key" | "line-start" => "Home",
        "end" | "line-end" => "End",
        "page-up" | "pageup" => "PageUp",
        "page-down" | "pagedown" => "PageDown",
        "insert" => "Insert",
        "menu" | "context-menu" => "ContextMenu",
        "caps-lock" | "capslock" => "CapsLock",
        _ => "",
    };
    if !named.is_empty() {
        return Some(named.to_string());
    }
    if let Some(n) = lower.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()) {
        return (1..=12).contains(&n).then(|| format!("F{n}"));
    }
    let mut chars = name.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii_graphic() => Some(c.to_string()),
        _ => None,
    }
}

/// The Linux evdev code for a modifier name as stored in [`Key::modifiers`].
pub fn modifier_code(modifier: &str) -> Option<i32> {
    Some(match modifier {
        "Control" => 29,
        "Shift" => 42,
        "Alt" => 56,
        "Meta" => 125,
        _ => return None,
    })
}

/// The Linux evdev code for a key, plus whether Shift is needed to type it.
/// The codes assume AAE's full keyboard layout is in use on the device (see
/// `provision::apply_keyboard_layout`), which maps them as a PC keyboard does.
pub fn evdev_code(name: &str) -> Option<(i32, bool)> {
    const LETTERS: [i32; 26] = [
        30, 48, 46, 32, 18, 33, 34, 35, 23, 36, 37, 38, 50, 49, 24, 25, 16, 19, 31, 20, 22, 47, 17,
        45, 21, 44,
    ];
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        if c.is_ascii_alphabetic() {
            let index = (c.to_ascii_lowercase() as u8 - b'a') as usize;
            return Some((LETTERS[index], c.is_ascii_uppercase()));
        }
        let plain = |code| Some((code, false));
        let shifted = |code| Some((code, true));
        return match c {
            '1'..='9' => plain(c as i32 - '1' as i32 + 2),
            '0' => plain(11),
            '!' => shifted(2),
            '@' => shifted(3),
            '#' => shifted(4),
            '$' => shifted(5),
            '%' => shifted(6),
            '^' => shifted(7),
            '&' => shifted(8),
            '*' => shifted(9),
            '(' => shifted(10),
            ')' => shifted(11),
            '-' => plain(12),
            '_' => shifted(12),
            '=' => plain(13),
            '+' => shifted(13),
            '[' => plain(26),
            '{' => shifted(26),
            ']' => plain(27),
            '}' => shifted(27),
            ';' => plain(39),
            ':' => shifted(39),
            '\'' => plain(40),
            '"' => shifted(40),
            '`' => plain(41),
            '~' => shifted(41),
            '\\' => plain(43),
            '|' => shifted(43),
            ',' => plain(51),
            '<' => shifted(51),
            '.' => plain(52),
            '>' => shifted(52),
            '/' => plain(53),
            '?' => shifted(53),
            ' ' => plain(57),
            _ => None,
        };
    }
    if let Some(n) = name.strip_prefix('F').and_then(|n| n.parse::<i32>().ok()) {
        return match n {
            1..=10 => Some((58 + n, false)),
            11 => Some((87, false)),
            12 => Some((88, false)),
            _ => None,
        };
    }
    let code = match name {
        // Android's own buttons. Home is 172 (KEY_HOMEPAGE), not 102: AAE's
        // keyboard layout keeps 102 for the keyboard's Home key.
        "GoBack" => 158,
        "GoHome" => 172,
        "AppSwitch" => 580,
        "Power" => 116,
        "AudioVolumeUp" => 115,
        "AudioVolumeDown" => 114,
        "AudioVolumeMute" => 113,
        "Escape" => 1,
        "Backspace" => 14,
        "Tab" => 15,
        "Enter" => 28,
        "CapsLock" => 58,
        "Home" => 102,
        "ArrowUp" => 103,
        "PageUp" => 104,
        "ArrowLeft" => 105,
        "ArrowRight" => 106,
        "End" => 107,
        "ArrowDown" => 108,
        "PageDown" => 109,
        "Insert" => 110,
        "Delete" => 111,
        "ContextMenu" => 127,
        _ => return None,
    };
    Some((code, false))
}

/// The Linux evdev code for a macOS virtual key code (`kVK_*`), as AppKit
/// reports in `NSEvent.keyCode`. This is how the Mac app forwards keys.
///
/// Command becomes Meta, which TalkBack's keymap uses as its modifier, and
/// Option becomes Alt. The Mac Help key sits where PC keyboards have Insert,
/// so it becomes Insert. Fn has no Android equivalent and returns `None`.
pub fn mac_to_evdev(keycode: u16) -> Option<i32> {
    Some(match keycode {
        0x00 => 30,  // A
        0x01 => 31,  // S
        0x02 => 32,  // D
        0x03 => 33,  // F
        0x04 => 35,  // H
        0x05 => 34,  // G
        0x06 => 44,  // Z
        0x07 => 45,  // X
        0x08 => 46,  // C
        0x09 => 47,  // V
        0x0A => 86,  // ISO section, the extra key on ISO keyboards
        0x0B => 48,  // B
        0x0C => 16,  // Q
        0x0D => 17,  // W
        0x0E => 18,  // E
        0x0F => 19,  // R
        0x10 => 21,  // Y
        0x11 => 20,  // T
        0x12 => 2,   // 1
        0x13 => 3,   // 2
        0x14 => 4,   // 3
        0x15 => 5,   // 4
        0x16 => 7,   // 6
        0x17 => 6,   // 5
        0x18 => 13,  // =
        0x19 => 10,  // 9
        0x1A => 8,   // 7
        0x1B => 12,  // -
        0x1C => 9,   // 8
        0x1D => 11,  // 0
        0x1E => 27,  // ]
        0x1F => 24,  // O
        0x20 => 22,  // U
        0x21 => 26,  // [
        0x22 => 23,  // I
        0x23 => 25,  // P
        0x24 => 28,  // Return
        0x25 => 38,  // L
        0x26 => 36,  // J
        0x27 => 40,  // '
        0x28 => 37,  // K
        0x29 => 39,  // ;
        0x2A => 43,  // backslash
        0x2B => 51,  // ,
        0x2C => 53,  // /
        0x2D => 49,  // N
        0x2E => 50,  // M
        0x2F => 52,  // .
        0x30 => 15,  // Tab
        0x31 => 57,  // Space
        0x32 => 41,  // `
        0x33 => 14,  // Delete, which is Backspace
        0x35 => 1,   // Escape
        0x36 => 126, // Right Command, as Meta
        0x37 => 125, // Command, as Meta
        0x38 => 42,  // Shift
        0x39 => 58,  // Caps Lock
        0x3A => 56,  // Option, as Alt
        0x3B => 29,  // Control
        0x3C => 54,  // Right Shift
        0x3D => 100, // Right Option
        0x3E => 97,  // Right Control
        0x40 => 187, // F17
        0x41 => 83,  // keypad .
        0x43 => 55,  // keypad *
        0x45 => 78,  // keypad +
        0x47 => 69,  // keypad Clear, as Num Lock
        0x48 => 115, // Volume Up
        0x49 => 114, // Volume Down
        0x4A => 113, // Mute
        0x4B => 98,  // keypad /
        0x4C => 96,  // keypad Enter
        0x4E => 74,  // keypad -
        0x4F => 188, // F18
        0x50 => 189, // F19
        0x51 => 117, // keypad =
        0x52 => 82,  // keypad 0
        0x53 => 79,  // keypad 1
        0x54 => 80,  // keypad 2
        0x55 => 81,  // keypad 3
        0x56 => 75,  // keypad 4
        0x57 => 76,  // keypad 5
        0x58 => 77,  // keypad 6
        0x59 => 71,  // keypad 7
        0x5A => 190, // F20
        0x5B => 72,  // keypad 8
        0x5C => 73,  // keypad 9
        0x60 => 63,  // F5
        0x61 => 64,  // F6
        0x62 => 65,  // F7
        0x63 => 61,  // F3
        0x64 => 66,  // F8
        0x65 => 67,  // F9
        0x67 => 87,  // F11
        0x69 => 183, // F13
        0x6A => 186, // F16
        0x6B => 184, // F14
        0x6D => 68,  // F10
        0x6F => 88,  // F12
        0x71 => 185, // F15
        0x72 => 110, // Help, where PC keyboards have Insert
        0x73 => 102, // Home
        0x74 => 104, // Page Up
        0x75 => 111, // Forward Delete
        0x76 => 62,  // F4
        0x77 => 107, // End
        0x78 => 60,  // F2
        0x79 => 109, // Page Down
        0x7A => 59,  // F1
        0x7B => 105, // Left
        0x7C => 106, // Right
        0x7D => 108, // Down
        0x7E => 103, // Up
        _ => return None,
    })
}

/// True for the evdev codes of modifier keys.
pub fn is_modifier(code: i32) -> bool {
    matches!(code, 29 | 42 | 54 | 56 | 97 | 100 | 125 | 126)
}

/// Names accepted by [`parse`], for help text.
pub const HELP: &str = "Android buttons: back, home, recents, power, volume-up, volume-down, mute. \
Keys: enter, escape, tab, space, backspace, delete, up, down, left, right, home-key, end, \
page-up, page-down, insert, menu, F1 to F12, or any single character. \
Add modifiers with plus: ctrl, alt, shift, meta. For example meta+right, which moves TalkBack to the next item.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_buttons_and_keys() {
        assert_eq!(parse("Back").unwrap().name, "GoBack");
        assert_eq!(parse("f5").unwrap().name, "F5");
        assert_eq!(parse("a").unwrap().name, "a");
        assert!(parse("f13").is_none());
        assert!(parse("nonsense").is_none());
    }

    #[test]
    fn maps_keys_to_evdev_codes() {
        assert_eq!(evdev_code("a"), Some((30, false)));
        assert_eq!(evdev_code("A"), Some((30, true)));
        assert_eq!(evdev_code("?"), Some((53, true)));
        assert_eq!(evdev_code("ArrowRight"), Some((106, false)));
        assert_eq!(evdev_code("F12"), Some((88, false)));
        assert_eq!(evdev_code("GoBack"), Some((158, false)));
        assert_eq!(evdev_code("GoHome"), Some((172, false)));
        assert_eq!(evdev_code("Nonsense"), None);
        assert_eq!(modifier_code("Alt"), Some(56));
    }

    #[test]
    fn maps_mac_key_codes() {
        assert_eq!(mac_to_evdev(0x00), Some(30)); // A
        assert_eq!(mac_to_evdev(0x37), Some(125)); // Command to Meta
        assert_eq!(mac_to_evdev(0x3A), Some(56)); // Option to Alt
        assert_eq!(mac_to_evdev(0x7C), Some(106)); // Right arrow
        assert_eq!(mac_to_evdev(0x35), Some(1)); // Escape
        assert_eq!(mac_to_evdev(0x3F), None); // Fn
        assert!(is_modifier(125));
        assert!(!is_modifier(30));
    }

    #[test]
    fn parses_modifiers() {
        let key = parse("ctrl+shift+a").unwrap();
        assert_eq!(key.modifiers, vec!["Control", "Shift"]);
        assert_eq!(key.to_string(), "Control+Shift+a");
        assert!(parse("hyper+a").is_none());
    }
}
