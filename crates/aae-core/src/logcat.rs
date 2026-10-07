//! Reading the device log (`adb logcat`), with each line's process name, so it
//! can be filtered by app, tag, level and text.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use crate::adb::Adb;
use crate::error::Result;
use crate::platform::NoConsole;

/// How many lines a [`LogStream`] keeps; older ones are dropped.
const CAPACITY: usize = 50_000;

/// How important a log line is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub enum Level {
    Verbose,
    Debug,
    Info,
    Warning,
    Error,
    Fatal,
}

impl Level {
    pub const ALL: [Level; 6] = [
        Level::Verbose,
        Level::Debug,
        Level::Info,
        Level::Warning,
        Level::Error,
        Level::Fatal,
    ];

    /// The level for logcat's letter, such as `E`.
    pub fn from_letter(letter: char) -> Option<Level> {
        Some(match letter {
            'V' => Level::Verbose,
            'D' => Level::Debug,
            'I' => Level::Info,
            'W' => Level::Warning,
            'E' => Level::Error,
            'F' | 'A' => Level::Fatal,
            _ => return None,
        })
    }

    /// A level from its letter or name, such as "w" or "warning".
    pub fn parse(text: &str) -> Option<Level> {
        let text = text.trim().to_lowercase();
        if let [letter] = text.to_uppercase().chars().collect::<Vec<_>>()[..] {
            return Level::from_letter(letter);
        }
        Level::ALL
            .into_iter()
            .find(|level| level.name() == text || (text == "warn" && *level == Level::Warning))
    }

    pub fn letter(self) -> char {
        match self {
            Level::Verbose => 'V',
            Level::Debug => 'D',
            Level::Info => 'I',
            Level::Warning => 'W',
            Level::Error => 'E',
            Level::Fatal => 'F',
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Level::Verbose => "verbose",
            Level::Debug => "debug",
            Level::Info => "info",
            Level::Warning => "warning",
            Level::Error => "error",
            Level::Fatal => "fatal",
        }
    }
}

/// One line of the device log.
#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    /// Counts up from 1 for each line read, so a reader can ask for what's new.
    pub seq: u64,
    /// The device's clock time, such as "10-06 18:27:36.037".
    pub time: String,
    pub pid: u32,
    pub tid: u32,
    pub level: Level,
    pub tag: String,
    pub message: String,
    /// The process that logged it, usually the app's package name.
    pub process: Option<String>,
}

impl Entry {
    /// The time of day, such as "18:27:36.037".
    pub fn clock(&self) -> &str {
        self.time.split_once(' ').map_or(&self.time, |(_, t)| t)
    }

    /// The line as a screen reader user wants it: what matters first, level
    /// only when it's a warning or worse.
    pub fn spoken(&self) -> String {
        if self.level >= Level::Warning {
            format!("{}, {}: {}", self.level.name(), self.tag, self.message)
        } else {
            format!("{}: {}", self.tag, self.message)
        }
    }

    /// The line as text for saving, like logcat's own with the process added.
    pub fn to_line(&self) -> String {
        format!(
            "{} {:>5} {:>5} {} {} {}: {}",
            self.time,
            self.pid,
            self.tid,
            self.level.letter(),
            self.process.as_deref().unwrap_or("?"),
            self.tag,
            self.message
        )
    }
}

/// Reads a `logcat -v threadtime` line, such as
/// `10-06 18:24:59.768   290   290 I Zygote  : Process 2287 exited`.
/// Lines that aren't log entries, such as `--------- beginning of main`, give None.
pub fn parse_line(line: &str) -> Option<Entry> {
    fn token(text: &str) -> Option<(&str, &str)> {
        let text = text.trim_start();
        let end = text.find(char::is_whitespace)?;
        Some((&text[..end], &text[end..]))
    }
    let (date, rest) = token(line)?;
    let (time, rest) = token(rest)?;
    let (pid, rest) = token(rest)?;
    let (tid, rest) = token(rest)?;
    let (level, rest) = token(rest)?;
    let mut level_chars = level.chars();
    let level = match (level_chars.next(), level_chars.next()) {
        (Some(letter), None) => Level::from_letter(letter)?,
        _ => return None,
    };
    if date.len() != 5 || !date.contains('-') {
        return None;
    }
    let rest = rest.trim_start();
    let (tag, message) = match rest.find(": ") {
        Some(i) => (&rest[..i], &rest[i + 2..]),
        None => (rest.strip_suffix(':')?, ""),
    };
    Some(Entry {
        seq: 0,
        time: format!("{date} {time}"),
        pid: pid.parse().ok()?,
        tid: tid.parse().ok()?,
        level,
        tag: tag.trim_end().to_string(),
        message: message.to_string(),
        process: None,
    })
}

/// Which log lines to show. Empty fields match everything.
#[derive(Debug, Clone, Default)]
pub struct Filter {
    /// A process or package name. Its other processes, such as
    /// `com.example:remote`, match too.
    pub process: Option<String>,
    /// A tag, matched exactly but ignoring case.
    pub tag: Option<String>,
    /// The least important level shown.
    pub level: Option<Level>,
    /// Text to find in the tag or message, ignoring case.
    pub text: Option<String>,
}

impl Filter {
    pub fn matches(&self, entry: &Entry) -> bool {
        if let Some(level) = self.level {
            if entry.level < level {
                return false;
            }
        }
        if let Some(wanted) = non_empty(&self.process) {
            match &entry.process {
                Some(process) if same_app(process, wanted) => {}
                _ => return false,
            }
        }
        if let Some(tag) = non_empty(&self.tag) {
            if !entry.tag.eq_ignore_ascii_case(tag.trim()) {
                return false;
            }
        }
        if let Some(text) = non_empty(&self.text) {
            let text = text.to_lowercase();
            if !entry.tag.to_lowercase().contains(&text)
                && !entry.message.to_lowercase().contains(&text)
            {
                return false;
            }
        }
        true
    }
}

fn non_empty(value: &Option<String>) -> Option<&str> {
    value.as_deref().map(str::trim).filter(|v| !v.is_empty())
}

fn same_app(process: &str, wanted: &str) -> bool {
    process == wanted
        || process
            .strip_prefix(wanted)
            .is_some_and(|rest| rest.starts_with(':'))
}

/// Which process each process ID belongs to.
#[derive(Debug, Default)]
struct Processes {
    names: HashMap<u32, String>,
    refreshed: Option<Instant>,
}

impl Processes {
    /// Learns process names from Android's "Start proc" lines, which name
    /// processes that may have ended before `ps` could see them.
    fn learn(&mut self, entry: &Entry) {
        if entry.tag == "ActivityManager" {
            if let Some((pid, name)) = started_process(&entry.message) {
                self.names.insert(pid, name);
            }
        }
    }

    /// Reads the running processes, at most once every two seconds.
    async fn refresh(&mut self, adb: &Adb) {
        if self
            .refreshed
            .is_some_and(|at| at.elapsed() < Duration::from_secs(2))
        {
            return;
        }
        self.refreshed = Some(Instant::now());
        // Android 8 and later list every process with -A; earlier versions
        // list them all by default and don't know the option.
        let mut found = match adb.shell("ps -A -o PID,NAME").await {
            Ok(out) => parse_ps(&out),
            Err(_) => HashMap::new(),
        };
        if found.len() < 5 {
            if let Ok(out) = adb.shell("ps").await {
                found = parse_ps(&out);
            }
        }
        self.names.extend(found);
    }

    fn name(&self, pid: u32) -> Option<String> {
        self.names.get(&pid).cloned()
    }
}

/// Reads ActivityManager's "Start proc 1543:com.example/u0a53 for …" (Android
/// 7 and later) or "Start proc com.example for …: pid=1543 uid=…" (earlier).
fn started_process(message: &str) -> Option<(u32, String)> {
    let rest = message.strip_prefix("Start proc ")?;
    let first = rest.split_whitespace().next()?;
    if let Some((pid, name)) = first.split_once(':') {
        if let Ok(pid) = pid.parse() {
            let name = name.split('/').next().unwrap_or(name);
            return Some((pid, name.to_string()));
        }
    }
    let pid = rest
        .split_once("pid=")?
        .1
        .split(|c: char| !c.is_ascii_digit())
        .next()?
        .parse()
        .ok()?;
    Some((pid, first.to_string()))
}

/// Reads `ps` output: the PID column by its heading, the name from the end.
fn parse_ps(out: &str) -> HashMap<u32, String> {
    let mut lines = out.lines();
    let Some(pid_column) = lines
        .next()
        .and_then(|header| header.split_whitespace().position(|h| h == "PID"))
    else {
        return HashMap::new();
    };
    lines
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let pid = fields.get(pid_column)?.parse().ok()?;
            Some((pid, fields.last()?.to_string()))
        })
        .collect()
}

/// The whole device log as it is now.
pub async fn dump(adb: &Adb) -> Result<Vec<Entry>> {
    let out = adb.raw(&["logcat", "-d", "-v", "threadtime"]).await?;
    let mut processes = Processes::default();
    processes.refresh(adb).await;
    let mut entries: Vec<Entry> = out.lines().filter_map(parse_line).collect();
    for entry in &entries {
        processes.learn(entry);
    }
    for (i, entry) in entries.iter_mut().enumerate() {
        entry.seq = i as u64 + 1;
        entry.process = processes.name(entry.pid);
    }
    Ok(entries)
}

#[derive(Default)]
struct Shared {
    entries: VecDeque<Entry>,
    next_seq: u64,
    /// Why the log isn't being read right now, if it isn't.
    problem: Option<String>,
}

/// The device log, read as it's written, keeping the last lines in memory.
/// Reading stops when this is dropped.
pub struct LogStream {
    shared: Arc<Mutex<Shared>>,
    task: tokio::task::JoinHandle<()>,
}

impl LogStream {
    /// Starts reading the log, beginning with the last `history` lines. Must
    /// be called on a Tokio runtime.
    pub fn start(adb: Adb, history: usize) -> LogStream {
        let shared = Arc::new(Mutex::new(Shared {
            next_seq: 1,
            ..Default::default()
        }));
        let task = tokio::spawn(read_log(adb, history.max(1), shared.clone()));
        LogStream { shared, task }
    }

    /// The last `limit` lines after `since` that match the filter, oldest first.
    pub fn entries(&self, since: u64, filter: &Filter, limit: usize) -> Vec<Entry> {
        let shared = self.shared.lock().unwrap();
        let mut found: Vec<Entry> = shared
            .entries
            .iter()
            .rev()
            .take_while(|e| e.seq > since)
            .filter(|e| filter.matches(e))
            .take(limit)
            .cloned()
            .collect();
        found.reverse();
        found
    }

    /// The number of the last line read, or 0.
    pub fn latest(&self) -> u64 {
        self.shared.lock().unwrap().next_seq - 1
    }

    /// The processes that have logged something, by name.
    pub fn processes(&self) -> Vec<String> {
        let shared = self.shared.lock().unwrap();
        let names: BTreeSet<&str> = shared
            .entries
            .iter()
            .filter_map(|e| e.process.as_deref())
            .collect();
        names.into_iter().map(String::from).collect()
    }

    /// Forgets the lines read so far. The device's own log is left alone.
    pub fn clear(&self) {
        self.shared.lock().unwrap().entries.clear();
    }

    /// Why the log isn't being read right now, if it isn't.
    pub fn problem(&self) -> Option<String> {
        self.shared.lock().unwrap().problem.clone()
    }
}

impl Drop for LogStream {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn read_log(adb: Adb, history: usize, shared: Arc<Mutex<Shared>>) {
    let mut processes = Processes::default();
    // After the first run, logcat restarts (say after Android restarts) from
    // the time of the last line read, skipping lines already seen.
    let mut last_time: Option<String> = None;
    loop {
        processes.refresh(&adb).await;
        let start = match &last_time {
            Some(time) => time.clone(),
            None => history.to_string(),
        };
        let child = Command::new(&adb.bin)
            .no_console()
            .args([
                "-s",
                &adb.serial,
                "logcat",
                "-v",
                "threadtime",
                "-T",
                &start,
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn();
        let mut child = match child {
            Ok(child) => child,
            Err(e) => {
                shared.lock().unwrap().problem = Some(format!("adb could not run: {e}"));
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
        };
        let Some(stdout) = child.stdout.take() else {
            continue;
        };
        let mut lines = BufReader::new(stdout).lines();
        let mut read_any = false;
        // Lines up to this time were read before logcat restarted.
        let mut replayed_until = last_time.clone();
        while let Ok(Some(line)) = lines.next_line().await {
            let Some(mut entry) = parse_line(&line) else {
                continue;
            };
            if !read_any {
                read_any = true;
                shared.lock().unwrap().problem = None;
            }
            if let Some(until) = &replayed_until {
                if entry.time.as_str() <= until.as_str() {
                    continue;
                }
                replayed_until = None;
            }
            processes.learn(&entry);
            if processes.name(entry.pid).is_none() {
                processes.refresh(&adb).await;
            }
            entry.process = processes.name(entry.pid);
            last_time = Some(entry.time.clone());
            let mut shared = shared.lock().unwrap();
            entry.seq = shared.next_seq;
            shared.next_seq += 1;
            shared.entries.push_back(entry);
            if shared.entries.len() > CAPACITY {
                shared.entries.pop_front();
            }
        }
        let _ = child.wait().await;
        if !read_any && last_time.is_some() {
            // This logcat may not take a time to start from; start afresh.
            last_time = None;
        }
        shared.lock().unwrap().problem =
            Some("The device's log stopped. AAE is reconnecting.".into());
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_log_lines() {
        let entry = parse_line(
            "10-06 18:24:59.768   290   290 I Zygote  : Process 2287 exited due to signal 9 (Killed)",
        )
        .unwrap();
        assert_eq!(entry.time, "10-06 18:24:59.768");
        assert_eq!(entry.clock(), "18:24:59.768");
        assert_eq!((entry.pid, entry.tid), (290, 290));
        assert_eq!(entry.level, Level::Info);
        assert_eq!(entry.tag, "Zygote");
        assert_eq!(
            entry.message,
            "Process 2287 exited due to signal 9 (Killed)"
        );

        let entry =
            parse_line("10-06 18:23:00.004  1438  1452 E memtrack: Couldn't load: module").unwrap();
        assert_eq!(entry.tag, "memtrack");
        assert_eq!(entry.message, "Couldn't load: module");
        assert_eq!(entry.spoken(), "error, memtrack: Couldn't load: module");

        let entry = parse_line("10-06 18:23:00.004  1438  1452 D EmptyTag:").unwrap();
        assert_eq!(
            (entry.tag.as_str(), entry.message.as_str()),
            ("EmptyTag", "")
        );

        assert!(parse_line("--------- beginning of main").is_none());
        assert!(parse_line("").is_none());
    }

    #[test]
    fn learns_started_processes() {
        assert_eq!(
            started_process("Start proc 1543:com.android.inputmethod.latin/u0a53 for service x"),
            Some((1543, "com.android.inputmethod.latin".into()))
        );
        assert_eq!(
            started_process(
                "Start proc com.example for activity com.example/.Main: pid=2001 uid=10050 gids={}"
            ),
            Some((2001, "com.example".into()))
        );
        assert_eq!(started_process("Killing 1543:com.x/u0a53"), None);
    }

    #[test]
    fn reads_process_lists() {
        let new = "  PID NAME\n    1 init\n  662 com.android.systemui\n";
        assert_eq!(parse_ps(new).get(&662).unwrap(), "com.android.systemui");
        let old = "USER     PID   PPID  VSIZE  RSS     WCHAN    PC        NAME\n\
                   root      1     0     8904   788   SyS_epoll_ 00000000 S /init\n\
                   u0_a53    1543  1258  1000   100   SyS_epoll_ 00000000 S com.android.inputmethod.latin\n";
        let found = parse_ps(old);
        assert_eq!(found.get(&1).unwrap(), "/init");
        assert_eq!(found.get(&1543).unwrap(), "com.android.inputmethod.latin");
    }

    #[test]
    fn filters_lines() {
        let mut entry =
            parse_line("10-06 18:23:00.004  1438  1452 W ActivityManager: Slow op").unwrap();
        entry.process = Some("com.example:remote".into());
        let filter = |f: Filter| f.matches(&entry);
        assert!(filter(Filter::default()));
        assert!(filter(Filter {
            process: Some("com.example".into()),
            ..Default::default()
        }));
        assert!(!filter(Filter {
            process: Some("com.ex".into()),
            ..Default::default()
        }));
        assert!(filter(Filter {
            tag: Some("activitymanager".into()),
            ..Default::default()
        }));
        assert!(filter(Filter {
            level: Some(Level::Warning),
            ..Default::default()
        }));
        assert!(!filter(Filter {
            level: Some(Level::Error),
            ..Default::default()
        }));
        assert!(filter(Filter {
            text: Some("SLOW".into()),
            ..Default::default()
        }));
        assert!(!filter(Filter {
            text: Some("fast".into()),
            ..Default::default()
        }));
    }

    #[test]
    fn reads_level_names() {
        assert_eq!(Level::parse("w"), Some(Level::Warning));
        assert_eq!(Level::parse("Error"), Some(Level::Error));
        assert_eq!(Level::parse("warn"), Some(Level::Warning));
        assert_eq!(Level::parse("loud"), None);
    }
}
