//! Announcements, spoken through the user's screen reader when one is running,
//! and otherwise through a system voice, using Prism.
//!
//! Prism picks the best way to speak at each announcement: VoiceOver, NVDA,
//! JAWS, Narrator or Orca if running, or else a system voice. This matters in
//! device mode, where the user usually turns their screen reader off so it
//! doesn't fight Android's; AAE must still be heard then.
//!
//! On macOS, Prism's backends (VoiceOver and AVSpeech) need the process's main
//! thread to be running its run loop, as an app's is. In a command-line tool,
//! where it isn't, choosing a backend never returns, so the `aae` command
//! prints its messages instead of announcing them.

use std::sync::mpsc;

/// How an announcement treats speech already in progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priority {
    /// Queues behind anything being spoken. Used for success and information.
    Normal,
    /// Interrupts whatever is being spoken. Used for failures, which must not
    /// wait behind stale news.
    Interrupt,
}

enum Message {
    Say(String, Priority),
    Stop,
}

/// Where announcements go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// The user's screen reader if one is running, otherwise a system voice.
    /// Needs an app with a window and a running main thread, as the Mac app has.
    Best,
    /// Always a system voice. For command-line tools, which have no window for
    /// a screen reader to speak from.
    SystemVoice,
}

/// Speaks announcements on a thread of its own. Cheap to clone; all clones
/// share the same thread.
#[derive(Clone)]
pub struct Announcer {
    tx: mpsc::Sender<Message>,
}

impl Announcer {
    /// Starts the speech thread. Speech failures are logged, never returned:
    /// an announcement that can't be spoken must not stop the work it describes.
    pub fn new(route: Route) -> Self {
        let (tx, rx) = mpsc::channel::<Message>();
        let spawned = std::thread::Builder::new()
            .name("aae-speech".into())
            .spawn(move || run(rx, route));
        if let Err(e) = spawned {
            tracing::warn!("could not start the speech thread: {e}");
        }
        Announcer { tx }
    }

    pub fn say(&self, text: impl Into<String>, priority: Priority) {
        let _ = self.tx.send(Message::Say(text.into(), priority));
    }

    /// Says something without interrupting.
    pub fn info(&self, text: impl Into<String>) {
        self.say(text, Priority::Normal);
    }

    /// Says a failure, interrupting anything being spoken.
    pub fn failure(&self, text: impl Into<String>) {
        self.say(text, Priority::Interrupt);
    }

    /// Stops speech in progress, where the backend allows it.
    pub fn stop(&self) {
        let _ = self.tx.send(Message::Stop);
    }
}

#[cfg(feature = "speech")]
fn run(rx: mpsc::Receiver<Message>, route: Route) {
    tracing::debug!("starting Prism");
    let prism = match prismer::Prism::new() {
        Ok(prism) => prism,
        Err(e) => {
            tracing::warn!("speech is unavailable: {e}");
            // Keep draining so senders never block or fail.
            for message in rx {
                if let Message::Say(text, _) = message {
                    tracing::info!("announcement (not spoken): {text}");
                }
            }
            return;
        }
    };
    for id in prism.backend_ids() {
        tracing::debug!(
            "speech backend available: {}",
            prism.backend_name(id).unwrap_or_default()
        );
    }
    let system_voice = system_voice_backend(&prism);
    let mut last_backend = String::new();
    for message in rx {
        // Ask for the best backend each time: the user may have turned their
        // screen reader on or off since the last announcement.
        let chosen = match (route, system_voice) {
            (Route::SystemVoice, Some(id)) => {
                prism.acquire(id).and_then(|b| match b.initialize() {
                    Ok(()) | Err(prismer::Error::AlreadyInitialized) => Ok(b),
                    Err(e) => Err(e),
                })
            }
            _ => prism.acquire_best(),
        };
        let backend = match chosen {
            Ok(backend) => backend,
            Err(e) => {
                tracing::warn!("no speech backend is available: {e}");
                continue;
            }
        };
        let name = backend.name();
        if name != last_backend {
            tracing::debug!("speaking through {name}");
            last_backend = name;
        }
        let result = match message {
            Message::Say(text, priority) => {
                tracing::info!("announcement: {text}");
                backend.speak(&text, priority == Priority::Interrupt)
            }
            Message::Stop => backend.stop(),
        };
        if let Err(e) = result {
            tracing::debug!("speech request failed: {e}");
        }
    }
}

/// The backend that speaks with a system voice on this platform.
#[cfg(feature = "speech")]
fn system_voice_backend(prism: &prismer::Prism) -> Option<prismer::BackendId> {
    const NAMES: &[&str] = &[
        "AVSpeech",
        "OneCore",
        "SAPI",
        "SpeechDispatcher",
        "Speech Dispatcher",
    ];
    prism.backend_ids().into_iter().find(|&id| {
        prism
            .backend_name(id)
            .is_some_and(|name| NAMES.iter().any(|n| n.eq_ignore_ascii_case(&name)))
    })
}

#[cfg(not(feature = "speech"))]
fn run(rx: mpsc::Receiver<Message>, _route: Route) {
    for message in rx {
        if let Message::Say(text, _) = message {
            tracing::info!("announcement: {text}");
        }
    }
}
