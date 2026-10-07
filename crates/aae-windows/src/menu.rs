//! The menu bar and its keyboard shortcuts: the Mac app's, with Control for
//! Command and Alt for Option.

use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::PCWSTR;

use crate::ui;

pub const NEW_DEVICE: u16 = 100;
pub const UPDATE_TOOLS: u16 = 101;
pub const SETTINGS: u16 = 102;
pub const EXIT: u16 = 103;
pub const PASTE: u16 = 104;

pub const START: u16 = 200;
pub const STOP: u16 = 201;
pub const RESTART: u16 = 202;
pub const KEYBOARD: u16 = 203;
pub const GESTURES: u16 = 204;
pub const SPEAK_STATUS: u16 = 205;
pub const BACK: u16 = 210;
pub const HOME: u16 = 211;
pub const RECENTS: u16 = 212;
pub const NOTIFICATIONS: u16 = 213;
pub const QUICK_SETTINGS: u16 = 214;
pub const POWER: u16 = 215;
pub const ASSISTANT: u16 = 216;
pub const DEVICE_VOLUME_UP: u16 = 217;
pub const DEVICE_VOLUME_DOWN: u16 = 218;
pub const ROTATE_LEFT: u16 = 220;
pub const ROTATE_RIGHT: u16 = 221;
pub const MUTE: u16 = 222;
pub const VOLUME_UP: u16 = 223;
pub const VOLUME_DOWN: u16 = 224;
pub const CHECK_AUDIO: u16 = 225;
pub const MICROPHONE: u16 = 226;
pub const CHECK_MICROPHONE: u16 = 227;
pub const PLAY_FILE: u16 = 228;
pub const AUDIO_OUTPUT: u16 = 229;
pub const SPEECH_BRIDGE: u16 = 233;
pub const COPY_CLIPBOARD: u16 = 230;
pub const SEND_CLIPBOARD: u16 = 231;
pub const TYPE_CLIPBOARD: u16 = 232;
pub const INSTALL_APP: u16 = 240;
pub const INSTALL_SCREEN_READER: u16 = 241;
pub const SCREENSHOT: u16 = 242;
pub const RECORD: u16 = 244;
pub const RENAME: u16 = 250;
pub const HARDWARE: u16 = 254;
pub const EXPORT_DEVICE: u16 = 255;
pub const IMPORT_DEVICE: u16 = 107;
pub const COPY_DEVICE: u16 = 251;
pub const WIPE: u16 = 252;
pub const DELETE: u16 = 253;
pub const OPEN_LINK: u16 = 260;
pub const SEND_INTENT: u16 = 261;
pub const CONDITIONS: u16 = 262;
pub const DEVICE_SETTINGS: u16 = 263;
pub const SPEECH_LOG: u16 = 270;
pub const SHELL: u16 = 271;
pub const DEVICE_LOG: u16 = 272;
pub const INSPECTOR: u16 = 273;
pub const APPS: u16 = 274;
pub const SERVICES: u16 = 275;
pub const SNAPSHOTS: u16 = 276;
pub const OWN_WINDOW: u16 = 206;
pub const SERVE: u16 = 106;
pub const ANDROID_VERSIONS: u16 = 105;
pub const WATCH_BUILDS: u16 = 243;

pub const CHECK_UPDATES: u16 = 303;
pub const SELF_TEST: u16 = 300;
pub const DIAGNOSTIC_REPORT: u16 = 301;
pub const ABOUT: u16 = 302;

const CONTROL: u8 = 1;
const SHIFT: u8 = 2;
const ALT: u8 = 4;

/// A menu item: its command, its name, and its shortcut, if it has one.
struct Item(u16, &'static str, Option<(u8, u16)>);

const SEPARATOR: Item = Item(0, "", None);

fn menus() -> Vec<(&'static str, Vec<Item>)> {
    let c = CONTROL;
    let cs = CONTROL | SHIFT;
    let ca = CONTROL | ALT;
    vec![
        (
            "&File",
            vec![
                Item(NEW_DEVICE, "&New Device…", Some((c, b'N' as u16))),
                Item(IMPORT_DEVICE, "&Import Device…", None),
                Item(
                    ANDROID_VERSIONS,
                    "&Android Versions",
                    Some((ca, b'A' as u16)),
                ),
                Item(SERVE, "Serve Devices to &Phones…", None),
                Item(UPDATE_TOOLS, "&Update Android Tools…", None),
                Item(SETTINGS, "&Settings…", Some((c, VK_OEM_COMMA.0))),
                SEPARATOR,
                Item(EXIT, "E&xit", None),
            ],
        ),
        (
            "&Device",
            vec![
                Item(START, "&Start", Some((cs, b'S' as u16))),
                Item(STOP, "St&op", Some((cs, VK_OEM_PERIOD.0))),
                Item(RESTART, "&Restart", Some((cs, b'R' as u16))),
                Item(KEYBOARD, "Use Android &Keyboard", Some((cs, b'E' as u16))),
                Item(GESTURES, "Use &Gestures", Some((cs, b'G' as u16))),
                Item(OWN_WINDOW, "Open in O&wn Window", Some((ca, b'O' as u16))),
                Item(SPEAK_STATUS, "S&peak Status", Some((cs, b'I' as u16))),
                Item(
                    INSPECTOR,
                    "Accessibility &Inspector",
                    Some((ca, b'I' as u16)),
                ),
                Item(SPEECH_LOG, "Speech &Log", Some((ca, b'L' as u16))),
                Item(DEVICE_LOG, "Device Lo&g", Some((ca, b'J' as u16))),
                Item(SHELL, "S&hell", Some((ca, b'T' as u16))),
                SEPARATOR,
                Item(BACK, "&Back", Some((cs, b'B' as u16))),
                Item(HOME, "&Home", Some((cs, b'H' as u16))),
                Item(RECENTS, "Recent &Apps", Some((cs, b'A' as u16))),
                Item(NOTIFICATIONS, "&Notifications", Some((cs, b'N' as u16))),
                Item(QUICK_SETTINGS, "&Quick Settings", Some((cs, b'Q' as u16))),
                Item(POWER, "Po&wer Button", None),
                Item(ASSISTANT, "Assis&tant", None),
                Item(DEVICE_VOLUME_UP, "Device &Volume Up", None),
                Item(DEVICE_VOLUME_DOWN, "Device Volume Dow&n", None),
                SEPARATOR,
                Item(ROTATE_LEFT, "Rotate &Left", Some((cs, VK_LEFT.0))),
                Item(ROTATE_RIGHT, "Rotate Ri&ght", Some((cs, VK_RIGHT.0))),
                Item(MUTE, "&Mute Device Audio", Some((cs, b'M' as u16))),
                Item(VOLUME_UP, "Turn Device Audio &Up", Some((ca, VK_UP.0))),
                Item(
                    VOLUME_DOWN,
                    "Turn Device Audio &Down",
                    Some((ca, VK_DOWN.0)),
                ),
                Item(CHECK_AUDIO, "&Check Audio", Some((ca, b'K' as u16))),
                Item(AUDIO_OUTPUT, "Audio Ou&tput…", None),
                Item(MICROPHONE, MICROPHONE_ON, Some((cs, b'U' as u16))),
                Item(CHECK_MICROPHONE, "Check Microp&hone", None),
                Item(PLAY_FILE, PLAY_FILE_START, None),
                Item(SPEECH_BRIDGE, BRIDGE_ON, None),
                SEPARATOR,
                Item(
                    COPY_CLIPBOARD,
                    "&Copy Device Clipboard to Windows",
                    Some((cs, b'C' as u16)),
                ),
                Item(
                    SEND_CLIPBOARD,
                    "Send Windows Clipboard to De&vice",
                    Some((cs, b'V' as u16)),
                ),
                Item(
                    TYPE_CLIPBOARD,
                    "&Type Windows Clipboard on Device",
                    Some((ca, b'V' as u16)),
                ),
                SEPARATOR,
                Item(APPS, "A&pps", Some((ca, b'P' as u16))),
                Item(SERVICES, "Accessibility Ser&vices", Some((ca, b'U' as u16))),
                Item(OPEN_LINK, "Open &Link…", Some((cs, b'L' as u16))),
                Item(SEND_INTENT, "Send Int&ent…", None),
                Item(SNAPSHOTS, "Snaps&hots", Some((ca, b'S' as u16))),
                Item(
                    CONDITIONS,
                    "Batter&y, Location, Phone and Network",
                    Some((ca, b'B' as u16)),
                ),
                Item(
                    DEVICE_SETTINGS,
                    "Display and Lan&guage",
                    Some((ca, VK_OEM_COMMA.0)),
                ),
                SEPARATOR,
                Item(INSTALL_APP, "&Install App…", Some((c, b'I' as u16))),
                Item(WATCH_BUILDS, "Watch for New B&uilds…", None),
                Item(
                    INSTALL_SCREEN_READER,
                    "Install Screen Reader &Build…",
                    Some((cs | ALT, b'I' as u16)),
                ),
                Item(SCREENSHOT, "Save Screens&hot…", Some((cs, b'P' as u16))),
                Item(RECORD, RECORD_START, Some((ca, b'R' as u16))),
                SEPARATOR,
                Item(HARDWARE, "Hard&ware…", None),
                Item(EXPORT_DEVICE, "E&xport…", None),
                Item(RENAME, "Rena&me…", Some((0, VK_F2.0))),
                Item(COPY_DEVICE, "Cop&y…", Some((c, b'D' as u16))),
                Item(WIPE, "&Wipe…", None),
                Item(DELETE, "&Delete…", Some((0, VK_DELETE.0))),
            ],
        ),
        (
            "&Help",
            vec![
                Item(CHECK_UPDATES, "Check for &Updates…", None),
                SEPARATOR,
                Item(SELF_TEST, "Run &Self-Test", None),
                Item(DIAGNOSTIC_REPORT, "Save &Diagnostic Report…", None),
                SEPARATOR,
                Item(ABOUT, "&About AAE", None),
            ],
        ),
    ]
}

const MICROPHONE_ON: &str = "Turn On Micr&ophone";
const MICROPHONE_OFF: &str = "Turn Off Micr&ophone";

const BRIDGE_ON: &str = "Turn On Speech &Bridge";
const BRIDGE_OFF: &str = "Turn Off Speech &Bridge";

/// Names the speech bridge item for what it will do, as its menu opens.
pub fn name_speech_bridge(menu: HMENU, on: bool) {
    let text = ui::wide(if on { BRIDGE_OFF } else { BRIDGE_ON });
    unsafe {
        let _ = ModifyMenuW(
            menu,
            SPEECH_BRIDGE as u32,
            MF_BYCOMMAND | MF_STRING,
            SPEECH_BRIDGE as usize,
            PCWSTR(text.as_ptr()),
        );
    }
}

const RECORD_START: &str = "Record Scree&n…";
const RECORD_STOP: &str = "Stop Recordi&ng";

/// Names the recording item for what it will do, as its menu opens.
pub fn name_recording(menu: HMENU, recording: bool) {
    let text = format!(
        "{}\t{}",
        if recording { RECORD_STOP } else { RECORD_START },
        shortcut_name(CONTROL | ALT, b'R' as u16)
    );
    let text = ui::wide(&text);
    unsafe {
        let _ = ModifyMenuW(
            menu,
            RECORD as u32,
            MF_BYCOMMAND | MF_STRING,
            RECORD as usize,
            PCWSTR(text.as_ptr()),
        );
    }
}

const PLAY_FILE_START: &str = "Play Audio &File into Microphone…";
const PLAY_FILE_STOP: &str = "Stop Playing Audio &File";

/// Names the microphone items for what they will do, as their menu opens.
pub fn name_microphone(menu: HMENU, on: bool, playing: bool) {
    let play = ui::wide(if playing {
        PLAY_FILE_STOP
    } else {
        PLAY_FILE_START
    });
    unsafe {
        let _ = ModifyMenuW(
            menu,
            PLAY_FILE as u32,
            MF_BYCOMMAND | MF_STRING,
            PLAY_FILE as usize,
            PCWSTR(play.as_ptr()),
        );
    }
    let text = format!(
        "{}\t{}",
        if on { MICROPHONE_OFF } else { MICROPHONE_ON },
        shortcut_name(CONTROL | SHIFT, b'U' as u16)
    );
    let text = ui::wide(&text);
    unsafe {
        // Fails harmlessly for menus without it.
        let _ = ModifyMenuW(
            menu,
            MICROPHONE as u32,
            MF_BYCOMMAND | MF_STRING,
            MICROPHONE as usize,
            PCWSTR(text.as_ptr()),
        );
    }
}

fn shortcut_name(modifiers: u8, key: u16) -> String {
    let mut parts = Vec::new();
    if modifiers & CONTROL != 0 {
        parts.push("Ctrl".to_string());
    }
    if modifiers & SHIFT != 0 {
        parts.push("Shift".to_string());
    }
    if modifiers & ALT != 0 {
        parts.push("Alt".to_string());
    }
    parts.push(match VIRTUAL_KEY(key) {
        VK_OEM_COMMA => "Comma".into(),
        VK_OEM_PERIOD => "Period".into(),
        VK_LEFT => "Left".into(),
        VK_RIGHT => "Right".into(),
        VK_UP => "Up".into(),
        VK_DOWN => "Down".into(),
        VK_DELETE => "Del".into(),
        VK_F2 => "F2".into(),
        _ => char::from(key as u8).to_string(),
    });
    parts.join("+")
}

/// Builds the menu bar and the table of shortcuts.
pub fn build() -> (HMENU, HACCEL) {
    let mut accels = vec![ACCEL {
        fVirt: FVIRTKEY | FCONTROL,
        key: b'V' as u16,
        cmd: PASTE,
    }];
    unsafe {
        let bar = CreateMenu().unwrap();
        for (title, items) in menus() {
            let menu = CreatePopupMenu().unwrap();
            for Item(id, name, shortcut) in items {
                if id == 0 {
                    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
                    continue;
                }
                let mut text = name.to_string();
                if let Some((modifiers, key)) = shortcut {
                    text = format!("{text}\t{}", shortcut_name(modifiers, key));
                    let mut flags = FVIRTKEY;
                    if modifiers & CONTROL != 0 {
                        flags |= FCONTROL;
                    }
                    if modifiers & SHIFT != 0 {
                        flags |= FSHIFT;
                    }
                    if modifiers & ALT != 0 {
                        flags |= FALT;
                    }
                    accels.push(ACCEL {
                        fVirt: flags,
                        key,
                        cmd: id,
                    });
                }
                let text = ui::wide(&text);
                let _ = AppendMenuW(menu, MF_STRING, id as usize, PCWSTR(text.as_ptr()));
            }
            let title = ui::wide(title);
            let _ = AppendMenuW(bar, MF_POPUP, menu.0 as usize, PCWSTR(title.as_ptr()));
        }
        let table = CreateAcceleratorTableW(&accels).unwrap();
        (bar, table)
    }
}
