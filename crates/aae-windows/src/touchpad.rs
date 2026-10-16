//! The touchpad as the device's screen. A Windows Precision Touchpad reports
//! every finger's position on the pad, and each one goes to Android as it
//! moves, so Android's touch exploration sees a finger on a real screen and
//! the screen reader recognises its own gestures: explore by touch, swipes,
//! angled swipes, multi-finger gestures and swipes in from an edge.
//!
//! The whole pad is the whole screen, stretched to fit: its top edge, nearest
//! the keyboard, is the top of the screen, where the status bar is, and its
//! bottom edge is the bottom of the screen, where the navigation bar or
//! gesture area is.
//!
//! This part reads touchpad reports into fingers; `touchpad_input` gets the
//! reports from Windows.

use std::collections::HashMap;

/// HID usages a touchpad reports, as (usage page, usage).
pub const X: (u16, u16) = (0x01, 0x30);
pub const Y: (u16, u16) = (0x01, 0x31);
pub const TIP_SWITCH: (u16, u16) = (0x0D, 0x42);
pub const CONFIDENCE: (u16, u16) = (0x0D, 0x47);
pub const CONTACT_ID: (u16, u16) = (0x0D, 0x51);
pub const CONTACT_COUNT: (u16, u16) = (0x0D, 0x54);
pub const SCAN_TIME: (u16, u16) = (0x0D, 0x56);

/// The values in one report: (link collection, usage page, usage) to value.
/// Buttons, such as the tip switch, are in it only while they're on.
pub type Values = HashMap<(u16, u16, u16), u32>;

/// How far a finger moves on the pad, in millimetres, before it's taken as
/// moving, unless Settings say otherwise. A finger set down to tap wobbles
/// a little, which stretched over a bigger screen is more than the screen
/// reader allows a tap, so it explores instead.
pub const SETTLE_MM: u8 = 2;

/// A touchpad's extent, in its own units.
#[derive(Debug, Clone, Copy)]
pub struct Pad {
    pub x_min: i32,
    pub x_max: i32,
    pub y_min: i32,
    pub y_max: i32,
    /// Its units per millimetre, across and down.
    pub per_mm: (f64, f64),
    /// False if it doesn't say which touches are palms.
    pub reports_confidence: bool,
}

impl Pad {
    /// Where a point on the pad is, as fractions across and down.
    fn fraction(&self, x: i32, y: i32) -> (f64, f64) {
        let f = |v: i32, min: i32, max: i32| {
            ((v - min) as f64 / (max - min).max(1) as f64).clamp(0.0, 1.0)
        };
        (f(x, self.x_min, self.x_max), f(y, self.y_min, self.y_max))
    }

    /// Its width and height in millimetres.
    pub fn size_mm(&self) -> (f64, f64) {
        (
            (self.x_max - self.x_min).max(1) as f64 / self.per_mm.0,
            (self.y_max - self.y_min).max(1) as f64 / self.per_mm.1,
        )
    }
}

/// A pad's units per millimetre along one side, from its HID physical
/// extent: `units` is the HID unit code and `exponent` its power of ten.
/// When the pad doesn't say, or says something unlikely, it's taken as 10
/// centimetres across.
pub fn per_mm(logical: (i32, i32), physical: (i32, i32), units: u32, exponent: u32) -> f64 {
    let logical_range = (logical.1 - logical.0).max(1) as f64;
    let fallback = logical_range / 100.0;
    let mm_per_unit = match units {
        0x11 => 10.0, // centimetres
        0x13 => 25.4, // inches
        _ => return fallback,
    };
    // A 4-bit signed exponent.
    let exponent = ((exponent & 0xF) as i32 ^ 0x8) - 0x8;
    let mm = (physical.1 - physical.0) as f64 * 10f64.powi(exponent) * mm_per_unit;
    // Touchpads are a few centimetres to a few tens of centimetres across.
    if !(20.0..=400.0).contains(&mm) {
        return fallback;
    }
    logical_range / mm
}

/// A finger on the pad, as one report gives it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Contact {
    pub id: u32,
    pub x: i32,
    pub y: i32,
    /// On the pad, and not a palm.
    pub touching: bool,
}

/// Puts together a frame, every finger at one moment, from reports. Some
/// touchpads send a frame in several reports, the first saying how many
/// fingers there are and the rest saying none.
#[derive(Default)]
pub struct Assembler {
    scan_time: Option<u32>,
    expected: usize,
    contacts: Vec<Contact>,
}

impl Assembler {
    /// Adds a report, given its values and the link collections that hold
    /// a finger each. Returns the frame once it's complete.
    pub fn report(&mut self, links: &[u16], values: &Values, pad: &Pad) -> Option<Vec<Contact>> {
        let top = |usage: (u16, u16)| values.get(&(0, usage.0, usage.1)).copied();
        let Some(scan_time) = top(SCAN_TIME) else {
            self.clear();
            return None;
        };
        // A report from another moment: the frame it continued was lost.
        if !self.contacts.is_empty() && self.scan_time != Some(scan_time) {
            self.clear();
        }
        self.scan_time = Some(scan_time);
        if let Some(count) = top(CONTACT_COUNT).filter(|&c| c > 0) {
            self.expected = count as usize;
        }
        // Saying no fingers, it continues a frame; with no frame begun, the
        // report that began it was lost, and what it holds is only part.
        if self.expected == 0 {
            self.clear();
            return None;
        }
        for &link in links {
            let value = |usage: (u16, u16)| values.get(&(link, usage.0, usage.1)).copied();
            let on = |usage: (u16, u16)| values.contains_key(&(link, usage.0, usage.1));
            let (Some(id), Some(x), Some(y)) = (value(CONTACT_ID), value(X), value(Y)) else {
                continue;
            };
            self.contacts.push(Contact {
                id,
                x: x as i32,
                y: y as i32,
                touching: on(TIP_SWITCH) && (on(CONFIDENCE) || !pad.reports_confidence),
            });
            if self.contacts.len() >= self.expected {
                // Link collections past the count hold nothing current.
                let frame = std::mem::take(&mut self.contacts);
                self.clear();
                return Some(frame);
            }
        }
        None
    }

    pub fn clear(&mut self) {
        self.scan_time = None;
        self.expected = 0;
        self.contacts.clear();
    }
}

/// A finger on the screen: which, where as fractions of the screen as the
/// user sees it, and whether it's down. See `aae_ffi::PadTouch`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Touch {
    pub id: i32,
    pub x: f64,
    pub y: f64,
    pub down: bool,
}

struct Finger {
    /// Its id on the screen, the lowest free, as Android expects.
    slot: i32,
    /// Where it was set down, on the pad.
    start: (i32, i32),
    /// True once it has moved more than `Fingers::settle_mm` from there.
    moving: bool,
    /// Where it is on the screen, as last sent.
    at: (f64, f64),
}

/// The fingers on the pad, turned into touches on the screen.
pub struct Fingers {
    fingers: HashMap<u32, Finger>,
    /// How far a finger moves before it's taken as moving.
    settle_mm: f64,
}

impl Default for Fingers {
    fn default() -> Self {
        Fingers::new(SETTLE_MM)
    }
}

impl Fingers {
    pub fn new(settle_mm: u8) -> Fingers {
        Fingers {
            fingers: HashMap::new(),
            settle_mm: settle_mm as f64,
        }
    }

    /// Takes a frame and returns the touches to send: every finger down,
    /// and those lifting, or nothing if nothing changed.
    pub fn frame(&mut self, pad: &Pad, contacts: &[Contact]) -> Vec<Touch> {
        let mut changed = false;
        let mut lifted = Vec::new();
        // Fingers no longer touching, or no longer reported, have lifted.
        let touching: Vec<&Contact> = contacts.iter().filter(|c| c.touching).collect();
        self.fingers.retain(|id, finger| {
            if touching.iter().any(|c| c.id == *id) {
                return true;
            }
            lifted.push(Touch {
                id: finger.slot,
                x: finger.at.0,
                y: finger.at.1,
                down: false,
            });
            false
        });
        changed |= !lifted.is_empty();
        for contact in touching {
            match self.fingers.get_mut(&contact.id) {
                Some(finger) => {
                    if !finger.moving {
                        let dx = (contact.x - finger.start.0) as f64 / pad.per_mm.0;
                        let dy = (contact.y - finger.start.1) as f64 / pad.per_mm.1;
                        if dx.hypot(dy) < self.settle_mm {
                            continue;
                        }
                        finger.moving = true;
                    }
                    let at = pad.fraction(contact.x, contact.y);
                    if at != finger.at {
                        finger.at = at;
                        changed = true;
                    }
                }
                None => {
                    // Not one lifting now, as that would lift this finger too.
                    let slot = (0..)
                        .find(|s| {
                            self.fingers.values().all(|f| f.slot != *s)
                                && lifted.iter().all(|t| t.id != *s)
                        })
                        .unwrap_or(0);
                    self.fingers.insert(
                        contact.id,
                        Finger {
                            slot,
                            start: (contact.x, contact.y),
                            moving: false,
                            at: pad.fraction(contact.x, contact.y),
                        },
                    );
                    changed = true;
                }
            }
        }
        if !changed {
            return Vec::new();
        }
        let mut touches: Vec<Touch> = self.down_touches();
        touches.extend(lifted);
        touches
    }

    /// Lifts every finger, returning the touches that do it.
    pub fn lift_all(&mut self) -> Vec<Touch> {
        let touches = self
            .down_touches()
            .into_iter()
            .map(|t| Touch { down: false, ..t })
            .collect();
        self.fingers.clear();
        touches
    }

    pub fn any_down(&self) -> bool {
        !self.fingers.is_empty()
    }

    fn down_touches(&self) -> Vec<Touch> {
        let mut touches: Vec<Touch> = self
            .fingers
            .values()
            .map(|f| Touch {
                id: f.slot,
                x: f.at.0,
                y: f.at.1,
                down: true,
            })
            .collect();
        touches.sort_by_key(|t| t.id);
        touches
    }
}

/// How long a touch is held back while it's told what it is: a tap, a
/// swipe, or a finger exploring.
pub const DECIDE_MS: u64 = 200;
/// How far a finger goes in that time to be swiping, in millimetres on the pad.
const SWIPE_MM: f64 = 5.0;
/// The longest a swipe is held back; still down after that, the finger is
/// exploring.
const SWIPE_MAX_MS: u64 = 1000;
/// A touch this soon after a tap is the next of several, or a tap and hold,
/// so it goes at once to keep in time with the one before.
const NEXT_TAP_MS: u64 = 300;

/// What to send the screen.
#[derive(Debug, Clone, PartialEq)]
pub enum Out {
    /// Touches to send as they are.
    Live(Vec<Touch>),
    /// A tap or swipe to play cleanly: each finger's path, sampled together.
    Gesture(Vec<Vec<(f64, f64)>>),
}

enum Phase {
    Idle,
    /// Holding a touch back, with what the fingers have done since `start`.
    Waiting { start: u64, frames: Vec<Vec<Touch>> },
    /// Sending touches as they come.
    Live { start: u64 },
}

/// Tells taps and swipes, which are played cleanly once they end, from a
/// finger exploring, which goes live. Sent as they come, a quick tap or
/// swipe reaches Android with its timing stretched, and Android takes it
/// for a finger exploring.
pub struct Recogniser {
    phase: Phase,
    /// When the last tap ended.
    last_tap: Option<u64>,
    size_mm: (f64, f64),
}

impl Default for Recogniser {
    fn default() -> Self {
        Recogniser {
            phase: Phase::Idle,
            last_tap: None,
            size_mm: (100.0, 60.0),
        }
    }
}

impl Recogniser {
    /// True while a touch is held back: call `tick` now and then meanwhile.
    pub fn waiting(&self) -> bool {
        matches!(self.phase, Phase::Waiting { .. })
    }

    /// Takes touches from `Fingers::frame`, at `now` in milliseconds.
    pub fn touches(&mut self, now: u64, pad: &Pad, touches: Vec<Touch>) -> Vec<Out> {
        self.size_mm = pad.size_mm();
        if touches.is_empty() {
            return self.tick(now);
        }
        let all_up = touches.iter().all(|t| !t.down);
        match &mut self.phase {
            Phase::Idle => {
                if all_up {
                    return Vec::new();
                }
                if self
                    .last_tap
                    .is_some_and(|at| now.saturating_sub(at) < NEXT_TAP_MS)
                {
                    self.phase = Phase::Live { start: now };
                    return vec![Out::Live(touches)];
                }
                self.phase = Phase::Waiting {
                    start: now,
                    frames: vec![touches],
                };
                Vec::new()
            }
            Phase::Waiting { frames, .. } => {
                frames.push(touches);
                if !all_up {
                    return self.tick(now);
                }
                let frames = std::mem::take(frames);
                self.phase = Phase::Idle;
                let tracks = tracks(&frames);
                let moved = tracks.iter().any(|t| t.len() > 1);
                self.last_tap = (!moved).then_some(now);
                vec![Out::Gesture(tracks)]
            }
            Phase::Live { start } => {
                if all_up {
                    let quick = now.saturating_sub(*start) < DECIDE_MS;
                    self.last_tap = quick.then_some(now);
                    self.phase = Phase::Idle;
                }
                vec![Out::Live(touches)]
            }
        }
    }

    /// Decides about a touch held back long enough.
    pub fn tick(&mut self, now: u64) -> Vec<Out> {
        let Phase::Waiting { start, frames } = &self.phase else {
            return Vec::new();
        };
        let start = *start;
        let held = now.saturating_sub(start);
        if held < DECIDE_MS {
            return Vec::new();
        }
        let (w, h) = self.size_mm;
        let tracks = tracks(frames);
        let furthest = tracks
            .iter()
            .flat_map(|t| t.iter().map(move |p| (p.0 - t[0].0, p.1 - t[0].1)))
            .map(|(dx, dy)| (dx * w).hypot(dy * h))
            .fold(0.0, f64::max);
        if furthest >= SWIPE_MM && held < SWIPE_MAX_MS {
            return Vec::new();
        }
        // Exploring: the fingers still down go down where they are now.
        let mut down: Vec<Touch> = Vec::new();
        for touch in frames.iter().flatten() {
            down.retain(|t| t.id != touch.id);
            if touch.down {
                down.push(*touch);
            }
        }
        down.sort_by_key(|t| t.id);
        self.phase = Phase::Live { start };
        if down.is_empty() {
            return Vec::new();
        }
        vec![Out::Live(down)]
    }

    /// Forgets the touch: Android gets `lifts`, from `Fingers::lift_all`,
    /// only if it had the fingers.
    pub fn cancel(&mut self, lifts: Vec<Touch>) -> Vec<Out> {
        let live = matches!(self.phase, Phase::Live { .. });
        self.phase = Phase::Idle;
        self.last_tap = None;
        if live && !lifts.is_empty() {
            vec![Out::Live(lifts)]
        } else {
            Vec::new()
        }
    }
}

/// Each finger's path through frames of touches, sampled together, without
/// samples where nothing moved: a finger that stays put has one point.
fn tracks(frames: &[Vec<Touch>]) -> Vec<Vec<(f64, f64)>> {
    let mut ids: Vec<i32> = frames.iter().flatten().map(|t| t.id).collect();
    ids.sort_unstable();
    ids.dedup();
    // Each finger where it first was, until it's reported.
    let first = |id: i32| {
        frames
            .iter()
            .flatten()
            .find(|t| t.id == id)
            .map_or((0.0, 0.0), |t| (t.x, t.y))
    };
    let mut at: Vec<(f64, f64)> = ids.iter().map(|&id| first(id)).collect();
    let mut rows: Vec<Vec<(f64, f64)>> = vec![at.clone()];
    for frame in frames {
        for touch in frame {
            if let Some(i) = ids.iter().position(|&id| id == touch.id) {
                at[i] = (touch.x, touch.y);
            }
        }
        if rows.last() != Some(&at) {
            rows.push(at.clone());
        }
    }
    let moved = rows.len() > 1;
    (0..ids.len())
        .map(|i| {
            if moved {
                rows.iter().map(|r| r[i]).collect()
            } else {
                vec![rows[0][i]]
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pad 100 mm across and 60 mm down, at 10 units a millimetre.
    fn pad() -> Pad {
        Pad {
            x_min: 0,
            x_max: 1000,
            y_min: 0,
            y_max: 600,
            per_mm: (10.0, 10.0),
            reports_confidence: true,
        }
    }

    fn contact(id: u32, x: i32, y: i32) -> Contact {
        Contact {
            id,
            x,
            y,
            touching: true,
        }
    }

    fn report(scan: u32, count: u32, fingers: &[(u16, u32, i32, i32)]) -> Values {
        let mut values = Values::new();
        values.insert((0, SCAN_TIME.0, SCAN_TIME.1), scan);
        values.insert((0, CONTACT_COUNT.0, CONTACT_COUNT.1), count);
        for &(link, id, x, y) in fingers {
            values.insert((link, CONTACT_ID.0, CONTACT_ID.1), id);
            values.insert((link, X.0, X.1), x as u32);
            values.insert((link, Y.0, Y.1), y as u32);
            values.insert((link, TIP_SWITCH.0, TIP_SWITCH.1), 1);
            values.insert((link, CONFIDENCE.0, CONFIDENCE.1), 1);
        }
        values
    }

    #[test]
    fn the_pads_top_is_the_screens_top() {
        let mut fingers = Fingers::default();
        // Near the keyboard, in the middle: the status bar.
        let touches = fingers.frame(&pad(), &[contact(7, 500, 0)]);
        assert_eq!(
            touches,
            vec![Touch {
                id: 0,
                x: 0.5,
                y: 0.0,
                down: true
            }]
        );
        // The bottom right corner, and beyond it, is the screen's.
        let mut fingers = Fingers::default();
        let touches = fingers.frame(&pad(), &[contact(1, 1200, 600)]);
        assert_eq!((touches[0].x, touches[0].y), (1.0, 1.0));
    }

    #[test]
    fn a_finger_wobbling_in_place_stays_put() {
        let mut fingers = Fingers::default();
        fingers.frame(&pad(), &[contact(1, 500, 300)]);
        // Half a millimetre: nothing to send.
        assert!(fingers.frame(&pad(), &[contact(1, 505, 303)]).is_empty());
        // Lifting there lifts it where it went down.
        let lift = fingers.frame(&pad(), &[]);
        assert_eq!(
            lift,
            vec![Touch {
                id: 0,
                x: 0.5,
                y: 0.5,
                down: false
            }]
        );
    }

    #[test]
    fn steadiness_is_chosen() {
        // 1.5 millimetres: still a tap at 2, moving at 1.
        let mut steady = Fingers::new(2);
        steady.frame(&pad(), &[contact(1, 500, 300)]);
        assert!(steady.frame(&pad(), &[contact(1, 515, 300)]).is_empty());
        let mut eager = Fingers::new(1);
        eager.frame(&pad(), &[contact(1, 500, 300)]);
        assert_eq!(eager.frame(&pad(), &[contact(1, 515, 300)]).len(), 1);
    }

    #[test]
    fn a_moving_finger_follows_the_pad() {
        let mut fingers = Fingers::default();
        fingers.frame(&pad(), &[contact(1, 500, 300)]);
        let moved = fingers.frame(&pad(), &[contact(1, 600, 300)]);
        assert_eq!((moved[0].x, moved[0].y, moved[0].down), (0.6, 0.5, true));
        // Once moving, small steps count too.
        let moved = fingers.frame(&pad(), &[contact(1, 602, 300)]);
        assert_eq!(moved[0].x, 0.602);
    }

    #[test]
    fn fingers_keep_their_ids_and_reuse_free_ones() {
        let mut fingers = Fingers::default();
        fingers.frame(&pad(), &[contact(40, 100, 100)]);
        let two = fingers.frame(&pad(), &[contact(40, 100, 100), contact(41, 300, 100)]);
        assert_eq!(two.iter().map(|t| t.id).collect::<Vec<_>>(), [0, 1]);
        // The first lifts: every finger down goes, with the lift.
        let one = fingers.frame(&pad(), &[contact(41, 300, 100)]);
        assert_eq!(
            one.iter().map(|t| (t.id, t.down)).collect::<Vec<_>>(),
            [(1, true), (0, false)]
        );
        // A new finger takes the free id.
        let again = fingers.frame(&pad(), &[contact(41, 300, 100), contact(42, 500, 100)]);
        assert_eq!(again.iter().map(|t| t.id).collect::<Vec<_>>(), [0, 1]);
    }

    #[test]
    fn a_finger_landing_as_another_lifts_takes_another_id() {
        let mut fingers = Fingers::default();
        fingers.frame(&pad(), &[contact(40, 100, 100)]);
        let rolled = fingers.frame(&pad(), &[contact(41, 300, 100)]);
        assert_eq!(
            rolled.iter().map(|t| (t.id, t.down)).collect::<Vec<_>>(),
            [(1, true), (0, false)]
        );
    }

    #[test]
    fn a_palm_or_a_lifted_tip_lifts_the_finger() {
        let mut fingers = Fingers::default();
        fingers.frame(&pad(), &[contact(1, 500, 300)]);
        let palm = Contact {
            touching: false,
            ..contact(1, 500, 300)
        };
        let lift = fingers.frame(&pad(), &[palm]);
        assert_eq!(lift.len(), 1);
        assert!(!lift[0].down);
        assert!(!fingers.any_down());
    }

    #[test]
    fn lift_all_lifts_where_fingers_are() {
        let mut fingers = Fingers::default();
        fingers.frame(&pad(), &[contact(1, 0, 0), contact(2, 1000, 600)]);
        let lift = fingers.lift_all();
        assert_eq!(lift.len(), 2);
        assert!(lift.iter().all(|t| !t.down));
        assert!(fingers.lift_all().is_empty());
    }

    #[test]
    fn frames_in_one_report() {
        let mut assembler = Assembler::default();
        // Two link collections, one finger reported.
        let frame = assembler
            .report(&[1, 2], &report(10, 1, &[(1, 5, 100, 200)]), &pad())
            .unwrap();
        assert_eq!(frame, vec![contact(5, 100, 200)]);
    }

    #[test]
    fn frames_across_reports() {
        let mut assembler = Assembler::default();
        assert!(
            assembler
                .report(&[1], &report(10, 2, &[(1, 5, 100, 200)]), &pad())
                .is_none()
        );
        // The rest come with a count of none, at the same scan time.
        let frame = assembler
            .report(&[1], &report(10, 0, &[(1, 6, 300, 400)]), &pad())
            .unwrap();
        assert_eq!(frame, vec![contact(5, 100, 200), contact(6, 300, 400)]);
    }

    #[test]
    fn a_frame_missing_its_end_is_dropped() {
        let mut assembler = Assembler::default();
        assembler.report(&[1], &report(10, 2, &[(1, 5, 100, 200)]), &pad());
        // A new moment starts before the frame ended.
        let frame = assembler
            .report(&[1], &report(11, 1, &[(1, 5, 110, 200)]), &pad())
            .unwrap();
        assert_eq!(frame, vec![contact(5, 110, 200)]);
    }

    #[test]
    fn the_rest_of_a_frame_whose_start_was_lost_is_dropped() {
        let mut assembler = Assembler::default();
        assert!(
            assembler
                .report(&[1], &report(10, 0, &[(1, 6, 300, 400)]), &pad())
                .is_none()
        );
        // The next frame is whole.
        let frame = assembler
            .report(&[1], &report(11, 1, &[(1, 6, 300, 400)]), &pad())
            .unwrap();
        assert_eq!(frame, vec![contact(6, 300, 400)]);
    }

    #[test]
    fn a_finger_without_its_tip_switch_is_lifting() {
        let mut values = report(10, 1, &[(1, 5, 100, 200)]);
        values.remove(&(1, TIP_SWITCH.0, TIP_SWITCH.1));
        let frame = Assembler::default().report(&[1], &values, &pad()).unwrap();
        assert!(!frame[0].touching);
    }

    fn down(id: i32, x: f64, y: f64) -> Touch {
        Touch { id, x, y, down: true }
    }

    fn up(id: i32, x: f64, y: f64) -> Touch {
        Touch { id, x, y, down: false }
    }

    #[test]
    fn a_quick_tap_is_held_back_then_played() {
        let mut r = Recogniser::default();
        assert!(r.touches(0, &pad(), vec![down(0, 0.5, 0.5)]).is_empty());
        assert!(r.waiting());
        assert!(r.tick(50).is_empty());
        let out = r.touches(90, &pad(), vec![up(0, 0.5, 0.5)]);
        assert_eq!(out, vec![Out::Gesture(vec![vec![(0.5, 0.5)]])]);
        assert!(!r.waiting());
    }

    #[test]
    fn a_quick_swipe_is_played_whole() {
        let mut r = Recogniser::default();
        r.touches(0, &pad(), vec![down(0, 0.2, 0.5)]);
        r.touches(40, &pad(), vec![down(0, 0.3, 0.5)]);
        r.touches(80, &pad(), vec![down(0, 0.4, 0.5)]);
        let out = r.touches(120, &pad(), vec![up(0, 0.4, 0.5)]);
        assert_eq!(
            out,
            vec![Out::Gesture(vec![vec![(0.2, 0.5), (0.3, 0.5), (0.4, 0.5)]])]
        );
    }

    #[test]
    fn a_resting_finger_explores_live() {
        let mut r = Recogniser::default();
        r.touches(0, &pad(), vec![down(0, 0.5, 0.5)]);
        // Still down when it's time to decide: it goes down where it is.
        assert_eq!(
            r.tick(DECIDE_MS),
            vec![Out::Live(vec![down(0, 0.5, 0.5)])]
        );
        assert!(!r.waiting());
        assert_eq!(
            r.touches(300, &pad(), vec![down(0, 0.6, 0.5)]),
            vec![Out::Live(vec![down(0, 0.6, 0.5)])]
        );
        // A slow move, three millimetres in the time, explores too.
        let mut r = Recogniser::default();
        r.touches(0, &pad(), vec![down(0, 0.5, 0.5)]);
        r.touches(150, &pad(), vec![down(0, 0.53, 0.5)]);
        assert_eq!(
            r.tick(DECIDE_MS),
            vec![Out::Live(vec![down(0, 0.53, 0.5)])]
        );
    }

    #[test]
    fn a_long_swipe_waits_to_end() {
        let mut r = Recogniser::default();
        r.touches(0, &pad(), vec![down(0, 0.1, 0.5)]);
        r.touches(150, &pad(), vec![down(0, 0.3, 0.5)]);
        // Twenty millimetres gone: still swiping.
        assert!(r.tick(DECIDE_MS).is_empty());
        assert!(r.touches(400, &pad(), vec![down(0, 0.5, 0.8)]).is_empty());
        let out = r.touches(450, &pad(), vec![up(0, 0.5, 0.8)]);
        assert!(matches!(&out[..], [Out::Gesture(t)] if t[0].len() == 3));
    }

    #[test]
    fn a_tap_soon_after_a_tap_goes_at_once() {
        let mut r = Recogniser::default();
        r.touches(0, &pad(), vec![down(0, 0.5, 0.5)]);
        r.touches(60, &pad(), vec![up(0, 0.5, 0.5)]);
        // The second tap of a double tap, and its hold, keep their timing.
        assert_eq!(
            r.touches(160, &pad(), vec![down(0, 0.5, 0.5)]),
            vec![Out::Live(vec![down(0, 0.5, 0.5)])]
        );
        assert_eq!(
            r.touches(1500, &pad(), vec![up(0, 0.5, 0.5)]),
            vec![Out::Live(vec![up(0, 0.5, 0.5)])]
        );
        // Long after, a touch is held back again.
        assert!(r.touches(3000, &pad(), vec![down(0, 0.5, 0.5)]).is_empty());
    }

    #[test]
    fn fingers_of_a_gesture_share_samples() {
        let mut r = Recogniser::default();
        r.touches(0, &pad(), vec![down(0, 0.4, 0.2)]);
        r.touches(20, &pad(), vec![down(0, 0.4, 0.2), down(1, 0.6, 0.2)]);
        r.touches(60, &pad(), vec![down(0, 0.4, 0.5), down(1, 0.6, 0.5)]);
        r.touches(90, &pad(), vec![down(1, 0.6, 0.5), up(0, 0.4, 0.5)]);
        let out = r.touches(100, &pad(), vec![up(1, 0.6, 0.5)]);
        assert_eq!(
            out,
            vec![Out::Gesture(vec![
                vec![(0.4, 0.2), (0.4, 0.5)],
                vec![(0.6, 0.2), (0.6, 0.5)],
            ])]
        );
    }

    #[test]
    fn cancelling_lifts_only_what_android_has() {
        let mut r = Recogniser::default();
        r.touches(0, &pad(), vec![down(0, 0.5, 0.5)]);
        assert!(r.cancel(vec![up(0, 0.5, 0.5)]).is_empty());
        r.touches(1000, &pad(), vec![down(0, 0.5, 0.5)]);
        r.tick(1000 + DECIDE_MS);
        assert_eq!(
            r.cancel(vec![up(0, 0.5, 0.5)]),
            vec![Out::Live(vec![up(0, 0.5, 0.5)])]
        );
    }

    #[test]
    fn pad_sizes_from_physical_units() {
        // 0 to 1000 logical over 10.5 cm (1050 × 10^-2): 9.52 a millimetre.
        let cm = per_mm((0, 1000), (0, 1050), 0x11, 0xE);
        assert!((cm - 1000.0 / 105.0).abs() < 1e-9);
        // 4 inches (400 × 10^-2).
        let inch = per_mm((0, 1016), (0, 400), 0x13, 0xE);
        assert!((inch - 10.0).abs() < 1e-9);
        // Unknown units: 10 cm across.
        assert_eq!(per_mm((0, 1000), (0, 0), 0, 0), 10.0);
        // An unlikely size, a centimetre across: the same.
        assert_eq!(per_mm((0, 1000), (0, 1), 0x11, 0), 10.0);
    }
}
