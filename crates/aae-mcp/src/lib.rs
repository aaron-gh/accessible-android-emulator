//! AAE's MCP server: lets AI agents run, use and inspect AAE's Android
//! devices, as a tester would, through the Model Context Protocol.
//!
//! `aae mcp` runs it over standard input and output, the way agents such as
//! Claude Code start local servers. It calls the same core as the Mac and
//! Windows apps, so devices behave exactly as they do there, and an agent can
//! work on a device while you watch or listen in the app.
//!
//! Besides the screen, an agent can hear what a blind user hears: the speech
//! log records what the screen reader says, so screen reader behaviour can
//! be tested, not just what's drawn.
//!
//! Tools that delete things or run arbitrary commands are left out unless
//! the server is started with `--allow-destructive`.

use std::collections::HashMap;
use std::sync::Arc;

use aae_ffi::{DeviceInfo, Engine, Session};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig};
use rmcp::{ErrorData, ServerHandler, ServiceExt, schemars, tool, tool_handler, tool_router};
use serde::Deserialize;
use tokio::sync::Mutex;

/// How the server is started.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Offer tools that delete devices, apps or data, or run shell commands.
    pub allow_destructive: bool,
}

/// Runs the server over standard input and output until the agent closes it.
pub async fn serve(options: Options) -> anyhow::Result<()> {
    let server = AaeServer::new(options)?;
    let service = server
        .serve(rmcp::transport::stdio())
        .await
        .map_err(|e| anyhow::anyhow!("The MCP connection didn't start: {e}"))?;
    service.waiting().await?;
    Ok(())
}

#[derive(Clone)]
pub struct AaeServer {
    engine: Arc<Engine>,
    /// Connections to running devices, by device id.
    sessions: Arc<Mutex<HashMap<String, Arc<Session>>>>,
    options: Options,
    tool_router: ToolRouter<Self>,
}

/// The result of a tool: what it found, or why it couldn't, which the agent
/// sees either way.
pub(crate) type Outcome = anyhow::Result<CallToolResult>;

pub(crate) fn text(text: impl Into<String>) -> Outcome {
    Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
}

pub(crate) fn json(value: &impl serde::Serialize) -> Outcome {
    text(serde_json::to_string_pretty(value)?)
}

/// Turns a failure into a result the agent reads, rather than a protocol error.
pub(crate) fn respond(outcome: Outcome) -> Result<CallToolResult, ErrorData> {
    Ok(outcome
        .unwrap_or_else(|e| CallToolResult::error(vec![ContentBlock::text(format!("{e:#}"))])))
}

/// Picks a device. Every tool that works on a device takes this.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct DeviceParam {
    /// The device's name, as list_devices gives it. Leave it out when only
    /// one device is running.
    pub device: Option<String>,
}

impl AaeServer {
    pub fn new(options: Options) -> anyhow::Result<Self> {
        let engine = Engine::new().map_err(|e| anyhow::anyhow!("{e}"))?;
        let tool_router = Self::device_tools();
        Ok(AaeServer {
            engine,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            options,
            tool_router,
        })
    }

    /// Finds a device by its name or id, ignoring case, or the only one running.
    pub(crate) fn device(&self, name: Option<&str>) -> anyhow::Result<DeviceInfo> {
        let devices = self.engine.devices()?;
        match name.map(str::trim).filter(|n| !n.is_empty()) {
            Some(name) => devices
                .iter()
                .find(|d| d.name.eq_ignore_ascii_case(name) || d.id == name)
                .cloned()
                .ok_or_else(|| {
                    let names: Vec<&str> = devices.iter().map(|d| d.name.as_str()).collect();
                    anyhow::anyhow!(
                        "There's no device called \"{name}\". The devices are: {}.",
                        if names.is_empty() {
                            "none yet".into()
                        } else {
                            names.join(", ")
                        }
                    )
                }),
            None => {
                let running: Vec<&DeviceInfo> = devices.iter().filter(|d| d.running).collect();
                match running.as_slice() {
                    [one] => Ok((*one).clone()),
                    [] => anyhow::bail!(
                        "No device is running. Start one with start_device, or name one with `device`."
                    ),
                    several => anyhow::bail!(
                        "Several devices are running: {}. Say which with `device`.",
                        several
                            .iter()
                            .map(|d| d.name.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                }
            }
        }
    }

    /// The connection to a running device, made the first time it's needed.
    pub(crate) async fn session(
        &self,
        name: Option<&str>,
    ) -> anyhow::Result<(DeviceInfo, Arc<Session>)> {
        let device = self.device(name)?;
        if !device.running {
            self.sessions.lock().await.remove(&device.id);
            anyhow::bail!("{} isn't running. Start it with start_device.", device.name);
        }
        let mut sessions = self.sessions.lock().await;
        if let Some(session) = sessions.get(&device.id) {
            return Ok((device, session.clone()));
        }
        let session = self.engine.open_session(device.id.clone()).await?;
        sessions.insert(device.id.clone(), session.clone());
        Ok((device, session))
    }

    /// Forgets a device's connection, after it stops or restarts.
    pub(crate) async fn forget(&self, id: &str) {
        self.sessions.lock().await.remove(id);
    }
}

#[derive(serde::Serialize)]
struct DeviceSummary {
    name: String,
    android: String,
    kind: String,
    size: String,
    running: bool,
    screen_reader: Option<String>,
    speech_log: bool,
}

impl From<&DeviceInfo> for DeviceSummary {
    fn from(d: &DeviceInfo) -> Self {
        DeviceSummary {
            name: d.name.clone(),
            android: d.android.clone(),
            kind: d.kind.clone(),
            size: d.profile.clone(),
            running: d.running,
            screen_reader: d.screen_reader.clone(),
            speech_log: d.speech_log,
        }
    }
}

#[tool_router(router = device_tools)]
impl AaeServer {
    /// Lists AAE's devices: name, Android version, image kind, size, whether
    /// it's running, its screen reader, and whether its speech log is on.
    #[tool(annotations(read_only_hint = true))]
    async fn list_devices(&self) -> Result<CallToolResult, ErrorData> {
        respond((|| {
            let devices = self.engine.devices()?;
            json(&devices.iter().map(DeviceSummary::from).collect::<Vec<_>>())
        })())
    }

    /// Says how a device is: its Android version, whether it's running, and
    /// whether its screen reader is running.
    #[tool(annotations(read_only_hint = true))]
    async fn device_status(
        &self,
        Parameters(p): Parameters<DeviceParam>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            async {
                let device = self.device(p.device.as_deref())?;
                if !device.running {
                    self.forget(&device.id).await;
                    return text(format!("{}, {}. Stopped.", device.name, device.android));
                }
                let (_, session) = self.session(Some(&device.id)).await?;
                let reader = session.screen_reader_status().await?;
                text(format!(
                    "{}, {}. Running. {reader}",
                    device.name, device.android
                ))
            }
            .await,
        )
    }

    /// Checks everything AAE needs: virtualisation, the Android SDK, AAE's
    /// own parts, and each running device's screen reader and speech.
    #[tool(annotations(read_only_hint = true))]
    async fn self_test(&self) -> Result<CallToolResult, ErrorData> {
        let checks = self.engine.self_test().await;
        let lines: Vec<String> = checks
            .iter()
            .map(|c| {
                let outcome = match c.outcome {
                    aae_ffi::CheckOutcome::Passed => "passed",
                    aae_ffi::CheckOutcome::Warning => "warning",
                    aae_ffi::CheckOutcome::Failed => "failed",
                };
                format!("{}: {outcome}. {}", c.name, c.detail)
            })
            .collect();
        respond(text(lines.join("\n")))
    }
}

const INSTRUCTIONS: &str = "AAE, the Accessible Android Emulator, runs Android virtual devices that each have a screen reader (such as TalkBack or Backtalk) on. Use it to test Android apps, especially their accessibility. Devices are named; tools take a `device`, which can be left out when only one is running. Typical flow: list_devices, start_device if needed, then look with screenshot, inspect (the accessibility tree) and touch_targets, act with tap_target or gesture (as a screen reader user would: swipe-right moves to the next item, double-tap activates it) or type_text and press_key, and listen with the speech log (speech_log_start, then speech_log) to hear exactly what the screen reader said. Snapshots make repeatable starting points.";

#[tool_handler(router = self.tool_router)]
impl ServerHandler for AaeServer {
    fn get_info(&self) -> ServerConfig {
        let mut info = Implementation::new("aae", env!("CARGO_PKG_VERSION"));
        info.title = Some("Accessible Android Emulator".into());
        let mut instructions = INSTRUCTIONS.to_string();
        if !self.options.allow_destructive {
            instructions.push_str(" Tools that delete devices, apps or data, or run shell commands, are off; the user can turn them on by starting the server with --allow-destructive.");
        }
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(info)
            .with_instructions(instructions)
    }
}
