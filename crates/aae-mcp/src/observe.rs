//! Looking at a device: screenshots, the accessibility tree and its checks,
//! the things that can be touched, what the screen reader said, and the
//! device's log.

use base64::Engine as _;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock};
use rmcp::{ErrorData, schemars, tool, tool_router};
use serde::{Deserialize, Serialize};

use crate::{AaeServer, DeviceParam, json, respond, text};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct ScreenshotParam {
    /// The device's name. Leave it out when only one device is running.
    pub device: Option<String>,
    /// How many pixels across, at most; the shape is kept. Defaults to 540,
    /// about half a phone's width. 0 gives the screen's full size.
    pub width: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct InspectParam {
    /// The device's name. Leave it out when only one device is running.
    pub device: Option<String>,
    /// "text" (the default): each element as a screen reader describes it,
    /// indented inside its parent. "json": every property of every element.
    pub format: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct SpeechLogParam {
    /// The device's name. Leave it out when only one device is running.
    pub device: Option<String>,
    /// Only what was said after this time, in milliseconds since 1970, as a
    /// previous call's `next_since` gives. Leave it out for everything recorded.
    pub since: Option<u64>,
    /// Empty the log after reading it.
    pub clear: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct DeviceLogParam {
    /// The device's name. Leave it out when only one device is running.
    pub device: Option<String>,
    /// Only lines from this app's package, or another process, by name.
    pub app: Option<String>,
    /// Only lines with this tag.
    pub tag: Option<String>,
    /// The least important level shown: verbose, debug, info, warning,
    /// error or fatal.
    pub level: Option<String>,
    /// Only lines containing this text, in the tag or message.
    pub text: Option<String>,
    /// Only lines after this one, as a previous call's `next_since` gives.
    pub since: Option<u64>,
    /// The most lines to return, the newest ones; 200 if left out.
    pub limit: Option<u32>,
}

#[derive(Serialize)]
struct Target {
    label: String,
    x: i32,
    y: i32,
    bounds: [i32; 4],
}

#[derive(Serialize)]
struct Targets {
    screen_width: i32,
    screen_height: i32,
    targets: Vec<Target>,
}

#[derive(Serialize)]
struct Utterance {
    time: u64,
    clock: String,
    text: String,
}

#[derive(Serialize)]
struct SpeechLog {
    utterances: Vec<Utterance>,
    next_since: u64,
}

#[derive(Serialize)]
struct LogLines {
    lines: Vec<String>,
    next_since: u64,
}

fn level(name: &str) -> anyhow::Result<aae_ffi::LogLevel> {
    use aae_ffi::LogLevel::*;
    Ok(match name.to_ascii_lowercase().as_str() {
        "verbose" | "v" => Verbose,
        "debug" | "d" => Debug,
        "info" | "i" => Info,
        "warning" | "warn" | "w" => Warning,
        "error" | "e" => Error,
        "fatal" | "f" | "assert" => Fatal,
        other => anyhow::bail!(
            "\"{other}\" isn't a log level. Use verbose, debug, info, warning, error or fatal."
        ),
    })
}

#[tool_router(router = observe_tools, vis = "pub(crate)")]
impl AaeServer {
    /// Takes a screenshot of a device. Positions for tap and gesture are in
    /// the screen's own pixels, which the reply gives; a scaled screenshot
    /// needs its positions scaling up to match.
    #[tool(annotations(read_only_hint = true))]
    async fn screenshot(
        &self,
        Parameters(p): Parameters<ScreenshotParam>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            async {
                let (device, session) = self.session(p.device.as_deref()).await?;
                let full = session.screenshot_png(0).await?;
                let png = match p.width.unwrap_or(540) {
                    0 => full,
                    width => shrink(&full, width)?,
                };
                let size = session.clone().screen_size().await?;
                let (width, height) = png_size(&png).unwrap_or((0, 0));
                Ok(CallToolResult::success(vec![
                    ContentBlock::image(base64::engine::general_purpose::STANDARD.encode(&png), "image/png"),
                    ContentBlock::text(format!(
                        "{}'s screen, shown at {width} by {height} pixels. The screen is {} by {} pixels, which tap and gesture positions use.",
                        device.name, size.x, size.y
                    )),
                ]))
            }
            .await,
        )
    }

    /// Reads the screen's accessibility tree: every window and element, each
    /// described as a screen reader says it, with the accessibility problems
    /// found on the screen after it.
    #[tool(annotations(read_only_hint = true))]
    async fn inspect(
        &self,
        Parameters(p): Parameters<InspectParam>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            async {
                let (_, session) = self.session(p.device.as_deref()).await?;
                let inspection = session.inspect().await?;
                if p.format
                    .as_deref()
                    .is_some_and(|f| f.eq_ignore_ascii_case("json"))
                {
                    return text(inspection.json);
                }
                let mut out = inspection.text;
                out.push_str("\n\n");
                out.push_str(&issues_text(&inspection.issues));
                text(out)
            }
            .await,
        )
    }

    /// Checks the current screen for accessibility problems: unlabelled
    /// controls, images without descriptions, small touch targets and
    /// duplicate labels.
    #[tool(annotations(read_only_hint = true))]
    async fn check_accessibility(
        &self,
        Parameters(p): Parameters<DeviceParam>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            async {
                let (_, session) = self.session(p.device.as_deref()).await?;
                let inspection = session.inspect().await?;
                text(issues_text(&inspection.issues))
            }
            .await,
        )
    }

    /// Lists the things on the screen that can be touched, in reading order,
    /// as a screen reader would visit them: each one's label, its centre, and
    /// its bounds as [left, top, right, bottom], in the screen's pixels.
    /// tap_target and gesture can then act on one.
    #[tool(annotations(read_only_hint = true))]
    async fn touch_targets(
        &self,
        Parameters(p): Parameters<DeviceParam>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            async {
                let (_, session) = self.session(p.device.as_deref()).await?;
                let screen = self.targets(&session).await?;
                json(&Targets {
                    screen_width: screen.width,
                    screen_height: screen.height,
                    targets: screen
                        .targets
                        .into_iter()
                        .map(|t| Target {
                            label: t.label,
                            x: t.x,
                            y: t.y,
                            bounds: [t.left, t.top, t.right, t.bottom],
                        })
                        .collect(),
                })
            }
            .await,
        )
    }

    /// Starts recording what the device's screen reader says, so speech_log
    /// can read it. Recording stays on, for
    /// the app and later sessions too, until speech_log_stop.
    #[tool(annotations(destructive_hint = false, idempotent_hint = true))]
    async fn speech_log_start(
        &self,
        Parameters(p): Parameters<DeviceParam>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            async {
                let (_, session) = self.session(p.device.as_deref()).await?;
                if session.speech_log_on() {
                    return text("The speech log is already recording.");
                }
                text(session.set_speech_log(true).await?)
            }
            .await,
        )
    }

    /// Stops recording what the screen reader says.
    #[tool(annotations(destructive_hint = false, idempotent_hint = true))]
    async fn speech_log_stop(
        &self,
        Parameters(p): Parameters<DeviceParam>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            async {
                let (_, session) = self.session(p.device.as_deref()).await?;
                if !session.speech_log_on() {
                    return text("The speech log isn't recording.");
                }
                text(session.set_speech_log(false).await?)
            }
            .await,
        )
    }

    /// Reads the screen reader's utterances, oldest first, with times. The
    /// speech log must be recording; see
    /// speech_log_start. Pass the reply's next_since to get only what's new.
    #[tool(annotations(read_only_hint = true))]
    async fn speech_log(
        &self,
        Parameters(p): Parameters<SpeechLogParam>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            async {
                let (_, session) = self.session(p.device.as_deref()).await?;
                if !session.speech_log_on() {
                    anyhow::bail!(
                        "The speech log isn't recording. Start it with speech_log_start."
                    );
                }
                let since = p.since.unwrap_or(0);
                let said = session.speech_log(since, p.clear.unwrap_or(false)).await?;
                let next_since = said.last().map_or(since, |u| u.time);
                json(&SpeechLog {
                    utterances: said
                        .into_iter()
                        .map(|u| Utterance {
                            time: u.time,
                            clock: u.clock,
                            text: u.text,
                        })
                        .collect(),
                    next_since,
                })
            }
            .await,
        )
    }

    /// Reads the device's log (logcat), filtered by app, tag, level and
    /// text, newest lines last. The first call starts reading, with the
    /// last few thousand lines. Pass the reply's next_since to get only
    /// what's new.
    #[tool(annotations(read_only_hint = true))]
    async fn device_log(
        &self,
        Parameters(p): Parameters<DeviceLogParam>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            async {
                let (_, session) = self.session(p.device.as_deref()).await?;
                let first = session.log_latest() == 0;
                session.start_logs();
                if first {
                    // Give it a moment to read the recent lines.
                    for _ in 0..20 {
                        if session.log_latest() > 0 {
                            break;
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    }
                }
                if let Some(problem) = session.log_problem() {
                    anyhow::bail!("{problem}");
                }
                let filter = aae_ffi::LogFilter {
                    process: p.app,
                    tag: p.tag,
                    level: p.level.as_deref().map(level).transpose()?,
                    text: p.text,
                };
                let entries =
                    session.log_entries(p.since.unwrap_or(0), filter, p.limit.unwrap_or(200));
                json(&LogLines {
                    lines: entries.iter().map(|e| e.line.clone()).collect(),
                    next_since: session.log_latest(),
                })
            }
            .await,
        )
    }
}

impl AaeServer {
    /// The touch targets, with AAE's helper on just while it reads them.
    pub(crate) async fn targets(
        &self,
        session: &std::sync::Arc<aae_ffi::Session>,
    ) -> anyhow::Result<aae_ffi::TouchTargets> {
        session.use_helper(true).await?;
        let targets = session.clone().touch_targets().await;
        session.use_helper(false).await?;
        Ok(targets?)
    }
}

fn issues_text(issues: &[aae_ffi::IssueInfo]) -> String {
    if issues.is_empty() {
        return "No accessibility problems found on this screen.".into();
    }
    let mut out = format!("{} accessibility problems:", issues.len());
    for issue in issues {
        out.push_str(&format!(
            "\n- {}: {} ({}{})",
            if issue.error { "Error" } else { "Warning" },
            issue.message,
            issue.element,
            issue
                .id
                .as_ref()
                .map(|id| format!(", {id}"))
                .unwrap_or_default()
        ));
    }
    out
}

/// Scales a PNG down to at most `max_width` pixels across, keeping its
/// shape, by averaging blocks of pixels. The emulator's own scaling is
/// ignored by some versions, so AAE does it.
fn shrink(png_data: &[u8], max_width: u32) -> anyhow::Result<Vec<u8>> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(png_data));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info()?;
    let mut pixels = vec![0; reader.output_buffer_size().unwrap_or(0)];
    let info = reader.next_frame(&mut pixels)?;
    let channels = match info.color_type {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Grayscale => 1,
        png::ColorType::Indexed => anyhow::bail!("The screenshot came in a form AAE can't scale."),
    };
    let (width, height) = (info.width as usize, info.height as usize);
    let factor = (width as u32).div_ceil(max_width.max(1)).max(1) as usize;
    if factor == 1 {
        return Ok(png_data.to_vec());
    }
    let (out_width, out_height) = (width / factor, height / factor);
    let mut out = vec![0u8; out_width * out_height * channels];
    for y in 0..out_height {
        for x in 0..out_width {
            for c in 0..channels {
                let mut sum = 0u32;
                for dy in 0..factor {
                    let row = (y * factor + dy) * info.line_size;
                    for dx in 0..factor {
                        sum += pixels[row + (x * factor + dx) * channels + c] as u32;
                    }
                }
                out[(y * out_width + x) * channels + c] = (sum / (factor * factor) as u32) as u8;
            }
        }
    }
    let mut encoded = Vec::new();
    let mut encoder = png::Encoder::new(&mut encoded, out_width as u32, out_height as u32);
    encoder.set_color(info.color_type);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&out)?;
    Ok(encoded)
}

/// A PNG's width and height, from its header.
fn png_size(png: &[u8]) -> Option<(u32, u32)> {
    let header = png.get(16..24)?;
    Some((
        u32::from_be_bytes(header[0..4].try_into().ok()?),
        u32::from_be_bytes(header[4..8].try_into().ok()?),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A PNG of the given size, every pixel the same colour.
    fn png(width: u32, height: u32, colour: [u8; 4]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let pixels: Vec<u8> = colour.repeat((width * height) as usize);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&pixels)
            .unwrap();
        out
    }

    #[test]
    fn shrinks_screenshots_keeping_their_shape_and_colour() {
        let full = png(1080, 2400, [10, 20, 30, 255]);
        assert_eq!(png_size(&full), Some((1080, 2400)));
        let small = shrink(&full, 540).unwrap();
        assert_eq!(png_size(&small), Some((540, 1200)));
        let decoder = png::Decoder::new(std::io::Cursor::new(&small));
        let mut reader = decoder.read_info().unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        reader.next_frame(&mut pixels).unwrap();
        assert_eq!(&pixels[..4], &[10, 20, 30, 255]);
    }

    #[test]
    fn leaves_small_screenshots_alone() {
        let full = png(300, 600, [0, 0, 0, 255]);
        assert_eq!(shrink(&full, 540).unwrap(), full);
    }

    #[test]
    fn reads_log_levels() {
        assert!(matches!(
            level("Warning").unwrap(),
            aae_ffi::LogLevel::Warning
        ));
        assert!(matches!(level("e").unwrap(), aae_ffi::LogLevel::Error));
        assert!(level("loud").is_err());
    }
}
