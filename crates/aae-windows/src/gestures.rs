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
//! Keys are scan codes, with 0x100 added for extended keys, so they're the
//! same keys whatever the keyboard layout.

use std::collections::HashSet;

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
    /// Say what's at the touch point.
    WhereIsIt,
    Help,
    /// A key that does nothing in gesture mode.
    Unknown,
}

pub const HELP: &str = "Gesture keys. Arrows swipe. Hold one arrow and press another for a two-part swipe, such as up then left. Space double taps. T taps, R triple taps. H double taps and holds, and L touches and holds, until you let go. Hold 2, 3 or 4 while pressing a key to use that many fingers. Gestures happen at the touch point: Tab and Shift Tab move it from item to item, Shift arrows move it a step, C puts it in the middle, and W says where it is. Control Windows Escape returns to Windows.";

const EXTENDED: u16 = 0x100;
const TAB: u16 = 0x0F;

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

fn tap(key: u16) -> Option<&'static str> {
    match key {
        0x39 | 0x1C | 0x20 => Some("double-tap"), // Space, Enter, D
        k if k == EXTENDED | 0x1C => Some("double-tap"), // keypad Enter
        0x14 => Some("tap"),                      // T
        0x13 => Some("triple-tap"),               // R
        _ => None,
    }
}

/// Keys that hold a touch down for as long as they're held.
fn hold(key: u16) -> Option<&'static str> {
    match key {
        0x23 => Some("double-tap-hold"), // H
        0x26 => Some("tap-hold"),        // L
        _ => None,
    }
}

fn other(key: u16) -> Option<GestureAction> {
    match key {
        0x2E => Some(GestureAction::Centre),    // C
        0x11 => Some(GestureAction::WhereIsIt), // W
        0x35 => Some(GestureAction::Help),      // slash, for question mark
        _ => None,
    }
}

#[derive(Default)]
pub struct GestureKeys {
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
        if let Some(tap) = tap(key) {
            actions.push(GestureAction::Gesture(format!("{}{tap}", prefix(fingers))));
        } else if let Some(hold) = hold(key) {
            self.holding = Some(key);
            actions.push(GestureAction::Press(format!("{}{hold}", prefix(fingers))));
        } else if key == TAB {
            actions.push(if self.shift() {
                GestureAction::PreviousItem
            } else {
                GestureAction::NextItem
            });
        } else if let Some(action) = other(key) {
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
