//! Speech bridge client. The helper's speech relay sends each utterance here
//! (see the helper's SpeechBridge); the app speaks it and reports completion.
//!
//! Connects to the relay's abstract socket through `adb forward`, retrying
//! every 2 s while the relay isn't running.

use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

use crate::adb::Adb;

const SOCKET: &str = "localabstract:aae_speech_bridge";

/// A request from the relay.
#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    /// Speak this, and say when it's done with [`Bridge::finished`].
    Speak {
        id: u64,
        text: String,
        /// Such as "en-US".
        language: String,
        /// Android's speech rate and pitch: 100 is normal.
        rate: u32,
        pitch: u32,
    },
    /// Stop speaking at once.
    Stop,
}

/// A running bridge to one device. Dropping it ends it.
pub struct Bridge {
    replies: mpsc::UnboundedSender<u64>,
    task: tokio::task::JoinHandle<()>,
}

impl Bridge {
    /// Starts the client. `requests` is called on AAE's runtime and must
    /// return quickly. A `polite` client is refused while another is
    /// connected; an impolite one replaces it.
    pub fn start(
        adb: Adb,
        polite: bool,
        requests: impl Fn(Request) + Send + Sync + 'static,
    ) -> Bridge {
        let (replies, mut finished) = mpsc::unbounded_channel::<u64>();
        let task = tokio::spawn(async move {
            loop {
                if let Err(e) = serve(&adb, polite, &requests, &mut finished).await {
                    tracing::debug!("speech bridge: {e}");
                }
                // The connection ended: stop any utterance in progress.
                requests(Request::Stop);
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        });
        Bridge { replies, task }
    }

    /// Says an utterance has finished, or was stopped.
    pub fn finished(&self, id: u64) {
        let _ = self.replies.send(id);
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// One connection, until it ends.
async fn serve(
    adb: &Adb,
    polite: bool,
    requests: &(impl Fn(Request) + Send + Sync),
    finished: &mut mpsc::UnboundedReceiver<u64>,
) -> std::io::Result<()> {
    let port = adb
        .raw(&["forward", "tcp:0", SOCKET])
        .await
        .map_err(std::io::Error::other)?;
    let port = port.trim().to_string();
    let result = async {
        let stream = tokio::net::TcpStream::connect(format!("127.0.0.1:{port}")).await?;
        stream.set_nodelay(true)?;
        let (read, mut write) = stream.into_split();
        let hello = json!({"type": "hello", "polite": polite}).to_string() + "\n";
        write.write_all(hello.as_bytes()).await?;
        let mut lines = BufReader::new(read).lines();
        // Replies from before this connection are for an older one.
        while finished.try_recv().is_ok() {}
        loop {
            tokio::select! {
                line = lines.next_line() => {
                    let Some(line) = line? else { return Ok(()) };
                    if let Some(request) = parse(&line) {
                        requests(request);
                    }
                }
                id = finished.recv() => {
                    let Some(id) = id else { return Ok(()) };
                    let reply = json!({"type": "done", "id": id}).to_string() + "\n";
                    write.write_all(reply.as_bytes()).await?;
                }
            }
        }
    }
    .await;
    let _ = adb
        .raw(&["forward", "--remove", &format!("tcp:{port}")])
        .await;
    result
}

fn parse(line: &str) -> Option<Request> {
    let message: Value = serde_json::from_str(line).ok()?;
    match message["type"].as_str()? {
        "speak" => Some(Request::Speak {
            id: message["id"].as_u64()?,
            text: message["text"].as_str()?.to_string(),
            language: message["language"].as_str().unwrap_or("").to_string(),
            rate: message["rate"].as_u64().unwrap_or(100) as u32,
            pitch: message["pitch"].as_u64().unwrap_or(100) as u32,
        }),
        "stop" => Some(Request::Stop),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_relays_messages_are_read() {
        assert_eq!(
            parse(
                r#"{"type":"speak","id":3,"text":"Settings","language":"en-US","rate":150,"pitch":100}"#
            ),
            Some(Request::Speak {
                id: 3,
                text: "Settings".into(),
                language: "en-US".into(),
                rate: 150,
                pitch: 100,
            })
        );
        assert_eq!(parse(r#"{"type":"stop"}"#), Some(Request::Stop));
        assert_eq!(parse("nonsense"), None);
    }
}
