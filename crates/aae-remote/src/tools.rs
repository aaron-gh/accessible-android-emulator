//! The testing tools, for the phone: the speech log, the device log, the
//! accessibility inspector, apps, accessibility services, snapshots, the
//! battery, location and phone, links and intents, and the clipboard. Each
//! acts on a running device, given as "id".

use std::sync::Arc;

use aae_ffi::{
    AaeError, AccessKind, AppChoice, AppPartInfo, AppPartKind, CallAction, IntentExtra, IntentInfo,
    LogFilter, LogLevel, Session,
};
use serde_json::{Value, json};

use crate::server::Server;

fn failed(message: impl Into<String>) -> AaeError {
    AaeError::Failed {
        message: message.into(),
    }
}

fn text(params: &Value, key: &str) -> Result<String, AaeError> {
    params[key]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| failed(format!("The request needs \"{key}\".")))
}

fn optional(params: &Value, key: &str) -> Option<String> {
    params[key]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn level(name: &str) -> Option<LogLevel> {
    Some(match name {
        "verbose" => LogLevel::Verbose,
        "debug" => LogLevel::Debug,
        "info" => LogLevel::Info,
        "warning" => LogLevel::Warning,
        "error" => LogLevel::Error,
        "fatal" => LogLevel::Fatal,
        _ => return None,
    })
}

fn level_name(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Verbose => "verbose",
        LogLevel::Debug => "debug",
        LogLevel::Info => "info",
        LogLevel::Warning => "warning",
        LogLevel::Error => "error",
        LogLevel::Fatal => "fatal",
    }
}

fn access_kind(name: &str) -> Option<AccessKind> {
    Some(match name {
        "battery" => AccessKind::Battery,
        "overlay" => AccessKind::Overlay,
        "usage" => AccessKind::Usage,
        "write-settings" => AccessKind::WriteSettings,
        _ => return None,
    })
}

fn access_name(kind: AccessKind) -> &'static str {
    match kind {
        AccessKind::Battery => "battery",
        AccessKind::Overlay => "overlay",
        AccessKind::Usage => "usage",
        AccessKind::WriteSettings => "write-settings",
    }
}

fn part_kind(kind: AppPartKind) -> &'static str {
    match kind {
        AppPartKind::AccessibilityService => "accessibility-service",
        AppPartKind::Keyboard => "keyboard",
        AppPartKind::NotificationListener => "notification-listener",
        AppPartKind::DeviceAdministrator => "device-administrator",
    }
}

fn part_kind_named(name: &str) -> Option<AppPartKind> {
    Some(match name {
        "accessibility-service" => AppPartKind::AccessibilityService,
        "keyboard" => AppPartKind::Keyboard,
        "notification-listener" => AppPartKind::NotificationListener,
        "device-administrator" => AppPartKind::DeviceAdministrator,
        _ => return None,
    })
}

/// A part of an installed app that needs the person's say, such as an
/// accessibility service.
pub fn part(p: &AppPartInfo) -> Value {
    json!({
        "component": p.component,
        "name": p.name,
        "part_kind": part_kind(p.kind),
        "kind": p.kind_description,
        "choice": p.choice,
    })
}

fn network(info: &aae_ffi::NetworkInfo) -> Value {
    json!({
        "airplane": info.airplane, "wifi": info.wifi, "data": info.data,
        "speed": info.speed, "description": info.description,
    })
}

/// Runs a testing tool's method, or returns None if it isn't one.
pub async fn call(
    server: &Arc<Server>,
    method: &str,
    params: &Value,
) -> Option<Result<Value, AaeError>> {
    if !method.starts_with("tools.") {
        return None;
    }
    let result = async {
        let session: Arc<Session> = server.session(&text(params, "id")?).await?;
        Ok::<Value, AaeError>(match method {
            // The speech log.
            "tools.speech_log.set" => json!(session.set_speech_log(params["on"].as_bool().unwrap_or(false)).await?),
            "tools.speech_log" => {
                let since = params["since"].as_u64().unwrap_or(0);
                let said = session.speech_log(since, params["clear"].as_bool().unwrap_or(false)).await?;
                json!({
                    "on": session.speech_log_on(),
                    "said": said.iter().map(|u| json!({"time": u.time, "clock": u.clock, "text": u.text})).collect::<Vec<_>>(),
                })
            }

            // The device log, read while the phone asks for it.
            "tools.log" => {
                session.start_logs();
                let filter = LogFilter {
                    process: optional(params, "app"),
                    tag: optional(params, "tag"),
                    level: params["level"].as_str().and_then(level),
                    text: optional(params, "search"),
                };
                let since = params["since"].as_u64().unwrap_or(0);
                let limit = params["limit"].as_u64().unwrap_or(500) as u32;
                let entries = session.log_entries(since, filter, limit);
                json!({
                    "latest": session.log_latest(),
                    "problem": session.log_problem(),
                    "processes": session.log_processes(),
                    "entries": entries.iter().map(|e| json!({
                        "seq": e.seq,
                        "level": level_name(e.level),
                        "spoken": e.spoken,
                        "line": e.line,
                        "process": e.process,
                        "clock": e.clock,
                    })).collect::<Vec<_>>(),
                })
            }
            "tools.log.stop" => {
                session.stop_logs();
                Value::Null
            }

            // The accessibility inspector.
            "tools.inspect" => {
                let inspection = session.inspect().await?;
                json!({
                    "rows": inspection.rows.iter().map(|r| json!({
                        "index": r.index, "parent": r.parent, "summary": r.summary, "details": r.details,
                    })).collect::<Vec<_>>(),
                    "issues": inspection.issues.iter().map(|i| json!({
                        "error": i.error, "message": i.message, "element": i.element, "id": i.id,
                    })).collect::<Vec<_>>(),
                    "text": inspection.text,
                })
            }

            // Apps.
            "tools.apps" => json!(
                session
                    .list_apps(params["system"].as_bool().unwrap_or(false))
                    .await?
                    .iter()
                    .map(|a| json!({
                        "package": a.package, "label": a.label, "version": a.version,
                        "system": a.system, "enabled": a.enabled, "launchable": a.launchable,
                    }))
                    .collect::<Vec<_>>()
            ),
            "tools.app.open" => {
                session.open_app(text(params, "package")?).await?;
                Value::Null
            }
            "tools.app.stop" => {
                session.force_stop_app(text(params, "package")?).await?;
                Value::Null
            }
            "tools.app.clear" => {
                session.clear_app_data(text(params, "package")?).await?;
                Value::Null
            }
            "tools.app.uninstall" => {
                session.uninstall_app(text(params, "package")?).await?;
                Value::Null
            }
            "tools.app.permissions" => {
                let found = session.app_permissions(text(params, "package")?).await?;
                json!({
                    "permissions": found.permissions.iter().map(|p| json!({
                        "name": p.name, "label": p.label, "granted": p.granted,
                    })).collect::<Vec<_>>(),
                    "access": found.access.iter().map(|a| json!({
                        "kind": access_name(a.kind), "name": a.name, "allowed": a.allowed,
                    })).collect::<Vec<_>>(),
                })
            }
            "tools.app.permission" => {
                session
                    .set_app_permission(
                        text(params, "package")?,
                        text(params, "permission")?,
                        params["granted"].as_bool().unwrap_or(false),
                    )
                    .await?;
                Value::Null
            }
            "tools.battery.health" => json!(session.battery_health().await?),
            "tools.battery.health.set" => {
                json!(session.set_battery_health(text(params, "health")?).await?)
            }
            "tools.battery.healths" => json!(
                aae_ffi::battery_healths()
                    .into_iter()
                    .map(|h| json!({"name": h.name, "label": h.label}))
                    .collect::<Vec<_>>()
            ),
            "tools.fingerprint" => json!(
                session
                    .touch_fingerprint(params["finger"].as_u64().unwrap_or(1) as u32)
                    .await?
            ),
            "tools.settings" => json!(
                session
                    .device_settings()
                    .await?
                    .into_iter()
                    .map(|s| json!({
                        "name": s.name,
                        "label": s.label,
                        "value": s.value,
                        "choices": s.choices.iter().map(|c| json!({"value": c.value, "label": c.label})).collect::<Vec<_>>(),
                    }))
                    .collect::<Vec<_>>()
            ),
            "tools.settings.for_new_devices" => json!(session.use_settings_for_new_devices().await?),
            "tools.settings.set" => json!(
                session
                    .change_device_setting(text(params, "name")?, text(params, "value")?)
                    .await?
            ),
            "tools.shake" => {
                session.shake().await?;
                Value::Null
            }
            "tools.app.grant_all" => json!(session.grant_all_permissions(text(params, "package")?).await?),
            "tools.app.access" => {
                let kind = access_kind(&text(params, "kind")?).ok_or_else(|| failed("Unknown special access."))?;
                session
                    .set_app_access(text(params, "package")?, kind, params["allowed"].as_bool().unwrap_or(false))
                    .await?;
                Value::Null
            }
            "tools.app.choices" => {
                // Parts the phone asked about after installing: on or off.
                let choices = params["choices"]
                    .as_array()
                    .map(|list| {
                        list.iter()
                            .filter_map(|c| {
                                Some(AppChoice {
                                    part: AppPartInfo {
                                        kind: part_kind_named(c["part_kind"].as_str()?)?,
                                        component: c["component"].as_str()?.to_string(),
                                        name: c["name"].as_str().unwrap_or("").to_string(),
                                        kind_description: c["kind"].as_str().unwrap_or("").to_string(),
                                        choice: None,
                                    },
                                    on: c["on"].as_bool().unwrap_or(false),
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                session.set_app_choices(choices).await?;
                Value::Null
            }

            // Accessibility services.
            "tools.services" => json!(
                session
                    .list_services()
                    .await?
                    .iter()
                    .map(|s| json!({
                        "component": s.component, "label": s.label, "description": s.description,
                        "screen_reader": s.screen_reader, "on": s.on,
                    }))
                    .collect::<Vec<_>>()
            ),
            "tools.service" => {
                session
                    .set_service(text(params, "component")?, params["on"].as_bool().unwrap_or(false))
                    .await?;
                Value::Null
            }

            // Snapshots.
            "tools.snapshots" => json!(
                session
                    .snapshots()
                    .await?
                    .iter()
                    .map(|s| json!({
                        "id": s.id, "name": s.name, "notes": s.notes, "taken": s.taken,
                        "size": s.size, "loaded": s.loaded, "compatible": s.compatible,
                    }))
                    .collect::<Vec<_>>()
            ),
            "tools.snapshot.save" => {
                session
                    .save_snapshot(text(params, "name")?, optional(params, "notes").unwrap_or_default())
                    .await?;
                Value::Null
            }
            "tools.snapshot.load" => {
                session.load_snapshot(text(params, "snapshot")?).await?;
                Value::Null
            }
            "tools.snapshot.delete" => {
                session.delete_snapshot(text(params, "snapshot")?).await?;
                Value::Null
            }

            // Battery, location and phone.
            "tools.battery" => {
                session
                    .set_battery(
                        params["level"].as_u64().unwrap_or(100).min(100) as u32,
                        params["charging"].as_bool().unwrap_or(true),
                    )
                    .await?;
                Value::Null
            }
            "tools.location" => {
                // Latitude and longitude, or a place, looked up on OpenStreetMap.
                let place = text(params, "place")?;
                let (latitude, longitude, name) = match aae_core::geocode::coordinates(&place) {
                    Some((lat, lon)) => (lat, lon, None),
                    None => {
                        let query = place.clone();
                        let found = tokio::task::spawn_blocking(move || aae_core::geocode::look_up(&query))
                            .await
                            .map_err(|e| failed(e.to_string()))?
                            .map_err(AaeError::from)?
                            .ok_or_else(|| failed(format!("Couldn't find {place}. Try an address, or latitude and longitude.")))?;
                        (found.latitude, found.longitude, Some(found.name))
                    }
                };
                session.set_location(latitude, longitude).await?;
                json!({"latitude": latitude, "longitude": longitude, "place": name})
            }
            "tools.sms" => {
                let from = optional(params, "from").unwrap_or_else(|| "5551234".into());
                session.send_sms(from, text(params, "text")?).await?;
                Value::Null
            }
            "tools.call" => {
                let action = match text(params, "action")?.as_str() {
                    "ring" => CallAction::Ring,
                    "hang-up" => CallAction::HangUp,
                    "answer" => CallAction::Answer,
                    "busy" => CallAction::Busy,
                    "hold" => CallAction::Hold,
                    "resume" => CallAction::Resume,
                    other => return Err(failed(format!("Unknown call action {other}."))),
                };
                let number = optional(params, "number").unwrap_or_else(|| "5551234".into());
                session.phone_call(action, number).await?;
                Value::Null
            }

            "tools.screen_changes" => match session.screen_changes().await? {
                Some(c) => json!({"count": c.count, "quiet_ms": c.quiet_ms}),
                None => Value::Null,
            },

            // The network.
            "tools.network" => network(&session.network().await?),
            "tools.microphone.stop" => json!(session.stop_playing_into_microphone()),
            "tools.route.stop" => json!(session.stop_route()),
            "tools.network.set" => {
                if let Some(on) = params["airplane"].as_bool() {
                    session.set_airplane_mode(on).await?;
                }
                if let Some(on) = params["wifi"].as_bool() {
                    session.set_wifi(on).await?;
                }
                if let Some(on) = params["data"].as_bool() {
                    session.set_mobile_data(on).await?;
                }
                if let Some(speed) = optional(params, "speed") {
                    session.set_network_speed(speed).await?;
                }
                // Android takes a moment to report changes.
                tokio::time::sleep(std::time::Duration::from_millis(800)).await;
                network(&session.network().await?)
            }
            "tools.network.speeds" => json!(
                aae_ffi::network_speeds()
                    .iter()
                    .map(|s| json!({"name": s.name, "description": s.description}))
                    .collect::<Vec<_>>()
            ),

            // Links, intents and the clipboard.
            "tools.link" => {
                session.open_link(text(params, "link")?, optional(params, "package")).await?;
                Value::Null
            }
            "tools.intent" => {
                let extras = params["extras"]
                    .as_object()
                    .map(|map| {
                        map.iter()
                            .map(|(k, v)| IntentExtra {
                                key: k.clone(),
                                value: v.as_str().unwrap_or("").to_string(),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                json!(
                    session
                        .send_intent(IntentInfo {
                            action: optional(params, "action"),
                            data: optional(params, "data"),
                            target: optional(params, "target"),
                            extras,
                            broadcast: params["broadcast"].as_bool().unwrap_or(false),
                        })
                        .await?
                )
            }
            "tools.clipboard" => json!(session.device_clipboard().await?),
            "tools.clipboard.set" => {
                session.set_device_clipboard(text(params, "text")?).await?;
                Value::Null
            }
            "tools.type" => {
                session.type_text(text(params, "text")?).await?;
                Value::Null
            }
            "tools.screen_text" => json!(session.screen_text().await?),

            _ => return Err(failed(format!("This computer's AAE doesn't know \"{method}\". Update AAE on it."))),
        })
    }
    .await;
    Some(result)
}
