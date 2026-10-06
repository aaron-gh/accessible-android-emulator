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
/// Android's own buttons (back, home and so on) return `None` and are sent by
/// name, which the emulator handles.
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
        assert_eq!(evdev_code("GoBack"), None);
        assert_eq!(modifier_code("Alt"), Some(56));
    }

    #[test]
    fn parses_modifiers() {
        let key = parse("ctrl+shift+a").unwrap();
        assert_eq!(key.modifiers, vec!["Control", "Shift"]);
        assert_eq!(key.to_string(), "Control+Shift+a");
        assert!(parse("hyper+a").is_none());
    }
}
