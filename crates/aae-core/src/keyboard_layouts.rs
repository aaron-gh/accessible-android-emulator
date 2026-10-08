//! AAE's keyboard layouts on the device, one per language Android has (see
//! android/helper/layouts/generate.py), and typing characters with them.
//!
//! Keys are sent by position, and the device's layout decides the character.
//! Where the computer's layout differs, such as a Mac's British layout and
//! Android's PC one, the apps send typed characters instead: [`Keys::stroke`]
//! finds the key, with Shift or AltGr, that types each one on the device.

use std::collections::HashMap;

pub use crate::keyboard_layouts_table::LAYOUTS;

/// One of AAE's layouts.
#[derive(Debug)]
pub struct Layout {
    /// Such as "english_uk".
    pub name: &'static str,
    /// Such as "English (UK)".
    pub label: &'static str,
    /// Language tags it's chosen for, such as "en-GB".
    pub languages: &'static [&'static str],
    /// Its name among the helper's layouts.
    pub descriptor: &'static str,
    /// The layout file.
    pub kcm: &'static str,
}

/// The helper's receiver that offers the layouts to Android.
const RECEIVER: &str = "io.github.aaron_gh.aae.helper/io.github.aaron_gh.aae.helper.KeyboardLayouts";

impl Layout {
    /// What Android selects it by.
    pub fn full_descriptor(&self) -> String {
        format!("{RECEIVER}/{}", self.descriptor)
    }

    /// The keys that type each character.
    pub fn keys(&self) -> Keys {
        Keys::parse(self.kcm)
    }
}

/// English (US), the layout devices have unless another is chosen.
pub fn default_layout() -> &'static Layout {
    &LAYOUTS[0]
}

/// The layout chosen for a device, or English (US).
pub fn for_device(meta: &crate::device::DeviceMeta) -> &'static Layout {
    meta.keyboard_layout
        .as_deref()
        .and_then(find)
        .unwrap_or_else(default_layout)
}

/// A layout by its name or label, ignoring case.
pub fn find(name: &str) -> Option<&'static Layout> {
    let name = name.trim();
    LAYOUTS
        .iter()
        .find(|l| l.name.eq_ignore_ascii_case(name) || l.label.eq_ignore_ascii_case(name))
}

/// The layout for the computer's keyboard: a Mac input source ID, such as
/// "com.apple.keylayout.British"; a Windows keyboard layout ID, such as
/// "00000809"; or a language tag, such as "de-CH" or "fr_FR.UTF-8".
/// English (US) if nothing matches.
pub fn for_host(id: &str) -> &'static Layout {
    let id = id.trim();
    if let Some(mac) = id.strip_prefix("com.apple.keylayout.") {
        return for_mac(mac);
    }
    if id.len() == 8 && id.chars().all(|c| c.is_ascii_hexdigit()) {
        return for_windows(id);
    }
    for_language(id).unwrap_or_else(default_layout)
}

/// The layout for a language tag, by the whole tag, then its language.
pub fn for_language(tag: &str) -> Option<&'static Layout> {
    // "fr_FR.UTF-8", as in LANG, is "fr-FR".
    let tag = tag.split('.').next().unwrap_or("").replace('_', "-");
    if tag.is_empty() || tag == "C" || tag == "POSIX" {
        return None;
    }
    let lower = tag.to_ascii_lowercase();
    let language = lower.split('-').next().unwrap_or("");
    LAYOUTS
        .iter()
        .find(|l| l.languages.iter().any(|t| t.eq_ignore_ascii_case(&lower)))
        .or_else(|| {
            LAYOUTS
                .iter()
                .find(|l| l.languages.iter().any(|t| t.eq_ignore_ascii_case(language)))
        })
        .or_else(|| {
            LAYOUTS.iter().find(|l| {
                l.languages
                    .iter()
                    .any(|t| t.split('-').next().is_some_and(|t| t.eq_ignore_ascii_case(language)))
            })
        })
}

/// Mac input sources, such as "British" or "German", whose names aren't languages.
fn for_mac(name: &str) -> &'static Layout {
    let name = name.to_ascii_lowercase();
    let pick = |n: &str| find(n).unwrap_or_else(default_layout);
    let table: &[(&str, &str)] = &[
        ("british", "english_uk"),
        ("irish", "english_uk"),
        ("usinternational", "english_us_intl"),
        ("abc-extended", "english_us_intl"),
        ("colemak", "english_us_colemak"),
        ("dvorak", "english_us_dvorak"),
        ("swissgerman", "swiss_german"),
        ("swiss german", "swiss_german"),
        ("swissfrench", "swiss_french"),
        ("swiss french", "swiss_french"),
        ("canadian", "french_ca"),
        ("belgian", "belgian"),
        ("german", "german"),
        ("austrian", "german"),
        ("french", "french"),
        ("latinamerican", "spanish_latin"),
        ("spanish", "spanish"),
        ("italian", "italian"),
        ("brazilian", "brazilian"),
        ("portuguese", "portuguese"),
        ("swedish", "swedish"),
        ("norwegian", "norwegian"),
        ("danish", "danish"),
        ("finnish", "finnish"),
        ("icelandic", "icelandic"),
        ("estonian", "estonian"),
        ("latvian", "latvian_qwerty"),
        ("lithuanian", "lithuanian"),
        ("polish", "polish"),
        ("czech-qwerty", "czech_qwerty"),
        ("czech", "czech"),
        ("slovak", "slovak"),
        ("hungarian", "hungarian"),
        ("romanian", "romanian"),
        ("croatian", "croatian_and_slovenian"),
        ("slovenian", "croatian_and_slovenian"),
        ("serbian-latin", "serbian_and_montenegrin_latin"),
        ("serbian", "serbian_and_montenegrin_cyrillic"),
        ("bulgarian-phonetic", "bulgarian_phonetic"),
        ("bulgarian", "bulgarian"),
        ("russian-pc", "russian"),
        ("russian", "russian_mac"),
        ("ukrainian", "ukrainian"),
        ("byelorussian", "belarusian"),
        ("greek", "greek"),
        ("hebrew", "hebrew"),
        ("arabic", "arabic"),
        ("persian", "persian"),
        ("turkish-standard", "turkish_f"),
        ("turkish", "turkish"),
        ("georgian", "georgian"),
        ("azeri", "azerbaijani"),
        ("mongolian", "mongolian"),
        ("thai-pattachote", "thai_pattachote"),
        ("thai", "thai_kedmanee"),
    ];
    table
        .iter()
        .find(|(prefix, _)| name.starts_with(prefix))
        .map(|(_, layout)| pick(layout))
        .unwrap_or_else(default_layout)
}

/// Windows keyboard layout IDs: the low four digits are the language, and a
/// few higher ones mark a variant.
fn for_windows(klid: &str) -> &'static Layout {
    let klid = klid.to_ascii_uppercase();
    let pick = |n: &str| find(n).unwrap_or_else(default_layout);
    let variants: &[(&str, &str)] = &[
        ("00010409", "english_us_dvorak"),
        ("00020409", "english_us_intl"),
        ("00010405", "czech_qwerty"),
        ("00000416", "brazilian"),
        ("00010416", "brazilian"),
    ];
    if let Some((_, name)) = variants.iter().find(|(id, _)| *id == klid) {
        return pick(name);
    }
    let languages: &[(&str, &str)] = &[
        ("0409", "en-US"),
        ("0809", "en-GB"),
        ("1809", "en-IE"),
        ("4009", "en-IN"),
        ("0407", "de-DE"),
        ("0C07", "de-AT"),
        ("0807", "de-CH"),
        ("040C", "fr-FR"),
        ("080C", "fr-BE"),
        ("0813", "nl-BE"),
        ("0C0C", "fr-CA"),
        ("100C", "fr-CH"),
        ("0410", "it"),
        ("040A", "es-ES"),
        ("0C0A", "es-ES"),
        ("080A", "es-MX"),
        ("0816", "pt-PT"),
        ("0416", "pt-BR"),
        ("041D", "sv"),
        ("0414", "nb"),
        ("0406", "da"),
        ("040B", "fi"),
        ("040F", "is"),
        ("0425", "et"),
        ("0426", "lv"),
        ("0427", "lt"),
        ("0415", "pl"),
        ("0405", "cs"),
        ("041B", "sk"),
        ("040E", "hu"),
        ("0418", "ro"),
        ("041A", "hr"),
        ("0424", "sl"),
        ("081A", "sr-Latn"),
        ("0C1A", "sr"),
        ("0402", "bg"),
        ("0419", "ru"),
        ("0422", "uk"),
        ("0423", "be"),
        ("0408", "el"),
        ("040D", "he"),
        ("0401", "ar"),
        ("0429", "fa"),
        ("041F", "tr"),
        ("0437", "ka"),
        ("042C", "az"),
        ("0450", "mn"),
        ("041E", "th"),
    ];
    let language = &klid[4..];
    languages
        .iter()
        .find(|(id, _)| *id == language)
        .and_then(|(_, tag)| for_language(tag))
        .unwrap_or_else(default_layout)
}

/// How to type a character: a key, by Linux key code, with Shift and AltGr.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stroke {
    pub key: i32,
    pub shift: bool,
    pub altgr: bool,
}

/// Left Shift and right Alt (AltGr), by Linux key code.
pub const SHIFT: i32 = 42;
pub const RIGHT_SHIFT: i32 = 54;
pub const ALTGR: i32 = 100;

/// The keys of a layout that type each character.
#[derive(Debug, Default)]
pub struct Keys {
    strokes: HashMap<char, Stroke>,
}

impl Keys {
    /// The key, with Shift or AltGr, that types `c`, if any does.
    pub fn stroke(&self, c: char) -> Option<Stroke> {
        self.strokes.get(&c).copied()
    }

    fn parse(kcm: &str) -> Keys {
        // Each key name's scan code; the lowest where several map to one,
        // as both the key beside Enter and the one beside left Shift may.
        let mut codes: HashMap<&str, i32> = HashMap::new();
        for line in kcm.lines() {
            let mut words = line.split_whitespace();
            if let (Some("map"), Some("key"), Some(code), Some(name)) =
                (words.next(), words.next(), words.next(), words.next())
                && let Ok(code) = code.parse::<i32>()
            {
                codes
                    .entry(name)
                    .and_modify(|c| *c = (*c).min(code))
                    .or_insert(code);
            }
        }
        let mut keys = Keys::default();
        let mut keypad = Vec::new();
        let mut current: Option<i32> = None;
        let mut on_keypad = false;
        for line in kcm.lines() {
            let line = line.trim();
            if let Some(name) = line.strip_prefix("key ").and_then(|r| r.strip_suffix('{')) {
                let name = name.trim();
                current = codes.get(name).copied();
                on_keypad = name.starts_with("NUMPAD_");
                continue;
            }
            if line.starts_with('}') {
                current = None;
                continue;
            }
            let Some(key) = current else { continue };
            let Some((modifiers, value)) = line.split_once(':') else {
                continue;
            };
            let Some(c) = character(value.trim()) else {
                continue;
            };
            for alternative in modifiers.split(',') {
                let mut shift = false;
                let mut altgr = false;
                let mut usable = true;
                for part in alternative.trim().split('+') {
                    match part.trim() {
                        "base" => {}
                        "shift" => shift = true,
                        "ralt" => altgr = true,
                        // Labels aren't typed; the rest need other modifiers.
                        _ => usable = false,
                    }
                }
                if !usable {
                    continue;
                }
                let stroke = Stroke { key, shift, altgr };
                if on_keypad {
                    keypad.push((c, stroke));
                } else {
                    keys.add(c, stroke);
                }
            }
        }
        // The main keys first: a digit is typed on the top row.
        for (c, stroke) in keypad {
            keys.add(c, stroke);
        }
        keys
    }

    /// Keeps the simplest way to type each character.
    fn add(&mut self, c: char, stroke: Stroke) {
        let cost = |s: &Stroke| s.shift as u8 + 2 * s.altgr as u8;
        match self.strokes.get(&c) {
            Some(old) if cost(old) <= cost(&stroke) => {}
            _ => {
                self.strokes.insert(c, stroke);
            }
        }
    }
}

/// A character as a layout file writes it: 'a', 'é', '\\' or '\''.
fn character(value: &str) -> Option<char> {
    let inner = value.strip_prefix('\'')?.strip_suffix('\'')?;
    let mut chars = inner.chars();
    match (chars.next()?, chars.next()) {
        ('\\', Some('u')) => {
            let hex: String = chars.collect();
            char::from_u32(u32::from_str_radix(&hex, 16).ok()?)
        }
        ('\\', Some(escaped)) if chars.next().is_none() => Some(match escaped {
            'n' => '\n',
            't' => '\t',
            other => other,
        }),
        (c, None) => Some(c),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_layout_types_letters() {
        for layout in LAYOUTS {
            let keys = layout.keys();
            assert!(!keys.strokes.is_empty(), "{} has no characters", layout.name);
        }
    }

    #[test]
    fn us_and_uk_type_symbols_where_their_keyboards_have_them() {
        let us = default_layout().keys();
        assert_eq!(us.stroke('a'), Some(Stroke { key: 30, shift: false, altgr: false }));
        assert_eq!(us.stroke('A'), Some(Stroke { key: 30, shift: true, altgr: false }));
        assert_eq!(us.stroke('@'), Some(Stroke { key: 3, shift: true, altgr: false }));
        assert_eq!(us.stroke('5'), Some(Stroke { key: 6, shift: false, altgr: false }));
        let uk = find("english_uk").unwrap().keys();
        assert_eq!(uk.stroke('"'), Some(Stroke { key: 3, shift: true, altgr: false }));
        assert_eq!(uk.stroke('@'), Some(Stroke { key: 40, shift: true, altgr: false }));
        assert_eq!(uk.stroke('£'), Some(Stroke { key: 4, shift: true, altgr: false }));
        assert_eq!(uk.stroke('#'), Some(Stroke { key: 43, shift: false, altgr: false }));
        assert_eq!(uk.stroke('\\'), Some(Stroke { key: 86, shift: false, altgr: false }));
    }

    #[test]
    fn german_swaps_y_and_z_and_has_altgr() {
        let de = find("german").unwrap().keys();
        assert_eq!(de.stroke('z'), Some(Stroke { key: 21, shift: false, altgr: false }));
        assert_eq!(de.stroke('y'), Some(Stroke { key: 44, shift: false, altgr: false }));
        assert_eq!(de.stroke('@'), Some(Stroke { key: 16, shift: false, altgr: true }));
        assert_eq!(de.stroke('ü'), Some(Stroke { key: 26, shift: false, altgr: false }));
    }

    #[test]
    fn the_computers_keyboard_chooses_a_layout() {
        assert_eq!(for_host("com.apple.keylayout.British").name, "english_uk");
        assert_eq!(for_host("com.apple.keylayout.US").name, "english_us");
        assert_eq!(for_host("com.apple.keylayout.ABC").name, "english_us");
        assert_eq!(for_host("com.apple.keylayout.SwissGerman").name, "swiss_german");
        assert_eq!(for_host("com.apple.keylayout.German").name, "german");
        assert_eq!(for_host("00000809").name, "english_uk");
        assert_eq!(for_host("00000407").name, "german");
        assert_eq!(for_host("00010409").name, "english_us_dvorak");
        assert_eq!(for_host("fr_FR.UTF-8").name, "french");
        assert_eq!(for_host("de-CH").name, "swiss_german");
        assert_eq!(for_host("pt-BR").name, "brazilian");
        assert_eq!(for_host("en_GB.UTF-8").name, "english_uk");
        assert_eq!(for_host("C").name, "english_us");
        assert_eq!(for_host("xx").name, "english_us");
    }
}
