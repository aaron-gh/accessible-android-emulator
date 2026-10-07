//! The device's language, display and accessibility settings, changed
//! without going through Settings: the language, font and display size, dark
//! theme, bold and high-contrast text, colour inversion and correction,
//! animations, captions, and the touch and hold delay.
//!
//! Each is a [`Setting`] with a fixed list of choices, so the apps can show
//! them all the same way; the language also takes any language tag.

use crate::adb::Adb;
use crate::error::{Error, Result};

/// One setting: its name for commands, its label, and its choices, each a
/// value for commands and a label.
pub struct Setting {
    pub name: &'static str,
    pub label: &'static str,
    pub choices: &'static [(&'static str, &'static str)],
}

const ON_OFF: &[(&str, &str)] = &[("on", "On"), ("off", "Off")];

pub const LANGUAGE: &str = "language";
pub const FONT_SIZE: &str = "font-size";
pub const DISPLAY_SIZE: &str = "display-size";
pub const DARK_THEME: &str = "dark-theme";
pub const BOLD_TEXT: &str = "bold-text";
pub const HIGH_CONTRAST_TEXT: &str = "high-contrast-text";
pub const COLOUR_INVERSION: &str = "colour-inversion";
pub const COLOUR_CORRECTION: &str = "colour-correction";
pub const ANIMATIONS: &str = "animations";
pub const CAPTIONS: &str = "captions";
pub const TOUCH_AND_HOLD: &str = "touch-and-hold";

pub const SETTINGS: &[Setting] = &[
    Setting {
        name: LANGUAGE,
        label: "Language",
        choices: &[
            ("en-US", "English (United States)"),
            ("en-GB", "English (United Kingdom)"),
            ("fr-FR", "French (France)"),
            ("de-DE", "German"),
            ("es-ES", "Spanish (Spain)"),
            ("es-419", "Spanish (Latin America)"),
            ("it-IT", "Italian"),
            ("pt-BR", "Portuguese (Brazil)"),
            ("nl-NL", "Dutch"),
            ("sv-SE", "Swedish"),
            ("pl-PL", "Polish"),
            ("ru-RU", "Russian"),
            ("tr-TR", "Turkish"),
            ("ar-EG", "Arabic, right to left"),
            ("he-IL", "Hebrew, right to left"),
            ("hi-IN", "Hindi"),
            ("ja-JP", "Japanese"),
            ("ko-KR", "Korean"),
            ("zh-CN", "Chinese (Simplified)"),
            ("zh-TW", "Chinese (Traditional)"),
            ("en-XA", "Pseudo-locale, accented and longer"),
            ("ar-XB", "Pseudo-locale, right to left"),
        ],
    },
    Setting {
        name: FONT_SIZE,
        label: "Font size",
        choices: &[
            ("85", "85 percent"),
            ("100", "100 percent, the default"),
            ("115", "115 percent"),
            ("130", "130 percent"),
            ("150", "150 percent"),
            ("180", "180 percent"),
            ("200", "200 percent"),
        ],
    },
    Setting {
        name: DISPLAY_SIZE,
        label: "Display size",
        choices: &[
            ("small", "Small"),
            ("default", "Default"),
            ("large", "Large"),
            ("larger", "Larger"),
            ("largest", "Largest"),
        ],
    },
    Setting {
        name: DARK_THEME,
        label: "Dark theme",
        choices: ON_OFF,
    },
    Setting {
        name: BOLD_TEXT,
        label: "Bold text",
        choices: ON_OFF,
    },
    Setting {
        name: HIGH_CONTRAST_TEXT,
        label: "High contrast text",
        choices: ON_OFF,
    },
    Setting {
        name: COLOUR_INVERSION,
        label: "Colour inversion",
        choices: ON_OFF,
    },
    Setting {
        name: COLOUR_CORRECTION,
        label: "Colour correction",
        choices: &[
            ("off", "Off"),
            ("deuteranomaly", "Red-green, green weak (deuteranomaly)"),
            ("protanomaly", "Red-green, red weak (protanomaly)"),
            ("tritanomaly", "Blue-yellow (tritanomaly)"),
            ("grayscale", "Greyscale"),
        ],
    },
    Setting {
        name: ANIMATIONS,
        label: "Animations",
        choices: ON_OFF,
    },
    Setting {
        name: CAPTIONS,
        label: "Captions",
        choices: ON_OFF,
    },
    Setting {
        name: TOUCH_AND_HOLD,
        label: "Touch and hold delay",
        choices: &[("short", "Short"), ("medium", "Medium"), ("long", "Long")],
    },
];

pub fn setting(name: &str) -> Option<&'static Setting> {
    let name = name
        .trim()
        .to_lowercase()
        .replace([' ', '_'], "-")
        .replace("color", "colour");
    SETTINGS.iter().find(|s| s.name == name)
}

/// Display sizes, as fractions of the screen's own density.
const DISPLAY_SIZES: &[(&str, f64)] = &[
    ("small", 0.85),
    ("default", 1.0),
    ("large", 1.1),
    ("larger", 1.2),
    ("largest", 1.3),
];

const TOUCH_AND_HOLD_MS: &[(&str, u32)] = &[("short", 400), ("medium", 1000), ("long", 1500)];

const DALTONIZER: &[(&str, i32)] = &[
    ("grayscale", 0),
    ("protanomaly", 11),
    ("deuteranomaly", 12),
    ("tritanomaly", 13),
];

const LOCALE_TOOL: &str = "CLASSPATH=$(pm path io.github.aaron_gh.aae.helper | cut -d: -f2) \
     app_process / io.github.aaron_gh.aae.helper.ShellTool locale";

/// Each setting's value now, by name, as one of its choices' values where
/// it is one. A setting the device doesn't have, such as dark theme before
/// Android 10, is left out.
pub async fn read(adb: &Adb) -> Result<Vec<(&'static str, String)>> {
    // One trip to the device for everything but the language.
    let script = [
        "echo font=$(settings get system font_scale)",
        "echo density=$(wm density | tr '\\n' ' ')",
        "echo night=$(cmd uimode night 2>/dev/null)",
        "echo bold=$(settings get secure font_weight_adjustment)",
        "echo contrast=$(settings get secure high_text_contrast_enabled)",
        "echo inversion=$(settings get secure accessibility_display_inversion_enabled)",
        "echo daltonizer=$(settings get secure accessibility_display_daltonizer_enabled)",
        "echo daltonizer_mode=$(settings get secure accessibility_display_daltonizer)",
        "echo animator=$(settings get global animator_duration_scale)",
        "echo captions=$(settings get secure accessibility_captioning_enabled)",
        "echo long_press=$(settings get secure long_press_timeout)",
        "echo sdk=$(getprop ro.build.version.sdk)",
    ]
    .join("; ");
    let out = adb.shell(&script).await?;
    let get = |key: &str| {
        out.lines()
            .find_map(|l| l.strip_prefix(&format!("{key}=")))
            .map(str::trim)
            .filter(|v| !v.is_empty() && *v != "null")
            .map(str::to_string)
    };
    let on = |key: &str| {
        if get(key).as_deref() == Some("1") {
            "on"
        } else {
            "off"
        }
        .to_string()
    };
    let sdk: u32 = get("sdk").and_then(|s| s.parse().ok()).unwrap_or(0);
    let mut values = Vec::new();

    if let Ok(languages) = adb.shell(LOCALE_TOOL).await
        && !languages.contains("failed")
        && !languages.trim().is_empty()
    {
        values.push((LANGUAGE, languages.trim().to_string()));
    }

    let font: f64 = get("font").and_then(|f| f.parse().ok()).unwrap_or(1.0);
    values.push((FONT_SIZE, format!("{}", (font * 100.0).round() as u32)));

    if let Some(density) = get("density") {
        let number = |label: &str| {
            density
                .split(label)
                .nth(1)
                .and_then(|rest| rest.split_whitespace().next())
                .and_then(|n| n.parse::<f64>().ok())
        };
        if let Some(physical) = number("Physical density:") {
            let now = number("Override density:").unwrap_or(physical);
            let size = DISPLAY_SIZES
                .iter()
                .find(|(_, f)| ((physical * f).round() - now).abs() < 1.5)
                .map_or(format!("{now} dpi"), |(name, _)| name.to_string());
            values.push((DISPLAY_SIZE, size));
        }
    }

    if sdk >= 29 {
        let night = get("night").unwrap_or_default().to_lowercase();
        values.push((
            DARK_THEME,
            if night.contains("yes") { "on" } else { "off" }.into(),
        ));
    }
    if sdk >= 31 {
        let bold = get("bold").and_then(|b| b.parse::<i32>().ok()).unwrap_or(0);
        values.push((BOLD_TEXT, if bold > 0 { "on" } else { "off" }.into()));
    }
    values.push((HIGH_CONTRAST_TEXT, on("contrast")));
    values.push((COLOUR_INVERSION, on("inversion")));
    let correction = if get("daltonizer").as_deref() == Some("1") {
        let mode: i32 = get("daltonizer_mode")
            .and_then(|m| m.parse().ok())
            .unwrap_or(12);
        DALTONIZER
            .iter()
            .find(|(_, m)| *m == mode)
            .map_or("deuteranomaly", |(name, _)| name)
    } else {
        "off"
    };
    values.push((COLOUR_CORRECTION, correction.into()));
    let animator: f64 = get("animator").and_then(|a| a.parse().ok()).unwrap_or(1.0);
    values.push((
        ANIMATIONS,
        if animator == 0.0 { "off" } else { "on" }.into(),
    ));
    values.push((CAPTIONS, on("captions")));
    let long_press: u32 = get("long_press")
        .and_then(|l| l.parse().ok())
        .unwrap_or(400);
    let touch = TOUCH_AND_HOLD_MS
        .iter()
        .min_by_key(|(_, ms)| ms.abs_diff(long_press))
        .map_or("short", |(name, _)| name);
    values.push((TOUCH_AND_HOLD, touch.into()));
    Ok(values)
}

/// Changes a setting, by name, to a value: one of its choices, or for the
/// language, any language tags, such as "fr-CA" or "fr-FR,en-US". Returns
/// what to say.
pub async fn change(adb: &Adb, name: &str, value: &str) -> Result<String> {
    let setting = setting(name).ok_or_else(|| {
        Error::Message(format!(
            "\"{name}\" isn't a setting AAE changes. They are: {}.",
            SETTINGS
                .iter()
                .map(|s| s.name)
                .collect::<Vec<_>>()
                .join(", ")
        ))
    })?;
    let value = value.trim();
    let choice = setting
        .choices
        .iter()
        .find(|(v, l)| v.eq_ignore_ascii_case(value) || l.eq_ignore_ascii_case(value));
    let wrong = || {
        Error::Message(format!(
            "{} is one of: {}.",
            setting.label,
            setting
                .choices
                .iter()
                .map(|(v, _)| *v)
                .collect::<Vec<_>>()
                .join(", ")
        ))
    };
    let on = |v: &str| match v.to_lowercase().as_str() {
        "on" | "yes" | "true" | "1" => Some(true),
        "off" | "no" | "false" | "0" => Some(false),
        _ => None,
    };
    let said_label = choice.map_or(value.to_string(), |(_, l)| l.to_string());
    // Settings newer than the device's Android.
    let needs = match setting.name {
        DARK_THEME => Some((29, "Android 10")),
        BOLD_TEXT => Some((31, "Android 12")),
        _ => None,
    };
    if let Some((api, version)) = needs {
        let sdk: u32 = adb
            .shell("getprop ro.build.version.sdk")
            .await?
            .trim()
            .parse()
            .unwrap_or(0);
        if sdk < api {
            return Err(Error::Message(format!(
                "{} came in {version}, so this device doesn't have it.",
                setting.label
            )));
        }
    }
    match setting.name {
        LANGUAGE => {
            let tags = choice.map_or(value, |(v, _)| v);
            let out = adb
                .shell(&format!("{LOCALE_TOOL} {}", crate::adb::shell_quote(tags)))
                .await?;
            if out.contains("failed") || out.trim().is_empty() {
                return Err(Error::Message(format!(
                    "The device's language couldn't be changed: {}",
                    out.trim()
                )));
            }
        }
        FONT_SIZE => {
            let percent: f64 = value.trim_end_matches('%').parse().map_err(|_| wrong())?;
            if !(50.0..=300.0).contains(&percent) {
                return Err(wrong());
            }
            adb.put_setting("system", "font_scale", &format!("{}", percent / 100.0))
                .await?;
            return Ok(format!("Font size: {percent} percent."));
        }
        DISPLAY_SIZE => {
            let (_, factor) = DISPLAY_SIZES
                .iter()
                .find(|(n, _)| Some(*n) == choice.map(|c| c.0))
                .ok_or_else(wrong)?;
            if *factor == 1.0 {
                adb.shell("wm density reset").await?;
            } else {
                let out = adb.shell("wm density").await?;
                let physical: f64 = out
                    .split("Physical density:")
                    .nth(1)
                    .and_then(|r| r.split_whitespace().next())
                    .and_then(|n| n.parse().ok())
                    .ok_or_else(|| {
                        Error::Message(format!("The screen's density is unknown: {out}"))
                    })?;
                adb.shell(&format!(
                    "wm density {}",
                    (physical * factor).round() as u32
                ))
                .await?;
            }
        }
        DARK_THEME => {
            let on = on(value).ok_or_else(wrong)?;
            let out = adb
                .shell(&format!(
                    "cmd uimode night {}",
                    if on { "yes" } else { "no" }
                ))
                .await?;
            if out.to_lowercase().contains("unknown") || out.to_lowercase().contains("error") {
                return Err(Error::Message(
                    "This Android version has no dark theme; it came in Android 10.".into(),
                ));
            }
        }
        BOLD_TEXT => {
            let on = on(value).ok_or_else(wrong)?;
            adb.put_setting(
                "secure",
                "font_weight_adjustment",
                if on { "300" } else { "0" },
            )
            .await?;
        }
        HIGH_CONTRAST_TEXT => {
            let on = on(value).ok_or_else(wrong)?;
            adb.put_setting(
                "secure",
                "high_text_contrast_enabled",
                if on { "1" } else { "0" },
            )
            .await?;
        }
        COLOUR_INVERSION => {
            let on = on(value).ok_or_else(wrong)?;
            adb.put_setting(
                "secure",
                "accessibility_display_inversion_enabled",
                if on { "1" } else { "0" },
            )
            .await?;
        }
        COLOUR_CORRECTION => {
            let name = choice.map(|c| c.0).ok_or_else(wrong)?;
            match DALTONIZER.iter().find(|(n, _)| *n == name) {
                Some((_, mode)) => {
                    adb.put_setting(
                        "secure",
                        "accessibility_display_daltonizer",
                        &mode.to_string(),
                    )
                    .await?;
                    adb.put_setting("secure", "accessibility_display_daltonizer_enabled", "1")
                        .await?;
                }
                None => {
                    adb.put_setting("secure", "accessibility_display_daltonizer_enabled", "0")
                        .await?;
                }
            }
        }
        ANIMATIONS => {
            let on = on(value).ok_or_else(wrong)?;
            for key in [
                "window_animation_scale",
                "transition_animation_scale",
                "animator_duration_scale",
            ] {
                adb.put_setting("global", key, if on { "1" } else { "0" })
                    .await?;
            }
        }
        CAPTIONS => {
            let on = on(value).ok_or_else(wrong)?;
            adb.put_setting(
                "secure",
                "accessibility_captioning_enabled",
                if on { "1" } else { "0" },
            )
            .await?;
        }
        TOUCH_AND_HOLD => {
            let name = choice.map(|c| c.0).ok_or_else(wrong)?;
            let (_, ms) = TOUCH_AND_HOLD_MS
                .iter()
                .find(|(n, _)| *n == name)
                .ok_or_else(wrong)?;
            adb.put_setting("secure", "long_press_timeout", &ms.to_string())
                .await?;
        }
        _ => return Err(wrong()),
    }
    Ok(format!("{}: {said_label}.", setting.label))
}

/// Says a setting's value: the choice's label, or the value itself.
pub fn label_of(name: &str, value: &str) -> String {
    setting(name)
        .and_then(|s| s.choices.iter().find(|(v, _)| *v == value))
        .map_or(value.to_string(), |(_, l)| l.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_are_found_by_name_however_written() {
        assert_eq!(setting("Color Correction").unwrap().name, COLOUR_CORRECTION);
        assert_eq!(setting("font_size").unwrap().name, FONT_SIZE);
        assert!(setting("brightness").is_none());
    }

    #[test]
    fn every_setting_has_choices() {
        for s in SETTINGS {
            assert!(!s.choices.is_empty(), "{}", s.name);
        }
    }
}
