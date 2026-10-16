//! Screen reader gestures, performed with simulated fingers.
//!
//! The touches go in through the emulator's touchscreen, so Android's touch
//! exploration sees them as it would a real finger and hands them to the
//! screen reader as gestures. Sizes are in centimetres and timings follow
//! what Android's gesture detectors expect: a swipe covers its first
//! centimetre well within 150 ms and never pauses, taps are quick, and a
//! double tap's taps come within 300 ms of each other.

use std::fmt;
use std::time::Duration;

use crate::adb::Adb;
use crate::control::{Controller, Orientation, TouchPoint};
use crate::error::{Error, Result};

/// A direction on the screen, as the user sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

impl Direction {
    pub const ALL: [Direction; 4] = [
        Direction::Up,
        Direction::Down,
        Direction::Left,
        Direction::Right,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Direction::Up => "up",
            Direction::Down => "down",
            Direction::Left => "left",
            Direction::Right => "right",
        }
    }

    fn parse(text: &str) -> Option<Direction> {
        Direction::ALL.into_iter().find(|d| d.name() == text)
    }

    /// One step in this direction, in screen coordinates (y grows downwards).
    fn vector(self) -> (f64, f64) {
        match self {
            Direction::Up => (0.0, -1.0),
            Direction::Down => (0.0, 1.0),
            Direction::Left => (-1.0, 0.0),
            Direction::Right => (1.0, 0.0),
        }
    }
}

/// A screen reader gesture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Gesture {
    /// A swipe in one direction, or two at an angle, such as up then right.
    Swipe { fingers: u8, path: Vec<Direction> },
    /// One, two or three taps; with `hold`, the last touch stays down, as in
    /// double tap and hold.
    Tap { fingers: u8, taps: u8, hold: bool },
}

impl Gesture {
    pub fn swipe(path: &[Direction]) -> Gesture {
        Gesture::Swipe {
            fingers: 1,
            path: path.to_vec(),
        }
    }

    /// Reads a gesture's name, such as "swipe-right", "swipe-up-then-left",
    /// "two-finger-swipe-down", "double-tap", "double-tap-hold" or
    /// "three-finger-tap". Spaces work in place of hyphens.
    pub fn parse(name: &str) -> Option<Gesture> {
        let name = name.trim().to_lowercase().replace([' ', '_'], "-");
        let mut words: Vec<&str> = name.split('-').filter(|w| !w.is_empty()).collect();
        let mut fingers = 1;
        if let [count, "finger" | "fingers", ..] = words[..] {
            fingers = count_word(count)?;
            words.drain(..2);
        }
        if !(1..=4).contains(&fingers) {
            return None;
        }
        match words[..] {
            ["swipe", ref rest @ ..] => {
                let path: Vec<Direction> = match rest {
                    [a] => vec![Direction::parse(a)?],
                    [a, "then", b] | [a, b] => vec![Direction::parse(a)?, Direction::parse(b)?],
                    _ => return None,
                };
                // Two-part swipes are one-finger gestures.
                if path.len() == 2 && fingers != 1 {
                    return None;
                }
                Some(Gesture::Swipe { fingers, path })
            }
            _ => {
                let (taps, rest) = match words[..] {
                    ["single", ref rest @ ..] => (1, rest),
                    ["double", ref rest @ ..] => (2, rest),
                    ["triple", ref rest @ ..] => (3, rest),
                    ref rest => (1, rest),
                };
                let hold = match rest {
                    ["tap"] => false,
                    ["tap", "hold"] | ["tap", "and", "hold"] => true,
                    _ => return None,
                };
                Some(Gesture::Tap {
                    fingers,
                    taps,
                    hold,
                })
            }
        }
    }

    /// Every gesture AAE can perform, for listing.
    pub fn all() -> Vec<Gesture> {
        let mut all = Vec::new();
        for d in Direction::ALL {
            all.push(Gesture::swipe(&[d]));
        }
        for a in Direction::ALL {
            for b in Direction::ALL {
                if a != b {
                    all.push(Gesture::swipe(&[a, b]));
                }
            }
        }
        for fingers in 2..=4 {
            for d in Direction::ALL {
                all.push(Gesture::Swipe {
                    fingers,
                    path: vec![d],
                });
            }
        }
        for fingers in 1..=4 {
            for taps in 1..=3 {
                all.push(Gesture::Tap {
                    fingers,
                    taps,
                    hold: false,
                });
            }
            for taps in 1..=2 {
                all.push(Gesture::Tap {
                    fingers,
                    taps,
                    hold: true,
                });
            }
        }
        all
    }

    fn fingers(&self) -> u8 {
        match self {
            Gesture::Swipe { fingers, .. } | Gesture::Tap { fingers, .. } => *fingers,
        }
    }
}

impl fmt::Display for Gesture {
    /// The gesture's name, such as "swipe up then right" or "two-finger double tap".
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let fingers = self.fingers();
        if fingers > 1 {
            write!(
                f,
                "{}-finger ",
                ["", "one", "two", "three", "four"][fingers as usize]
            )?;
        }
        match self {
            Gesture::Swipe { path, .. } => {
                let names: Vec<&str> = path.iter().map(|d| d.name()).collect();
                write!(f, "swipe {}", names.join(" then "))
            }
            Gesture::Tap { taps, hold, .. } => {
                let count = match taps {
                    1 => "",
                    2 => "double ",
                    _ => "triple ",
                };
                write!(f, "{count}tap{}", if *hold { " and hold" } else { "" })
            }
        }
    }
}

fn count_word(word: &str) -> Option<u8> {
    Some(match word {
        "one" | "1" => 1,
        "two" | "2" => 2,
        "three" | "3" => 3,
        "four" | "4" => 4,
        _ => return None,
    })
}

/// The touchscreen: its size in pixels, unrotated, and its density.
#[derive(Debug, Clone, Copy)]
pub struct Screen {
    pub width: u32,
    pub height: u32,
    /// Pixels per inch.
    pub density: u32,
    /// Which way the device is turned.
    pub orientation: Orientation,
}

impl Screen {
    /// Reads the screen from the device.
    pub async fn read(adb: &Adb) -> Result<Screen> {
        let size = adb.shell("wm size").await?;
        let density = adb.shell("wm density").await?;
        // "Physical size: 1080x2400", maybe followed by "Override size: …",
        // which is what's in use.
        let (width, height) = last_value(&size)
            .and_then(|v| v.split_once('x'))
            .and_then(|(w, h)| Some((w.trim().parse().ok()?, h.trim().parse().ok()?)))
            .ok_or_else(|| Error::Adb(format!("AAE couldn't read the screen size: {size}")))?;
        let density = last_value(&density)
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(420);
        let orientation = display_orientation(adb).await;
        Ok(Screen {
            width,
            height,
            density,
            orientation,
        })
    }

    fn pixels_per_cm(&self) -> f64 {
        self.density as f64 / 2.54
    }
}

/// How Android has turned the display, which is how it turns touches. The
/// device may be on its side while the app on screen, such as the home
/// screen, stays upright.
pub async fn display_orientation(adb: &Adb) -> Orientation {
    read_display_orientation(adb)
        .await
        .unwrap_or(Orientation::Portrait)
}

/// As `display_orientation`, or None if the device couldn't be asked.
pub async fn read_display_orientation(adb: &Adb) -> Option<Orientation> {
    let display = adb.shell("dumpsys display").await.ok()?;
    Some(match display
        .split("mCurrentOrientation=")
        .nth(1)
        .and_then(|rest| rest.chars().next())
    {
        Some('1') => Orientation::LandscapeLeft,
        Some('2') => Orientation::UpsideDown,
        Some('3') => Orientation::LandscapeRight,
        _ => Orientation::Portrait,
    })
}

fn last_value(text: &str) -> Option<&str> {
    text.lines()
        .filter_map(|l| l.split_once(':').map(|(_, v)| v.trim()))
        .next_back()
}

/// One moment of a gesture: where each finger is, and when.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    /// Milliseconds from the start of the gesture.
    pub at: u64,
    pub touches: Vec<TouchPoint>,
}

/// How far each part of a swipe goes.
const SWIPE_CM: f64 = 2.5;
/// How long each part of a swipe takes, and in how many steps.
const SWIPE_MS: u64 = 120;
const SWIPE_STEPS: u64 = 12;
/// How far apart fingers are, side by side.
const FINGER_GAP_CM: f64 = 1.2;
/// How long a finger stays down for a tap, and the gap between taps.
const TAP_MS: u64 = 50;
const TAP_GAP_MS: u64 = 100;
/// How long the last touch of a tap and hold stays down.
const HOLD_MS: u64 = 1000;

/// One moment of a gesture while it's planned: when, where each finger is
/// on the screen as the user sees it, and whether the fingers are down.
type Moment = (u64, Vec<(f64, f64)>, bool);

/// How far from the screen's edges gestures stay, so they aren't taken as
/// the system's edge gestures, such as Back.
const EDGE_CM: f64 = 0.4;

/// Works out the touches for a gesture, centred on `at` (in pixels of the
/// screen as the user sees it, as the accessibility inspector reports
/// positions), or on the middle of the screen. A gesture that would run off
/// the screen is moved back onto it.
pub fn plan(gesture: &Gesture, screen: &Screen, at: Option<(i32, i32)>) -> Vec<Frame> {
    let cm = screen.pixels_per_cm();
    let (w, h) = user_size(screen);
    let (cx, cy) = at.map_or((w / 2.0, h / 2.0), |(x, y)| (x as f64, y as f64));
    let fingers = gesture.fingers() as usize;
    let mut moments: Vec<Moment> = Vec::new();
    match gesture {
        Gesture::Swipe { path, .. } => {
            let step = SWIPE_CM * cm;
            // Start so the whole path is centred on the point.
            let (mut dx, mut dy) = (0.0, 0.0);
            for d in path {
                let (vx, vy) = d.vector();
                dx += vx * step;
                dy += vy * step;
            }
            // Fingers side by side, across the first direction of travel.
            let (vx, vy) = path[0].vector();
            let across = (-vy, vx);
            let mut points: Vec<(f64, f64)> = (0..fingers)
                .map(|i| {
                    let offset = (i as f64 - (fingers as f64 - 1.0) / 2.0) * FINGER_GAP_CM * cm;
                    (
                        cx - dx / 2.0 + across.0 * offset,
                        cy - dy / 2.0 + across.1 * offset,
                    )
                })
                .collect();
            let mut at = 0;
            moments.push((at, points.clone(), true));
            for d in path {
                let (vx, vy) = d.vector();
                let per_step = step / SWIPE_STEPS as f64;
                for _ in 0..SWIPE_STEPS {
                    at += SWIPE_MS / SWIPE_STEPS;
                    for p in &mut points {
                        p.0 += vx * per_step;
                        p.1 += vy * per_step;
                    }
                    moments.push((at, points.clone(), true));
                }
            }
            moments.push((at + 10, points, false));
        }
        Gesture::Tap { taps, hold, .. } => {
            let points: Vec<(f64, f64)> = (0..fingers)
                .map(|i| {
                    let offset = (i as f64 - (fingers as f64 - 1.0) / 2.0) * FINGER_GAP_CM * cm;
                    (cx + offset, cy)
                })
                .collect();
            let mut at = 0;
            for tap in 0..*taps {
                moments.push((at, points.clone(), true));
                at += if *hold && tap + 1 == *taps {
                    HOLD_MS
                } else {
                    TAP_MS
                };
                moments.push((at, points.clone(), false));
                at += TAP_GAP_MS;
            }
        }
    }
    // Move the whole gesture back inside the screen's edges if it runs over.
    let margin = EDGE_CM * cm;
    let all = moments.iter().flat_map(|(_, p, _)| p.iter());
    let (mut min_x, mut max_x, mut min_y, mut max_y) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    for &(x, y) in all {
        min_x = min_x.min(x);
        max_x = max_x.max(x);
        min_y = min_y.min(y);
        max_y = max_y.max(y);
    }
    let shift = |min: f64, max: f64, size: f64| {
        if min < margin {
            margin - min
        } else if max > size - margin {
            (size - margin - max).max(margin - min)
        } else {
            0.0
        }
    };
    let (sx, sy) = (shift(min_x, max_x, w), shift(min_y, max_y, h));
    moments
        .into_iter()
        .map(|(at, points, down)| Frame {
            at,
            touches: points
                .iter()
                .enumerate()
                .map(|(id, &(x, y))| {
                    let (x, y) = to_touchscreen(screen, x + sx, y + sy);
                    TouchPoint {
                        id: id as i32,
                        x,
                        y,
                        down,
                    }
                })
                .collect(),
        })
        .collect()
}

/// The screen's width and height as the user sees it, turned with the device.
pub fn user_size(screen: &Screen) -> (f64, f64) {
    match screen.orientation {
        Orientation::Portrait | Orientation::UpsideDown => {
            (screen.width as f64, screen.height as f64)
        }
        _ => (screen.height as f64, screen.width as f64),
    }
}

/// Turns a point on the screen as the user sees it into touchscreen pixels.
/// The touchscreen doesn't turn with the device: Android turns its touches
/// to match the display.
fn to_touchscreen(screen: &Screen, x: f64, y: f64) -> (i32, i32) {
    let (w, h) = user_size(screen);
    let (tx, ty) = match screen.orientation {
        Orientation::Portrait => (x, y),
        Orientation::UpsideDown => (w - x, h - y),
        // Turned left, the touchscreen's top is on the user's left.
        Orientation::LandscapeLeft => (h - y, x),
        Orientation::LandscapeRight => (y, w - x),
    };
    (tx.round() as i32, ty.round() as i32)
}

/// Turns a point given as fractions of the screen as the user sees it, 0 at
/// the left and top and 1 at the right and bottom, into touchscreen pixels,
/// as for a touchpad standing in for the screen. The top is where the
/// status bar is, however the device is turned.
pub fn touch_at(screen: &Screen, across: f64, down: f64) -> (i32, i32) {
    let (w, h) = user_size(screen);
    let x = across.clamp(0.0, 1.0) * (w - 1.0);
    let y = down.clamp(0.0, 1.0) * (h - 1.0);
    let (tx, ty) = to_touchscreen(screen, x, y);
    // Turning measures from the far edge, which is one past the last pixel.
    (
        tx.clamp(0, screen.width as i32 - 1),
        ty.clamp(0, screen.height as i32 - 1),
    )
}

/// How often a replayed swipe moves its fingers.
const REPLAY_STEP_MS: f64 = 10.0;

/// Works out the touches to replay a tap or swipe made on a touchpad, as
/// cleanly as one from keys: each track is a finger's path, as fractions of
/// the screen as the user sees it (see [`touch_at`]), sampled together.
/// Fingers go down and up together. A finger that stays put taps; a path
/// keeps its shape, at least [`SWIPE_CM`] long and at the speed of keys.
pub fn replay(screen: &Screen, tracks: &[Vec<(f64, f64)>]) -> Vec<Frame> {
    let (w, h) = user_size(screen);
    let (w1, h1) = ((w - 1.0).max(1.0), (h - 1.0).max(1.0));
    let tracks: Vec<Vec<(f64, f64)>> = tracks
        .iter()
        .filter(|t| !t.is_empty())
        .map(|t| {
            t.iter()
                .map(|&(x, y)| (x.clamp(0.0, 1.0) * w1, y.clamp(0.0, 1.0) * h1))
                .collect()
        })
        .collect();
    let Some(samples) = tracks.iter().map(Vec::len).max() else {
        return Vec::new();
    };
    // Every finger at every sample, staying where it was once its path ends.
    let at = |track: &Vec<(f64, f64)>, k: usize| track[k.min(track.len() - 1)];
    let frame = |at: u64, points: &[(f64, f64)], down: bool| Frame {
        at,
        touches: points
            .iter()
            .enumerate()
            .map(|(id, &(x, y))| {
                let (x, y) = touch_at(screen, x / w1, y / h1);
                TouchPoint {
                    id: id as i32,
                    x,
                    y,
                    down,
                }
            })
            .collect(),
    };
    let starts: Vec<(f64, f64)> = tracks.iter().map(|t| t[0]).collect();
    let length = |track: &Vec<(f64, f64)>| {
        (1..samples)
            .map(|k| {
                let (a, b) = (at(track, k - 1), at(track, k));
                (b.0 - a.0).hypot(b.1 - a.1)
            })
            .sum::<f64>()
    };
    // The finger that goes furthest leads.
    let lead = (0..tracks.len())
        .max_by(|&a, &b| length(&tracks[a]).total_cmp(&length(&tracks[b])))
        .unwrap_or(0);
    let travel = length(&tracks[lead]);
    if travel < 1.0 {
        return vec![
            frame(0, &starts, true),
            frame(TAP_MS, &starts, false),
        ];
    }
    // Too short a swipe is drawn bigger, from where it started.
    let cm = screen.pixels_per_cm();
    let scale = (SWIPE_CM * cm / travel).max(1.0);
    let tracks: Vec<Vec<(f64, f64)>> = tracks
        .iter()
        .zip(&starts)
        .map(|(track, start)| {
            (0..samples)
                .map(|k| {
                    let p = at(track, k);
                    (
                        (start.0 + (p.0 - start.0) * scale).clamp(0.0, w1),
                        (start.1 + (p.1 - start.1) * scale).clamp(0.0, h1),
                    )
                })
                .collect()
        })
        .collect();
    // How far the lead finger has gone at each sample.
    let mut gone = vec![0.0];
    for k in 1..samples {
        let (a, b) = (tracks[lead][k - 1], tracks[lead][k]);
        gone.push(gone[k - 1] + (b.0 - a.0).hypot(b.1 - a.1));
    }
    let travel = gone[samples - 1];
    let duration = (travel / cm * SWIPE_MS as f64 / SWIPE_CM).max(SWIPE_MS as f64 / 2.0);
    let steps = ((duration / REPLAY_STEP_MS).ceil() as usize).max(2);
    let mut frames = Vec::new();
    for step in 0..=steps {
        // Evenly along the lead finger's path, the others alongside.
        let s = travel * step as f64 / steps as f64;
        let k = gone.partition_point(|&g| g <= s).clamp(1, samples - 1);
        let span = gone[k] - gone[k - 1];
        let t = if span > 0.0 {
            ((s - gone[k - 1]) / span).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let points: Vec<(f64, f64)> = tracks
            .iter()
            .map(|track| {
                let (a, b) = (track[k - 1], track[k]);
                (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
            })
            .collect();
        frames.push(frame((duration * step as f64 / steps as f64) as u64, &points, true));
    }
    let end = frames.last().map_or(0, |f| f.at) + 10;
    let last: Vec<(f64, f64)> = tracks.iter().map(|t| t[samples - 1]).collect();
    frames.push(frame(end, &last, false));
    frames
}

/// Performs a gesture on the device's screen, at a point or in the middle;
/// see [`plan`].
pub async fn perform(
    controller: &Controller,
    screen: &Screen,
    gesture: &Gesture,
    at: Option<(i32, i32)>,
) -> Result<()> {
    play(controller, plan(gesture, screen, at)).await
}

/// Performs a gesture up to its last touch and leaves the fingers down, as
/// for double tap and hold, held for as long as the user wants. Returns the
/// touches that lift them; pass them to [`lift`].
pub async fn press(
    controller: &Controller,
    screen: &Screen,
    gesture: &Gesture,
    at: Option<(i32, i32)>,
) -> Result<Vec<TouchPoint>> {
    let mut frames = plan(gesture, screen, at);
    let release = frames.pop().map(|f| f.touches).unwrap_or_default();
    play(controller, frames).await?;
    Ok(release)
}

/// Lifts the fingers [`press`] left down.
pub async fn lift(controller: &Controller, release: &[TouchPoint]) -> Result<()> {
    controller.touch_points(release).await
}

async fn play(controller: &Controller, frames: Vec<Frame>) -> Result<()> {
    let start = tokio::time::Instant::now();
    for frame in frames {
        tokio::time::sleep_until(start + Duration::from_millis(frame.at)).await;
        controller.touch_points(&frame.touches).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_gesture_names() {
        use Direction::*;
        assert_eq!(
            Gesture::parse("swipe-right"),
            Some(Gesture::swipe(&[Right]))
        );
        assert_eq!(
            Gesture::parse("Swipe up then left"),
            Some(Gesture::swipe(&[Up, Left]))
        );
        assert_eq!(
            Gesture::parse("swipe-down-up"),
            Some(Gesture::swipe(&[Down, Up]))
        );
        assert_eq!(
            Gesture::parse("two-finger-swipe-down"),
            Some(Gesture::Swipe {
                fingers: 2,
                path: vec![Down]
            })
        );
        assert_eq!(
            Gesture::parse("double-tap-hold"),
            Some(Gesture::Tap {
                fingers: 1,
                taps: 2,
                hold: true
            })
        );
        assert_eq!(
            Gesture::parse("3-finger tap"),
            Some(Gesture::Tap {
                fingers: 3,
                taps: 1,
                hold: false
            })
        );
        assert_eq!(Gesture::parse("two-finger-swipe-up-then-left"), None);
        assert_eq!(Gesture::parse("five-finger-tap"), None);
        assert_eq!(Gesture::parse("wiggle"), None);
    }

    #[test]
    fn names_round_trip() {
        for gesture in Gesture::all() {
            let name = gesture.to_string();
            assert_eq!(Gesture::parse(&name), Some(gesture), "{name}");
        }
    }

    fn screen(orientation: Orientation) -> Screen {
        Screen {
            width: 1000,
            height: 2000,
            density: 254,
            orientation,
        }
    }

    #[test]
    fn a_touchpad_tap_replays_as_a_clean_tap() {
        let frames = replay(&screen(Orientation::Portrait), &[vec![(0.5, 0.5)]]);
        let summary: Vec<(u64, i32, i32, bool)> = frames
            .iter()
            .map(|f| (f.at, f.touches[0].x, f.touches[0].y, f.touches[0].down))
            .collect();
        assert_eq!(
            summary,
            [(0, 500, 1000, true), (TAP_MS, 500, 1000, false)]
        );
    }

    #[test]
    fn a_short_touchpad_swipe_replays_long_enough_and_quick() {
        // Half a centimetre to the right, at 100 pixels a centimetre.
        let frames = replay(
            &screen(Orientation::Portrait),
            &[vec![(0.5, 0.5), (0.525, 0.5), (0.55, 0.5)]],
        );
        let first = &frames[0].touches[0];
        let last = &frames.last().unwrap().touches[0];
        assert!(first.down && !last.down);
        assert_eq!((first.x, first.y), (500, 1000));
        // Drawn to a whole swipe's length, at the speed of keys.
        assert_eq!((last.x, last.y), (750, 1000));
        assert!(frames.iter().rev().skip(1).all(|f| f.touches[0].down));
        assert!(frames.last().unwrap().at <= SWIPE_MS + 20);
        // Moving steadily, never back.
        assert!(frames.windows(2).all(|w| w[1].touches[0].x >= w[0].touches[0].x));
    }

    #[test]
    fn fingers_of_a_touchpad_swipe_go_down_and_up_together() {
        let frames = replay(
            &screen(Orientation::Portrait),
            &[
                vec![(0.4, 0.3), (0.4, 0.6)],
                vec![(0.6, 0.3), (0.6, 0.6)],
            ],
        );
        assert!(frames.iter().all(|f| f.touches.len() == 2));
        assert!(frames[0].touches.iter().all(|t| t.down));
        assert!(frames.last().unwrap().touches.iter().all(|t| !t.down));
        assert_eq!(frames.last().unwrap().touches[1].y, frames.last().unwrap().touches[0].y);
    }

    #[test]
    fn swipes_are_centred_and_quick() {
        let frames = plan(
            &Gesture::swipe(&[Direction::Right]),
            &screen(Orientation::Portrait),
            None,
        );
        let first = &frames[0].touches[0];
        let last = frames.last().unwrap();
        // 2.5 cm at 100 pixels per cm, centred.
        assert_eq!((first.x, first.y), (375, 1000));
        assert_eq!((last.touches[0].x, last.touches[0].y), (625, 1000));
        assert!(!last.touches[0].down);
        assert!(last.at <= 150);
    }

    #[test]
    fn turned_screens_swipe_the_way_the_user_sees() {
        // Turned left, the user's right is down the touchscreen.
        let frames = plan(
            &Gesture::swipe(&[Direction::Right]),
            &screen(Orientation::LandscapeLeft),
            None,
        );
        let (first, last) = (&frames[0].touches[0], &frames.last().unwrap().touches[0]);
        assert_eq!(first.x, last.x);
        assert!(last.y > first.y);
        // A point near the user's top left is near the touchscreen's bottom left.
        let tap = plan(
            &Gesture::parse("tap").unwrap(),
            &screen(Orientation::LandscapeLeft),
            Some((100, 100)),
        );
        assert_eq!((tap[0].touches[0].x, tap[0].touches[0].y), (900, 100));
    }

    #[test]
    fn turned_screens_are_centred() {
        let frames = plan(
            &Gesture::Tap {
                fingers: 1,
                taps: 1,
                hold: false,
            },
            &screen(Orientation::LandscapeLeft),
            None,
        );
        let first = &frames[0].touches[0];
        assert_eq!((first.x, first.y), (500, 1000));
    }

    #[test]
    fn gestures_near_an_edge_stay_on_screen() {
        // A swipe right from near the right edge moves left to fit, keeping
        // 0.4 cm (40 pixels) clear of the edge.
        let frames = plan(
            &Gesture::swipe(&[Direction::Right]),
            &screen(Orientation::Portrait),
            Some((990, 300)),
        );
        let last = frames.last().unwrap().touches[0];
        assert_eq!(last.x, 960);
        assert_eq!(last.y, 300);
        let tap = plan(
            &Gesture::parse("tap").unwrap(),
            &screen(Orientation::Portrait),
            Some((5, 5)),
        );
        assert_eq!((tap[0].touches[0].x, tap[0].touches[0].y), (40, 40));
    }

    #[test]
    fn fractions_cover_the_whole_screen() {
        let portrait = screen(Orientation::Portrait);
        assert_eq!(touch_at(&portrait, 0.0, 0.0), (0, 0));
        assert_eq!(touch_at(&portrait, 1.0, 1.0), (999, 1999));
        assert_eq!(touch_at(&portrait, 0.5, 0.0), (500, 0));
        // Beyond the edges stays on them.
        assert_eq!(touch_at(&portrait, -0.2, 1.3), (0, 1999));
    }

    #[test]
    fn the_top_is_the_status_bar_however_the_device_is_turned() {
        // Upside down, the user's top is the touchscreen's bottom.
        let upside_down = screen(Orientation::UpsideDown);
        assert_eq!(touch_at(&upside_down, 0.0, 0.0), (999, 1999));
        assert_eq!(touch_at(&upside_down, 1.0, 1.0), (1, 1));
        // Turned left, the user's top edge runs down the touchscreen's right
        // side, and their left is the touchscreen's top.
        let left = screen(Orientation::LandscapeLeft);
        assert_eq!(touch_at(&left, 0.0, 0.0), (999, 0));
        assert_eq!(touch_at(&left, 1.0, 0.0), (999, 1999));
        assert_eq!(touch_at(&left, 0.0, 1.0), (1, 0));
        // Turned right, the opposite.
        let right = screen(Orientation::LandscapeRight);
        assert_eq!(touch_at(&right, 0.0, 0.0), (0, 1999));
        assert_eq!(touch_at(&right, 1.0, 1.0), (999, 1));
        // Matching how planned gestures turn: the same point either way.
        let tap = plan(&Gesture::parse("tap").unwrap(), &left, Some((100, 100)));
        let (w, h) = user_size(&left);
        assert_eq!(
            touch_at(&left, 100.0 / (w - 1.0), 100.0 / (h - 1.0)),
            (tap[0].touches[0].x, tap[0].touches[0].y)
        );
    }

    #[test]
    fn taps_hold_the_last_touch() {
        let frames = plan(
            &Gesture::Tap {
                fingers: 2,
                taps: 2,
                hold: true,
            },
            &screen(Orientation::Portrait),
            None,
        );
        assert_eq!(frames.len(), 4);
        assert_eq!(frames[0].touches.len(), 2);
        assert_eq!(frames[3].at - frames[2].at, HOLD_MS);
    }
}
