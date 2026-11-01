//! QEMU's machine protocol (QMP), for Googlebook devices.
//!
//! A Googlebook device runs in QEMU rather than Google's emulator. AAE gives
//! it a USB keyboard and a virtio touchscreen and nothing else that takes
//! input, so keys and touches sent here reach Android as a real keyboard and
//! touchscreen would. (Googlebook OS disables a virtio keyboard, which it
//! takes for a built-in one.)
//!
//! QEMU's control socket takes one client at a time, and the apps, `aae`,
//! `aae mcp` and `aae serve` may each have the same device open, so a
//! connection is closed once it has been idle for [`IDLE`], and opening one
//! waits while another process has the socket.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, ReadHalf, WriteHalf};
use tokio::sync::Mutex;

use crate::error::{Error, Result};
use crate::platform::{LocalStream, connect_local};

/// The touchscreen's coordinate range: QEMU scales every absolute axis to 0..=0x7fff.
const ABS_MAX: i64 = 0x7fff;

/// How long a connection stays open after its last command.
const IDLE: Duration = Duration::from_millis(250);

/// How long to wait for another process to close its connection.
const BUSY_TIMEOUT: Duration = Duration::from_secs(10);

struct Link {
    reader: BufReader<ReadHalf<LocalStream>>,
    writer: WriteHalf<LocalStream>,
}

/// Which finger is in which touchscreen slot.
#[derive(Default)]
struct Touches {
    /// Finger id to (slot, tracking id), for fingers that are down.
    down: Vec<(i32, i32, i32)>,
    next_tracking_id: i32,
}

/// A connection to one running VM's QMP socket.
#[derive(Clone)]
pub struct Qmp {
    path: PathBuf,
    link: Arc<Mutex<Option<Link>>>,
    /// Commands run, so a connection is only closed if none ran since.
    commands: Arc<AtomicU64>,
    touches: Arc<Mutex<Touches>>,
    /// The screen's size in pixels, to scale touches.
    screen: (u32, u32),
}

impl Qmp {
    /// Connects to the socket at `path`. `screen` is the display's size in pixels.
    pub async fn connect(path: &Path, screen: (u32, u32)) -> Result<Self> {
        let qmp = Qmp {
            path: path.to_path_buf(),
            link: Arc::new(Mutex::new(None)),
            commands: Arc::new(AtomicU64::new(0)),
            touches: Arc::new(Mutex::new(Touches::default())),
            screen,
        };
        qmp.execute("query-status", None).await?;
        Ok(qmp)
    }

    /// Opens a connection. While another process has one, QEMU refuses
    /// connections or holds them without its greeting, so this tries again
    /// until that one closes.
    async fn open(path: &Path) -> Result<Link> {
        let deadline = tokio::time::Instant::now() + BUSY_TIMEOUT;
        loop {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            match Self::try_open(path, left).await {
                Ok(Some(link)) => return Ok(link),
                Ok(None) if !left.is_zero() => {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                Ok(None) => {
                    return Err(Error::Vm(format!(
                        "QEMU's control socket didn't accept a connection within {} seconds. Another AAE process may be using the device.",
                        BUSY_TIMEOUT.as_secs()
                    )));
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// A connection, or None if QEMU refused it or sent no greeting within
    /// `wait`.
    async fn try_open(path: &Path, wait: Duration) -> Result<Option<Link>> {
        let stream = match connect_local(path).await {
            Ok(stream) => stream,
            Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => return Ok(None),
            Err(e) => return Err(Error::Vm(format!("QEMU's control socket: {e}"))),
        };
        let (read, writer) = tokio::io::split(stream);
        let mut link = Link {
            reader: BufReader::new(read),
            writer,
        };
        // The greeting, then capabilities negotiation.
        match tokio::time::timeout(wait, read_message(&mut link.reader)).await {
            Ok(greeting) => greeting?,
            Err(_) => return Ok(None),
        };
        send(&mut link, &json!({"execute": "qmp_capabilities"})).await?;
        Ok(Some(link))
    }

    /// Closes the connection once no command has run on it for [`IDLE`].
    fn close_when_idle(&self) {
        let count = self.commands.fetch_add(1, Ordering::Relaxed) + 1;
        let link = Arc::downgrade(&self.link);
        let commands = self.commands.clone();
        tokio::spawn(async move {
            tokio::time::sleep(IDLE).await;
            let Some(link) = link.upgrade() else {
                return;
            };
            let mut guard = link.lock().await;
            if commands.load(Ordering::Relaxed) == count {
                *guard = None;
            }
        });
    }

    /// Runs a command and returns its result.
    pub async fn execute(&self, command: &str, arguments: Option<Value>) -> Result<Value> {
        let mut request = json!({"execute": command});
        if let Some(arguments) = arguments {
            request["arguments"] = arguments;
        }
        let mut guard = self.link.lock().await;
        if guard.is_none() {
            *guard = Some(Self::open(&self.path).await?);
        }
        let link = guard.as_mut().expect("just opened");
        let reply = tokio::time::timeout(Duration::from_secs(20), send(link, &request))
            .await
            .unwrap_or_else(|_| Err(Error::Vm(format!("QEMU didn't answer {command}"))));
        let reply = match reply {
            Ok(reply) => reply,
            Err(e) => {
                // The connection may be out of step or gone; open a new one next time.
                *guard = None;
                return Err(e);
            }
        };
        drop(guard);
        self.close_when_idle();
        if let Some(error) = reply.get("error") {
            let text = error
                .get("desc")
                .and_then(Value::as_str)
                .unwrap_or("unknown error");
            return Err(Error::Vm(format!("QEMU: {text} ({command})")));
        }
        Ok(reply.get("return").cloned().unwrap_or(Value::Null))
    }

    async fn send_events(&self, events: Vec<Value>) -> Result<()> {
        self.execute("input-send-event", Some(json!({ "events": events })))
            .await
            .map(|_| ())
    }

    /// Presses or releases a key by its Linux evdev code. False if the code
    /// has no QEMU equivalent.
    pub async fn evdev_key(&self, code: i32, down: bool) -> Result<bool> {
        let Some(qcode) = qcode(code) else {
            return Ok(false);
        };
        self.send_events(vec![json!({
            "type": "key",
            "data": {"down": down, "key": {"type": "qcode", "data": qcode}}
        })])
        .await?;
        Ok(true)
    }

    /// Moves, puts down or lifts fingers, each by its id, as one touchscreen
    /// report. Coordinates are screen pixels.
    pub async fn touch_points(&self, points: &[crate::control::TouchPoint]) -> Result<()> {
        let mut touches = self.touches.lock().await;
        let was_touching = !touches.down.is_empty();
        let (width, height) = (self.screen.0.max(1) as i64, self.screen.1.max(1) as i64);
        let scale = |v: i32, size: i64| (v as i64).clamp(0, size - 1) * ABS_MAX / (size - 1).max(1);
        let mut events = Vec::new();
        for p in points {
            let known = touches.down.iter().position(|&(id, _, _)| id == p.id);
            match (known, p.down) {
                (Some(i), true) => {
                    let (_, slot, tracking) = touches.down[i];
                    events.push(mtt("update", slot, tracking, None));
                    events.push(mtt("data", slot, tracking, Some(("x", scale(p.x, width)))));
                    events.push(mtt("data", slot, tracking, Some(("y", scale(p.y, height)))));
                }
                (None, true) => {
                    let slot = (0..10)
                        .find(|s| !touches.down.iter().any(|&(_, used, _)| used == *s))
                        .ok_or_else(|| Error::Vm("more than ten fingers".into()))?;
                    touches.next_tracking_id = (touches.next_tracking_id + 1) % 0xffff;
                    let tracking = touches.next_tracking_id;
                    touches.down.push((p.id, slot, tracking));
                    events.push(mtt("begin", slot, tracking, None));
                    events.push(mtt("data", slot, tracking, Some(("x", scale(p.x, width)))));
                    events.push(mtt("data", slot, tracking, Some(("y", scale(p.y, height)))));
                }
                (Some(i), false) => {
                    let (_, slot, _) = touches.down.remove(i);
                    events.push(mtt("end", slot, -1, None));
                }
                (None, false) => {}
            }
        }
        let touching = !touches.down.is_empty();
        if touching != was_touching {
            events.push(json!({"type": "btn", "data": {"down": touching, "button": "touch"}}));
        }
        if events.is_empty() {
            return Ok(());
        }
        let result = self.send_events(events).await;
        if result.is_err() {
            touches.down.clear();
        }
        result
    }

    /// Asks the machine to power off, as its power button would.
    pub async fn power_down(&self) -> Result<()> {
        self.execute("system_powerdown", None).await.map(|_| ())
    }

    /// Ends QEMU at once.
    pub async fn quit(&self) -> Result<()> {
        self.execute("quit", None).await.map(|_| ())
    }
}

fn mtt(kind: &str, slot: i32, tracking: i32, axis: Option<(&str, i64)>) -> Value {
    let (axis, value) = axis.unwrap_or(("x", 0));
    json!({"type": "mtt", "data": {
        "type": kind, "slot": slot, "tracking-id": tracking, "axis": axis, "value": value
    }})
}

async fn send(link: &mut Link, message: &Value) -> Result<Value> {
    link.writer
        .write_all(format!("{message}\n").as_bytes())
        .await
        .map_err(|e| Error::Vm(format!("QEMU's control socket: {e}")))?;
    loop {
        let reply = read_message(&mut link.reader).await?;
        if reply.get("return").is_some() || reply.get("error").is_some() {
            return Ok(reply);
        }
    }
}

async fn read_message(reader: &mut BufReader<ReadHalf<LocalStream>>) -> Result<Value> {
    let mut line = String::new();
    let n = reader
        .read_line(&mut line)
        .await
        .map_err(|e| Error::Vm(format!("QEMU's control socket: {e}")))?;
    if n == 0 {
        return Err(Error::Vm("QEMU's control socket: closed".into()));
    }
    serde_json::from_str(&line)
        .map_err(|e| Error::Vm(format!("QEMU sent something unreadable: {e}")))
}

/// QEMU's name for the key with this Linux evdev code, for keys a USB
/// keyboard has. Android's own buttons, such as Back and Home, and media keys
/// aren't among them.
pub fn qcode(code: i32) -> Option<&'static str> {
    Some(match code {
        1 => "esc",
        2 => "1",
        3 => "2",
        4 => "3",
        5 => "4",
        6 => "5",
        7 => "6",
        8 => "7",
        9 => "8",
        10 => "9",
        11 => "0",
        12 => "minus",
        13 => "equal",
        14 => "backspace",
        15 => "tab",
        16 => "q",
        17 => "w",
        18 => "e",
        19 => "r",
        20 => "t",
        21 => "y",
        22 => "u",
        23 => "i",
        24 => "o",
        25 => "p",
        26 => "bracket_left",
        27 => "bracket_right",
        28 => "ret",
        29 => "ctrl",
        30 => "a",
        31 => "s",
        32 => "d",
        33 => "f",
        34 => "g",
        35 => "h",
        36 => "j",
        37 => "k",
        38 => "l",
        39 => "semicolon",
        40 => "apostrophe",
        41 => "grave_accent",
        42 => "shift",
        43 => "backslash",
        44 => "z",
        45 => "x",
        46 => "c",
        47 => "v",
        48 => "b",
        49 => "n",
        50 => "m",
        51 => "comma",
        52 => "dot",
        53 => "slash",
        54 => "shift_r",
        55 => "kp_multiply",
        56 => "alt",
        57 => "spc",
        58 => "caps_lock",
        59 => "f1",
        60 => "f2",
        61 => "f3",
        62 => "f4",
        63 => "f5",
        64 => "f6",
        65 => "f7",
        66 => "f8",
        67 => "f9",
        68 => "f10",
        69 => "num_lock",
        70 => "scroll_lock",
        71 => "kp_7",
        72 => "kp_8",
        73 => "kp_9",
        74 => "kp_subtract",
        75 => "kp_4",
        76 => "kp_5",
        77 => "kp_6",
        78 => "kp_add",
        79 => "kp_1",
        80 => "kp_2",
        81 => "kp_3",
        82 => "kp_0",
        83 => "kp_decimal",
        85 => "lang1",
        86 => "less",
        87 => "f11",
        88 => "f12",
        89 => "ro",
        92 => "henkan",
        93 => "katakanahiragana",
        94 => "muhenkan",
        96 => "kp_enter",
        97 => "ctrl_r",
        98 => "kp_divide",
        99 => "sysrq",
        100 => "alt_r",
        102 => "home",
        103 => "up",
        104 => "pgup",
        105 => "left",
        106 => "right",
        107 => "end",
        108 => "down",
        109 => "pgdn",
        110 => "insert",
        111 => "delete",
        113 => "audiomute",
        114 => "volumedown",
        115 => "volumeup",
        117 => "kp_equals",
        119 => "pause",
        121 => "kp_comma",
        124 => "yen",
        125 => "meta_l",
        126 => "meta_r",
        127 => "compose",
        128 => "stop",
        129 => "again",
        130 => "props",
        131 => "undo",
        132 => "front",
        133 => "copy",
        134 => "open",
        135 => "paste",
        136 => "find",
        137 => "cut",
        138 => "help",
        183 => "f13",
        184 => "f14",
        185 => "f15",
        186 => "f16",
        187 => "f17",
        188 => "f18",
        189 => "f19",
        190 => "f20",
        191 => "f21",
        192 => "f22",
        193 => "f23",
        194 => "f24",
        _ => return None,
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use tokio::net::UnixListener;

    /// A QMP server that, like QEMU's, serves one client at a time.
    fn one_client_server(path: &Path) {
        let listener = UnixListener::bind(path).unwrap();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let (read, mut write) = tokio::io::split(stream);
                let mut lines = BufReader::new(read).lines();
                write.write_all(b"{\"QMP\": {}}\n").await.unwrap();
                while let Ok(Some(_)) = lines.next_line().await {
                    if write.write_all(b"{\"return\": {}}\n").await.is_err() {
                        break;
                    }
                }
            }
        });
    }

    #[tokio::test]
    async fn processes_take_turns_on_the_socket() {
        let dir = std::env::temp_dir().join(format!("aae-qmp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("qmp.sock");
        let _ = std::fs::remove_file(&path);
        one_client_server(&path);

        let first = Qmp::connect(&path, (100, 100)).await.unwrap();
        // The second waits until the first has been idle and closes.
        let started = tokio::time::Instant::now();
        let second = Qmp::connect(&path, (100, 100)).await.unwrap();
        assert!(started.elapsed() < IDLE * 4);
        // And the first gets it back once the second is idle.
        first.execute("query-status", None).await.unwrap();
        drop(second);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
