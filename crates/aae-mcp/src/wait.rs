//! Waiting: for text to appear on the screen or in what the screen reader
//! says, or to go, so an agent needn't keep asking.

use std::time::{Duration, Instant};

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::{ErrorData, schemars, tool, tool_router};
use serde::Deserialize;

use crate::{AaeServer, respond, text};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct WaitForParam {
    /// The device's name. Leave it out when only one device is running.
    pub device: Option<String>,
    /// The text to wait for, ignoring case: part of an element's
    /// description on the screen, or of something the screen reader says.
    pub text: String,
    /// Where to look: "screen" (the default), the accessibility tree, as
    /// inspect shows it; or "speech", what the screen reader says from now
    /// on, which needs the speech log recording (see speech_log_start).
    pub source: Option<String>,
    /// Wait for the text to go from the screen, instead of to appear.
    pub gone: Option<bool>,
    /// How long to wait, in seconds; 10 if left out, at most 120.
    pub timeout_seconds: Option<u64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct PauseParam {
    /// How long, in seconds, at most 60.
    pub seconds: f64,
}

#[tool_router(router = wait_tools, vis = "pub(crate)")]
impl AaeServer {
    /// Waits until text appears on the screen, or is spoken by the screen
    /// reader, or with `gone`, leaves the screen. Says how long it took, or
    /// what was there when it gave up.
    #[tool(annotations(read_only_hint = true))]
    async fn wait_for(
        &self,
        Parameters(p): Parameters<WaitForParam>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            async {
                let (_, session) = self.session(p.device.as_deref()).await?;
                let wanted = p.text.trim().to_lowercase();
                if wanted.is_empty() {
                    anyhow::bail!("Say what text to wait for.");
                }
                let limit = Duration::from_secs(p.timeout_seconds.unwrap_or(10).clamp(1, 120));
                let gone = p.gone.unwrap_or(false);
                let start = Instant::now();
                match p.source.as_deref().unwrap_or("screen").to_ascii_lowercase().as_str() {
                    "screen" => {
                        session.use_helper(true).await?;
                        let result = async {
                            let mut last = String::new();
                            loop {
                                last = session.screen_text().await.unwrap_or(last);
                                if last.to_lowercase().contains(&wanted) != gone {
                                    return text(format!(
                                        "\"{}\" {} the screen after {:.1} seconds.",
                                        p.text,
                                        if gone { "left" } else { "is on" },
                                        start.elapsed().as_secs_f64()
                                    ));
                                }
                                if start.elapsed() >= limit {
                                    anyhow::bail!(
                                        "After {} seconds, \"{}\" {} on the screen. The screen shows:\n{}",
                                        limit.as_secs(),
                                        p.text,
                                        if gone { "is still" } else { "isn't" },
                                        excerpt(&last)
                                    );
                                }
                                tokio::time::sleep(Duration::from_millis(500)).await;
                            }
                        }
                        .await;
                        session.use_helper(false).await?;
                        result
                    }
                    "speech" => {
                        if gone {
                            anyhow::bail!("`gone` works only for the screen.");
                        }
                        if !session.speech_log_on() {
                            anyhow::bail!(
                                "The speech log isn't recording. Start it with speech_log_start."
                            );
                        }
                        // Only what's said from now on.
                        let mut since = session.speech_log(0, false).await?.last().map_or(0, |u| u.time);
                        let mut heard: Vec<String> = Vec::new();
                        loop {
                            for said in session.speech_log(since, false).await? {
                                since = since.max(said.time);
                                if said.text.to_lowercase().contains(&wanted) {
                                    return text(format!(
                                        "The screen reader said \"{}\" after {:.1} seconds.",
                                        said.text,
                                        start.elapsed().as_secs_f64()
                                    ));
                                }
                                heard.push(said.text);
                            }
                            if start.elapsed() >= limit {
                                anyhow::bail!(
                                    "After {} seconds, the screen reader hadn't said \"{}\". {}",
                                    limit.as_secs(),
                                    p.text,
                                    if heard.is_empty() {
                                        "It said nothing.".to_string()
                                    } else {
                                        format!("It said: {}", heard.join(" | "))
                                    }
                                );
                            }
                            tokio::time::sleep(Duration::from_millis(250)).await;
                        }
                    }
                    other => anyhow::bail!("\"{other}\" isn't a source. Use screen or speech."),
                }
            }
            .await,
        )
    }

    /// Waits a while, such as for an animation or a slow screen to settle.
    #[tool(annotations(read_only_hint = true))]
    async fn pause(
        &self,
        Parameters(p): Parameters<PauseParam>,
    ) -> Result<CallToolResult, ErrorData> {
        let seconds = p.seconds.clamp(0.0, 60.0);
        tokio::time::sleep(Duration::from_secs_f64(seconds)).await;
        respond(text(format!("Waited {seconds} seconds.")))
    }
}

/// The start of a long text, for an error message.
fn excerpt(text: &str) -> String {
    const MOST: usize = 3000;
    if text.chars().count() <= MOST {
        return text.to_string();
    }
    format!("{}…", text.chars().take(MOST).collect::<String>())
}
