//! Gesture mode's keys, as on the Mac. Every key is free, so no modifiers are
//! needed. Gestures happen at the touch point, which starts in the middle of
//! the screen:
//!
//! - An arrow swipes that way when released. Holding one arrow and pressing
//!   another performs a two-part swipe, such as up then left, at once.
//! - Space, Enter or D double taps, T taps, and R triple taps. H double taps
//!   and holds, and L touches and holds, for as long as the key is held.
//! - Holding 2, 3 or 4 while pressing an arrow or a tap key uses that many
//!   fingers.
//! - Tab and Shift-Tab move the touch point to the next or previous thing on
//!   the screen; Shift-arrows move it a step. C puts it back in the middle,
//!   and W says what's there.
//! - Question mark reads these keys out.
//!
//! Keys are scan codes, with 0x100 added for extended keys. Arrows, Tab,
//! Space, Enter and the digits 2 to 4 are where they are on every keyboard.
//! Letters and question mark are the keys that type them on the keyboard
//! layout in use (see [`KeyMap`]), so W is W on a French keyboard too. The
//! crown keys are the brackets, or on layouts that need AltGr for them, such
//! as French, the two keys where US brackets are.

use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GestureAction {
    /// Perform a gesture, such as "swipe-up-then-left", at the touch point.
    Gesture(String),
    /// Perform a gesture and leave its last touch down, until `Release`.
    Press(String),
    Release,
    /// Move the touch point to the next or previous thing on the screen.
    NextItem,
    PreviousItem,
    /// Move the touch point a step: -1, 0 or 1 across and down.
    Step(i32, i32),
    /// Put the touch point back in the middle of the screen.
    Centre,
    /// Say what's at the touch point, and where.
    WhereIsIt,
    /// Read every property of the element at the touch point.
    Details,
    /// Turn a watch's crown: 1 up, -1 down.
    Crown(i32),
    Help,
    /// A key that does nothing in gesture mode.
    Unknown,
}

const EXTENDED: u16 = 0x100;
const TAB: u16 = 0x0F;

/// What a key named by the character it types does.
#[derive(Clone, Copy)]
enum Role {
    Tap(&'static str),
    /// Holds a touch down for as long as the key is held.
    Hold(&'static str),
    Other(fn() -> GestureAction),
}

/// Keys named by the character they type, with their scan code on a US
/// keyboard, used when no key types it.
const NAMED_KEYS: [(&str, u16, Role); 10] = [
    ("t", 0x14, Role::Tap("tap")),
    ("r", 0x13, Role::Tap("triple-tap")),
    ("h", 0x23, Role::Hold("double-tap-hold")),
    ("l", 0x26, Role::Hold("tap-hold")),
    ("c", 0x2E, Role::Other(|| GestureAction::Centre)),
    ("w", 0x11, Role::Other(|| GestureAction::WhereIsIt)),
    ("d", 0x20, Role::Other(|| GestureAction::Details)),
    ("?", 0x35, Role::Other(|| GestureAction::Help)),
    ("[", 0x1A, Role::Other(|| GestureAction::Crown(1))),
    ("]", 0x1B, Role::Other(|| GestureAction::Crown(-1))),
];

/// Where the keys named by a character are, on a keyboard layout.
#[derive(Clone)]
pub struct KeyMap {
    taps: HashMap<u16, &'static str>,
    holds: HashMap<u16, &'static str>,
    others: HashMap<u16, GestureAction>,
    /// What the help says about the crown keys, ending in a space, or nothing.
    crown_help: String,
}

impl Default for KeyMap {
    /// Every key where it is on a US keyboard.
    fn default() -> Self {
        KeyMap::new(None)
    }
}

impl KeyMap {
    /// Finds the gesture keys on a keyboard layout: `typing` says what the
    /// key with a scan code types, with Shift (true) or without.
    pub fn new(typing: Option<&dyn Fn(u16, bool) -> Option<String>>) -> KeyMap {
        let mut taken: HashSet<u16> = [0x01, 0x0E, 0x1C, 0x39, TAB].into_iter().collect();
        taken.extend((0x02..=0x05).chain([0x1D, 0x2A, 0x36, 0x38, 0x3A]));
        let mut found: [Option<u16>; NAMED_KEYS.len()] = [None; NAMED_KEYS.len()];
        // Characters typed without Shift come first, so T is the key that
        // types t, not one that types T only with Shift.
        if let Some(typing) = typing {
            for shift in [false, true] {
                for key in 0x01..0x59 {
                    if taken.contains(&key) {
                        continue;
                    }
                    let Some(typed) = typing(key, shift).map(|t| t.to_lowercase()) else {
                        continue;
                    };
                    if let Some(index) = NAMED_KEYS.iter().position(|(c, _, _)| *c == typed)
                        && found[index].is_none()
                    {
                        found[index] = Some(key);
                        taken.insert(key);
                    }
                }
            }
        }
        for (index, (_, us_key, _)) in NAMED_KEYS.iter().enumerate() {
            if found[index].is_none() && taken.insert(*us_key) {
                found[index] = Some(*us_key);
            }
        }
        let mut map = KeyMap {
            taps: HashMap::new(),
            holds: HashMap::new(),
            others: HashMap::new(),
            crown_help: String::new(),
        };
        for (index, key) in found.iter().enumerate() {
            let Some(key) = *key else { continue };
            match NAMED_KEYS[index].2 {
                Role::Tap(name) => map.taps.insert(key, name),
                Role::Hold(name) => map.holds.insert(key, name),
                Role::Other(action) => {
                    map.others.insert(key, action());
                    None
                }
            };
        }
        let crown = |steps| {
            map.others
                .iter()
                .find(|(_, action)| **action == GestureAction::Crown(steps))
                .map(|(key, _)| *key)
        };
        let (Some(up), Some(down)) = (crown(1), crown(-1)) else {
            return map;
        };
        let types = |key, wanted: &str| {
            typing.is_none_or(|t| [false, true].iter().any(|&s| t(key, s).as_deref() == Some(wanted)))
        };
        map.crown_help = if types(up, "[") {
            "On a watch, left bracket turns the crown up and right bracket turns it down. ".into()
        } else {
            // Named by what they type, such as circumflex and dollar on a
            // French keyboard, or else by where they are.
            let name = |key, us_key, position: &str| {
                let typed = typing.and_then(|t| t(key, false));
                match typed {
                    Some(typed) => key_name(&typed).unwrap_or_else(|| {
                        if key == us_key {
                            position.into()
                        } else {
                            format!("the {typed} key")
                        }
                    }),
                    None => position.into(),
                }
            };
            format!(
                "On a watch, {} turns the crown up and {} turns it down. ",
                name(up, 0x1A, "the first key after P"),
                name(down, 0x1B, "the second key after P")
            )
        };
        map
    }

    /// The gesture keys, read out by question mark.
    pub fn help(&self, return_shortcut: &str) -> String {
        format!(
            "Gesture keys. Arrows swipe. Hold one arrow and press another for a two-part swipe, such as up then left. Space double taps. T taps, R triple taps. H double taps and holds, and L touches and holds, until you let go. Hold 2, 3 or 4 while pressing a key to use that many fingers. Gestures happen at the touch point: Tab and Shift Tab move it from item to item, Shift arrows move it a step, C puts it in the middle, W says what's there and where, and D reads its properties. {}{return_shortcut} returns to Windows.",
            self.crown_help
        )
    }

    fn tap(&self, key: u16) -> Option<&'static str> {
        match key {
            0x39 | 0x1C => Some("double-tap"),               // Space, Enter
            k if k == EXTENDED | 0x1C => Some("double-tap"), // keypad Enter
            _ => self.taps.get(&key).copied(),
        }
    }

    fn hold(&self, key: u16) -> Option<&'static str> {
        self.holds.get(&key).copied()
    }

    fn other(&self, key: u16) -> Option<GestureAction> {
        self.others.get(&key).cloned()
    }
}

/// A key's name as it's said, such as "the circumflex key" or "the semicolon
/// key", in words, as screen readers may not read punctuation at their usual
/// level. None for a character with no name here, which is then named by
/// where the key is.
fn key_name(typed: &str) -> Option<String> {
    let name = match typed {
        "^" => "circumflex",
        "$" => "dollar",
        "¨" => "diaeresis",
        "*" => "asterisk",
        "+" => "plus",
        "´" => "acute accent",
        "`" => "grave accent",
        "ü" => "u umlaut",
        "è" => "e grave",
        "ç" => "c cedilla",
        "à" => "a grave",
        "é" => "e acute",
        "º" => "ordinal",
        "¡" => "inverted exclamation mark",
        "[" => "left bracket",
        "]" => "right bracket",
        ";" => "semicolon",
        ":" => "colon",
        "," => "comma",
        "." => "period",
        "/" => "slash",
        "\\" => "backslash",
        "-" => "minus",
        "=" => "equals",
        "'" => "apostrophe",
        "\"" => "quotation mark",
        "§" => "section",
        "<" => "less than",
        ">" => "greater than",
        "#" => "number sign",
        "@" => "at sign",
        "&" => "ampersand",
        "%" => "percent",
        "!" => "exclamation mark",
        "~" => "tilde",
        "|" => "vertical bar",
        "°" => "degree",
        "£" => "pound",
        "€" => "euro",
        "_" => "underscore",
        "(" => "left parenthesis",
        ")" => "right parenthesis",
        other if !other.is_empty() && other.chars().all(char::is_alphanumeric) => other,
        _ => return None,
    };
    Some(format!("the {name} key"))
}

fn arrow(key: u16) -> Option<(&'static str, i32, i32)> {
    match key {
        k if k == EXTENDED | 0x4B => Some(("left", -1, 0)),
        k if k == EXTENDED | 0x4D => Some(("right", 1, 0)),
        k if k == EXTENDED | 0x50 => Some(("down", 0, 1)),
        k if k == EXTENDED | 0x48 => Some(("up", 0, -1)),
        _ => None,
    }
}

fn finger_key(key: u16) -> Option<u32> {
    match key {
        0x02 => Some(1),
        0x03 => Some(2),
        0x04 => Some(3),
        0x05 => Some(4),
        _ => None,
    }
}

fn is_shift(key: u16) -> bool {
    matches!(key, 0x2A | 0x36)
}

fn is_modifier(key: u16) -> bool {
    is_shift(key)
        || matches!(
            key,
            0x1D | 0x38 | 0x3A // Control, Alt, Caps Lock
        )
        || [0x1D, 0x38, 0x5B, 0x5C]
            .map(|k| EXTENDED | k)
            .contains(&key)
}

#[derive(Default)]
pub struct GestureKeys {
    /// Where the letter keys are, on the keyboard layout in use.
    map: KeyMap,
    /// Keys down, as far as gesture mode knows. A key going down again while
    /// in here is the keyboard repeating it.
    down: HashSet<u16>,
    /// An arrow pressed and not yet released: a swipe that may become the
    /// first part of a two-part swipe.
    pending: Option<(u16, &'static str, u32)>,
    /// The key holding a touch down, if one is.
    holding: Option<u16>,
}

impl GestureKeys {
    /// Uses the keys of another keyboard layout.
    pub fn use_map(&mut self, map: KeyMap) {
        self.map = map;
    }

    /// Handles a key going down or up, returning what to do.
    pub fn handle(&mut self, key: u16, is_down: bool) -> Vec<GestureAction> {
        let mut actions = Vec::new();
        if !is_down {
            self.down.remove(&key);
            if Some(key) == self.holding {
                self.lift(&mut actions);
            } else if let Some((pending_key, direction, fingers)) = self.pending
                && pending_key == key
            {
                // Releasing the arrow of a swipe still waiting makes it a plain swipe.
                self.pending = None;
                actions.push(GestureAction::Gesture(format!(
                    "{}swipe-{direction}",
                    prefix(fingers)
                )));
            }
            return actions;
        }
        if !self.down.insert(key) {
            return actions;
        }
        if is_modifier(key) || finger_key(key).is_some() {
            return actions;
        }
        // Another key lifts a held touch, in case its key's release never came.
        self.lift(&mut actions);
        if let Some((name, dx, dy)) = arrow(key) {
            if self.shift() {
                self.finish_pending(&mut actions);
                actions.push(GestureAction::Step(dx, dy));
                return actions;
            }
            let fingers = self.fingers();
            if let Some((first_key, first, first_fingers)) = self.pending.take() {
                if first_fingers == 1 && fingers == 1 && first != name {
                    self.down.remove(&first_key);
                    actions.push(GestureAction::Gesture(format!("swipe-{first}-then-{name}")));
                    return actions;
                }
                // Not a two-part swipe: do the first, and wait on this one.
                actions.push(GestureAction::Gesture(format!(
                    "{}swipe-{first}",
                    prefix(first_fingers)
                )));
            }
            self.pending = Some((key, name, fingers));
            return actions;
        }
        self.finish_pending(&mut actions);
        let fingers = self.fingers();
        if let Some(tap) = self.map.tap(key) {
            actions.push(GestureAction::Gesture(format!("{}{tap}", prefix(fingers))));
        } else if let Some(hold) = self.map.hold(key) {
            self.holding = Some(key);
            actions.push(GestureAction::Press(format!("{}{hold}", prefix(fingers))));
        } else if key == TAB {
            actions.push(if self.shift() {
                GestureAction::PreviousItem
            } else {
                GestureAction::NextItem
            });
        } else if let Some(action) = self.map.other(key) {
            actions.push(action);
        } else {
            actions.push(GestureAction::Unknown);
        }
        actions
    }

    /// Forgets keys and half-drawn swipes, lifting any held touch, when
    /// gesture mode ends.
    pub fn reset(&mut self) -> Vec<GestureAction> {
        let mut actions = Vec::new();
        self.lift(&mut actions);
        self.down.clear();
        self.pending = None;
        actions
    }

    fn lift(&mut self, actions: &mut Vec<GestureAction>) {
        if self.holding.take().is_some() {
            actions.push(GestureAction::Release);
        }
    }

    /// Performs a swipe still waiting on its arrow's release, before another
    /// key does something else.
    fn finish_pending(&mut self, actions: &mut Vec<GestureAction>) {
        if let Some((_, direction, fingers)) = self.pending.take() {
            actions.push(GestureAction::Gesture(format!(
                "{}swipe-{direction}",
                prefix(fingers)
            )));
        }
    }

    fn shift(&self) -> bool {
        self.down.iter().any(|&k| is_shift(k))
    }

    /// The number of fingers: the largest of 2, 3 or 4 held down, otherwise one.
    fn fingers(&self) -> u32 {
        self.down
            .iter()
            .filter_map(|&k| finger_key(k))
            .max()
            .unwrap_or(1)
    }
}

fn prefix(fingers: u32) -> String {
    match fingers {
        2 => "two-finger-".into(),
        3 => "three-finger-".into(),
        4 => "four-finger-".into(),
        _ => String::new(),
    }
}
