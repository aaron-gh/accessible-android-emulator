//! Acting on a device, as a user would: gestures and taps, typing and keys,
//! opening apps, links and intents, turning it, and the conditions around
//! it, such as its battery, location and calls.

use aae_ffi::{CallAction, IntentExtra, IntentInfo, ScreenPoint, TouchTarget};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::{ErrorData, schemars, tool, tool_router};
use serde::Deserialize;

use crate::{AaeServer, DeviceParam, respond, text};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct Point {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct GestureParam {
    /// The device's name. Leave it out when only one device is running.
    pub device: Option<String>,
    /// Gestures to perform in order, such as ["swipe-right", "double-tap"].
    /// Swipes: swipe-up, swipe-down, swipe-left, swipe-right; two-part
    /// swipes such as swipe-up-then-left; two-, three- or four-finger
    /// swipes such as two-finger-swipe-down. Taps: tap, double-tap,
    /// triple-tap, tap-hold (a long press), double-tap-hold, and
    /// two-finger-tap and the like.
    pub gestures: Vec<String>,
    /// Where to perform them, in the screen's pixels; the middle of the
    /// screen if left out. With a screen reader on, swipes and double taps
    /// work anywhere; a tap at a point moves its focus there.
    pub at: Option<Point>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct TargetParam {
    /// The device's name. Leave it out when only one device is running.
    pub device: Option<String>,
    /// The thing to touch, by its label as touch_targets lists it: an exact
    /// match first, ignoring case, otherwise the first label containing it.
    pub target: Option<String>,
    /// Or the thing's position in touch_targets' list, counting from 0.
    pub index: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct TextParam {
    /// The device's name. Leave it out when only one device is running.
    pub device: Option<String>,
    pub text: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct KeysParam {
    /// The device's name. Leave it out when only one device is running.
    pub device: Option<String>,
    /// Keys to press in order. Android buttons: back, home, recents, power,
    /// volume-up, volume-down, mute. Keys: enter, escape, tab, space,
    /// backspace, delete, up, down, left, right, home-key, end, page-up,
    /// page-down, insert, menu, F1 to F12, or any single character. Add
    /// modifiers with plus: ctrl, alt, shift, meta, as in meta+right, which
    /// moves TalkBack to the next item.
    pub keys: Vec<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct AppParam {
    /// The device's name. Leave it out when only one device is running.
    pub device: Option<String>,
    /// The app, by the name people see, such as "Settings", or its package.
    pub app: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct LinkParam {
    /// The device's name. Leave it out when only one device is running.
    pub device: Option<String>,
    /// A web address or an app's own link, such as myapp://home.
    pub link: String,
    /// The app's package to open it in; whichever Android picks if left out.
    pub app: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct IntentParam {
    /// The device's name. Leave it out when only one device is running.
    pub device: Option<String>,
    /// Such as android.intent.action.VIEW.
    pub action: Option<String>,
    /// A link or other address.
    pub data: Option<String>,
    /// An app's package, or package/class for one of its screens or receivers.
    pub target: Option<String>,
    /// Text extras, by key.
    pub extras: Option<std::collections::BTreeMap<String, String>>,
    /// Send it as a broadcast, rather than to open a screen.
    pub broadcast: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct RotateParam {
    /// The device's name. Leave it out when only one device is running.
    pub device: Option<String>,
    /// "left" or "right": which way to turn it, a quarter turn.
    pub direction: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct BatteryParam {
    /// The device's name. Leave it out when only one device is running.
    pub device: Option<String>,
    /// From 0 to 100.
    pub level: u32,
    pub charging: bool,
    /// The battery's health: good, failed, dead, overvoltage or overheated.
    /// Leave out to keep it as it is.
    pub health: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct FingerprintParam {
    /// The device's name. Leave it out when only one device is running.
    pub device: Option<String>,
    /// Which finger, from 1 to 10. Touch with the same finger while Android's
    /// settings enroll one, and later to unlock with it; any other is refused.
    pub finger: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct NetworkParam {
    /// The device's name. Leave it out when only one device is running.
    pub device: Option<String>,
    /// Turns airplane mode on or off. Leave out to keep it as it is.
    pub airplane: Option<bool>,
    /// Turns Wi-Fi on or off.
    pub wifi: Option<bool>,
    /// Turns mobile data on or off.
    pub data: Option<bool>,
    /// The connection's speed and delay: full, lte, 3g, slow-3g, edge or gprs.
    pub speed: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct MicrophoneFileParam {
    /// The device's name. Leave it out when only one device is running.
    pub device: Option<String>,
    /// The sound file's full path: WAV, MP3, FLAC, Ogg Vorbis or M4A.
    pub path: String,
    /// Wait until it has played, and say how it went. Otherwise it returns
    /// at once, and the file plays when an app on the device next records,
    /// such as after voice typing starts.
    #[serde(default)]
    pub wait: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct LocationParam {
    /// The device's name. Leave it out when only one device is running.
    pub device: Option<String>,
    pub latitude: f64,
    pub longitude: f64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct SmsParam {
    /// The device's name. Leave it out when only one device is running.
    pub device: Option<String>,
    /// The number it's from.
    pub from: String,
    pub text: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct CallParam {
    /// The device's name. Leave it out when only one device is running.
    pub device: Option<String>,
    /// What the other end does: "ring" calls the device from `number`;
    /// "answer" or "busy" for a call the device is making; "hold",
    /// "resume" and "hang-up" for a call in progress.
    pub action: String,
    /// The other end's number.
    pub number: String,
}

fn find_target<'a>(targets: &'a [TouchTarget], p: &TargetParam) -> anyhow::Result<&'a TouchTarget> {
    if let Some(index) = p.index {
        return targets.get(index).ok_or_else(|| {
            anyhow::anyhow!(
                "There are only {} things on the screen to touch.",
                targets.len()
            )
        });
    }
    let Some(wanted) = p.target.as_deref().map(str::trim).filter(|t| !t.is_empty()) else {
        anyhow::bail!("Say which thing to touch, with `target` or `index`.");
    };
    let lower = wanted.to_lowercase();
    targets
        .iter()
        .find(|t| t.label.to_lowercase() == lower)
        .or_else(|| targets.iter().find(|t| t.label.to_lowercase().contains(&lower)))
        .ok_or_else(|| {
            let labels: Vec<&str> = targets.iter().map(|t| t.label.as_str()).collect();
            anyhow::anyhow!(
                "Nothing on the screen is called \"{wanted}\". The things that can be touched are: {}.",
                labels.join("; ")
            )
        })
}

#[tool_router(router = act_tools, vis = "pub(crate)")]
impl AaeServer {
    /// Performs screen reader gestures, in order, with simulated fingers on
    /// the touchscreen, so the screen reader sees real touches. With a
    /// screen reader on, swipe-right and swipe-left move between items,
    /// double-tap activates the focused one, and two-finger swipes scroll.
    #[tool(annotations(destructive_hint = false))]
    async fn gesture(
        &self,
        Parameters(p): Parameters<GestureParam>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            async {
                let (_, session) = self.session(p.device.as_deref()).await?;
                if p.gestures.is_empty() {
                    anyhow::bail!("Say which gestures to perform.");
                }
                let at = p.at.map(|a| ScreenPoint { x: a.x, y: a.y });
                for name in &p.gestures {
                    session.clone().perform_gesture(name.clone(), at).await?;
                }
                text(format!("Performed {}.", p.gestures.join(", then ")))
            }
            .await,
        )
    }

    /// Taps a thing on the screen, found by its label or position in
    /// touch_targets. With a screen reader on, this moves the screen
    /// reader's focus to it and reads it out, as exploring by touch does;
    /// activate_target also activates it.
    #[tool(annotations(destructive_hint = false))]
    async fn tap_target(
        &self,
        Parameters(p): Parameters<TargetParam>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            async {
                let (_, session) = self.session(p.device.as_deref()).await?;
                let screen = self.targets(&session).await?;
                let target = find_target(&screen.targets, &p)?;
                let at = ScreenPoint {
                    x: target.x,
                    y: target.y,
                };
                session
                    .clone()
                    .perform_gesture("tap".into(), Some(at))
                    .await?;
                text(format!(
                    "Tapped {}, at {}, {}.",
                    target.label, target.x, target.y
                ))
            }
            .await,
        )
    }

    /// Activates a thing on the screen, found by its label or position in
    /// touch_targets, as a user would: with a screen reader on, a tap to
    /// focus it then a double tap; with none, a single tap.
    #[tool(annotations(destructive_hint = false))]
    async fn activate_target(
        &self,
        Parameters(p): Parameters<TargetParam>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            async {
                let (_, session) = self.session(p.device.as_deref()).await?;
                let screen = self.targets(&session).await?;
                let target = find_target(&screen.targets, &p)?;
                let at = ScreenPoint {
                    x: target.x,
                    y: target.y,
                };
                let reader_on = session.screen_reader_status().await?.ends_with("is on.");
                session
                    .clone()
                    .perform_gesture("tap".into(), Some(at))
                    .await?;
                if reader_on {
                    // Long enough for the screen reader to take the focus.
                    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                    session
                        .clone()
                        .perform_gesture("double-tap".into(), Some(at))
                        .await?;
                }
                text(format!("Activated {}.", target.label))
            }
            .await,
        )
    }

    /// Types text into the focused field, as key presses.
    #[tool(annotations(destructive_hint = false))]
    async fn type_text(
        &self,
        Parameters(p): Parameters<TextParam>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            async {
                let (_, session) = self.session(p.device.as_deref()).await?;
                session.type_text(p.text.clone()).await?;
                text(format!("Typed {} characters.", p.text.chars().count()))
            }
            .await,
        )
    }

    /// Presses keys and Android's buttons, in order, such as back, home, or
    /// meta+right for the screen reader's next item.
    #[tool(annotations(destructive_hint = false))]
    async fn press_key(
        &self,
        Parameters(p): Parameters<KeysParam>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            async {
                let (_, session) = self.session(p.device.as_deref()).await?;
                if p.keys.is_empty() {
                    anyhow::bail!("Say which keys to press.");
                }
                for key in &p.keys {
                    session.press(key.clone()).await?;
                }
                text(format!("Pressed {}.", p.keys.join(", then ")))
            }
            .await,
        )
    }

    /// Opens an app, as its icon would, by the name people see or its package.
    #[tool(annotations(destructive_hint = false))]
    async fn open_app(
        &self,
        Parameters(p): Parameters<AppParam>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            async {
                let (_, session) = self.session(p.device.as_deref()).await?;
                let package = self.app_package(&session, &p.app).await?;
                session.open_app(package.clone()).await?;
                text(format!("Opened {package}."))
            }
            .await,
        )
    }

    /// Opens a web address or an app's own link.
    #[tool(annotations(destructive_hint = false))]
    async fn open_link(
        &self,
        Parameters(p): Parameters<LinkParam>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            async {
                let (_, session) = self.session(p.device.as_deref()).await?;
                session.open_link(p.link.clone(), p.app).await?;
                text(format!("Opened {}.", p.link))
            }
            .await,
        )
    }

    /// Sends an intent, to open a screen or as a broadcast, and says what
    /// Android answered.
    #[tool(annotations(destructive_hint = false))]
    async fn send_intent(
        &self,
        Parameters(p): Parameters<IntentParam>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            async {
                let (_, session) = self.session(p.device.as_deref()).await?;
                let answer = session
                    .send_intent(IntentInfo {
                        action: p.action,
                        data: p.data,
                        target: p.target,
                        extras: p
                            .extras
                            .unwrap_or_default()
                            .into_iter()
                            .map(|(key, value)| IntentExtra { key, value })
                            .collect(),
                        broadcast: p.broadcast.unwrap_or(false),
                    })
                    .await?;
                text(if answer.is_empty() {
                    "Sent.".into()
                } else {
                    answer
                })
            }
            .await,
        )
    }

    /// Pulls down the notifications.
    #[tool(annotations(destructive_hint = false, idempotent_hint = true))]
    async fn open_notifications(
        &self,
        Parameters(p): Parameters<DeviceParam>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            async {
                let (_, session) = self.session(p.device.as_deref()).await?;
                session
                    .shell("cmd statusbar expand-notifications".into())
                    .await?;
                text("The notifications are open.")
            }
            .await,
        )
    }

    /// Pulls down the quick settings.
    #[tool(annotations(destructive_hint = false, idempotent_hint = true))]
    async fn open_quick_settings(
        &self,
        Parameters(p): Parameters<DeviceParam>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            async {
                let (_, session) = self.session(p.device.as_deref()).await?;
                session
                    .shell("cmd statusbar expand-settings".into())
                    .await?;
                text("The quick settings are open.")
            }
            .await,
        )
    }

    /// Turns the device a quarter turn, left or right, and says which way
    /// up it is now.
    #[tool(annotations(destructive_hint = false))]
    async fn rotate(
        &self,
        Parameters(p): Parameters<RotateParam>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            async {
                let (_, session) = self.session(p.device.as_deref()).await?;
                let left = match p.direction.to_ascii_lowercase().as_str() {
                    "left" => true,
                    "right" => false,
                    other => anyhow::bail!("\"{other}\" isn't a direction. Use left or right."),
                };
                text(format!("{}.", session.rotate(left).await?))
            }
            .await,
        )
    }

    /// Sets the battery level and whether it's charging.
    #[tool(annotations(destructive_hint = false, idempotent_hint = true))]
    async fn set_battery(
        &self,
        Parameters(p): Parameters<BatteryParam>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            async {
                let (_, session) = self.session(p.device.as_deref()).await?;
                session.set_battery(p.level.min(100), p.charging).await?;
                if let Some(health) = p.health {
                    session.set_battery_health(health).await?;
                }
                text(format!(
                    "The battery is at {}%, {}, health {}.",
                    p.level.min(100),
                    if p.charging {
                        "charging"
                    } else {
                        "not charging"
                    },
                    session.battery_health().await?
                ))
            }
            .await,
        )
    }

    /// The device's network: turns airplane mode, Wi-Fi or mobile data on
    /// or off, or sets the connection's speed, to test an app offline or on a
    /// poor connection. With nothing to change, says how the network is.
    #[tool(annotations(destructive_hint = false, idempotent_hint = true))]
    async fn network(
        &self,
        Parameters(p): Parameters<NetworkParam>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            async {
                let (_, session) = self.session(p.device.as_deref()).await?;
                let changing = p.airplane.is_some()
                    || p.wifi.is_some()
                    || p.data.is_some()
                    || p.speed.is_some();
                if let Some(on) = p.airplane {
                    session.set_airplane_mode(on).await?;
                }
                if let Some(on) = p.wifi {
                    session.set_wifi(on).await?;
                }
                if let Some(on) = p.data {
                    session.set_mobile_data(on).await?;
                }
                if let Some(speed) = p.speed {
                    session.set_network_speed(speed).await?;
                }
                if changing {
                    // Android takes a moment to report changes.
                    tokio::time::sleep(std::time::Duration::from_millis(800)).await;
                }
                text(session.network().await?.description)
            }
            .await,
        )
    }

    /// Plays a sound file into the device's microphone the next time an app
    /// on it records, from the start, for testing voice input such as voice
    /// typing or search. Start the app listening after calling this.
    #[tool(annotations(destructive_hint = false))]
    async fn play_into_microphone(
        &self,
        Parameters(p): Parameters<MicrophoneFileParam>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            async {
                let (_, session) = self.session(p.device.as_deref()).await?;
                // Says now if it isn't sound, rather than when it would play.
                let sound = aae_core::microphone::decode(std::path::Path::new(&p.path))?;
                if p.wait {
                    return text(session.play_into_microphone(p.path).await?);
                }
                tokio::spawn(async move {
                    if let Err(e) = session.play_into_microphone(p.path).await {
                        tracing::warn!("playing into the microphone: {e}");
                    }
                });
                text(format!(
                    "{:.1} seconds of audio will play into the device's microphone when an app on it next records.",
                    sound.seconds()
                ))
            }
            .await,
        )
    }

    /// Touches the fingerprint sensor with a finger, then lifts it.
    #[tool(annotations(destructive_hint = false))]
    async fn touch_fingerprint(
        &self,
        Parameters(p): Parameters<FingerprintParam>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            async {
                let (_, session) = self.session(p.device.as_deref()).await?;
                text(session.touch_fingerprint(p.finger.unwrap_or(1)).await?)
            }
            .await,
        )
    }

    /// Shakes the device, as for apps that act on a shake.
    #[tool(annotations(destructive_hint = false))]
    async fn shake(
        &self,
        Parameters(p): Parameters<DeviceParam>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            async {
                let (_, session) = self.session(p.device.as_deref()).await?;
                session.shake().await?;
                text("Shook the device.")
            }
            .await,
        )
    }

    /// Sets where the device is, by latitude and longitude.
    #[tool(annotations(destructive_hint = false, idempotent_hint = true))]
    async fn set_location(
        &self,
        Parameters(p): Parameters<LocationParam>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            async {
                let (_, session) = self.session(p.device.as_deref()).await?;
                session.set_location(p.latitude, p.longitude).await?;
                text(format!("The device is at {}, {}.", p.latitude, p.longitude))
            }
            .await,
        )
    }

    /// Sends the device a text message, as if from a number.
    #[tool(annotations(destructive_hint = false))]
    async fn send_sms(
        &self,
        Parameters(p): Parameters<SmsParam>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            async {
                let (_, session) = self.session(p.device.as_deref()).await?;
                session.send_sms(p.from.clone(), p.text).await?;
                text(format!("Sent a text message from {}.", p.from))
            }
            .await,
        )
    }

    /// Plays the other end of a phone call: calls the device, or answers,
    /// is busy for, holds, resumes or hangs up a call.
    #[tool(annotations(destructive_hint = false))]
    async fn phone_call(
        &self,
        Parameters(p): Parameters<CallParam>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            async {
                let (_, session) = self.session(p.device.as_deref()).await?;
                let action = match p.action.to_ascii_lowercase().replace([' ', '_'], "-").as_str() {
                    "ring" | "call" => CallAction::Ring,
                    "answer" => CallAction::Answer,
                    "busy" => CallAction::Busy,
                    "hold" => CallAction::Hold,
                    "resume" => CallAction::Resume,
                    "hang-up" | "hangup" | "end" => CallAction::HangUp,
                    other => anyhow::bail!(
                        "\"{other}\" isn't a call action. Use ring, answer, busy, hold, resume or hang-up."
                    ),
                };
                session.phone_call(action, p.number.clone()).await?;
                text(format!("Done: {} with {}.", p.action, p.number))
            }
            .await,
        )
    }

    /// Reads the device's clipboard.
    #[tool(annotations(read_only_hint = true))]
    async fn get_clipboard(
        &self,
        Parameters(p): Parameters<DeviceParam>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            async {
                let (_, session) = self.session(p.device.as_deref()).await?;
                let clipboard = session.device_clipboard().await?;
                text(if clipboard.is_empty() {
                    "The clipboard is empty.".into()
                } else {
                    clipboard
                })
            }
            .await,
        )
    }

    /// Puts text on the device's clipboard.
    #[tool(annotations(destructive_hint = false, idempotent_hint = true))]
    async fn set_clipboard(
        &self,
        Parameters(p): Parameters<TextParam>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            async {
                let (_, session) = self.session(p.device.as_deref()).await?;
                session.set_device_clipboard(p.text).await?;
                text("The clipboard is set.")
            }
            .await,
        )
    }
}

impl AaeServer {
    /// An app's package, from the name people see or the package itself.
    pub(crate) async fn app_package(
        &self,
        session: &std::sync::Arc<aae_ffi::Session>,
        app: &str,
    ) -> anyhow::Result<String> {
        let app = app.trim();
        let apps = session.list_apps(true).await?;
        let lower = app.to_lowercase();
        apps.iter()
            .find(|a| a.package == app)
            .or_else(|| {
                apps.iter()
                    .find(|a| a.label.to_lowercase() == lower && a.launchable)
            })
            .or_else(|| apps.iter().find(|a| a.label.to_lowercase() == lower))
            .map(|a| a.package.clone())
            .ok_or_else(|| {
                anyhow::anyhow!("No app on the device is called \"{app}\". list_apps lists them.")
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(label: &str) -> TouchTarget {
        TouchTarget {
            label: label.into(),
            x: 1,
            y: 2,
            left: 0,
            top: 0,
            right: 2,
            bottom: 4,
        }
    }

    fn by(target: Option<&str>, index: Option<usize>) -> TargetParam {
        TargetParam {
            device: None,
            target: target.map(String::from),
            index,
        }
    }

    #[test]
    fn finds_targets_by_label_exact_match_first() {
        let targets = [target("Settings, button"), target("Settings")];
        assert_eq!(
            find_target(&targets, &by(Some("settings"), None))
                .unwrap()
                .label,
            "Settings"
        );
        assert_eq!(
            find_target(&targets, &by(Some("button"), None))
                .unwrap()
                .label,
            "Settings, button"
        );
        assert_eq!(
            find_target(&targets, &by(None, Some(0))).unwrap().label,
            "Settings, button"
        );
    }

    #[test]
    fn says_what_there_is_when_nothing_matches() {
        let targets = [target("Gmail"), target("Photos")];
        let error = find_target(&targets, &by(Some("Chrome"), None))
            .err()
            .expect("nothing matches")
            .to_string();
        assert!(error.contains("Gmail; Photos"), "{error}");
        assert!(find_target(&targets, &by(None, Some(5))).is_err());
        assert!(find_target(&targets, &by(None, None)).is_err());
    }
}
