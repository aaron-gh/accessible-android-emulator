//! Opening a link on the device, and sending an intent, to test how apps
//! answer them.

use aae_ffi::{IntentExtra, IntentInfo};

use crate::app::{on_ui, say, with_session};
use crate::forms::{self, Field, Form, Role};
use crate::speech::Tone;
use crate::ui;

/// Asks for a link and the app to open it in, then opens it.
pub fn open_link() {
    with_session(|session| async move {
        // The apps that can open links are the ones with screens.
        let mut apps: Vec<_> = session
            .list_apps(false)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|a| a.launchable)
            .collect();
        apps.sort_by_key(|a| a.label.to_lowercase());
        let mut choices = vec!["Whichever app Android chooses".to_string()];
        choices.extend(apps.iter().map(|a| a.label.clone()));
        let answer = on_ui(move || {
            forms::run(
                ui::main_window(),
                Form::new("Open a Link")
                    .field(Field::Edit {
                        label: "Link, such as https://example.com or myapp://settings".into(),
                        value: String::new(),
                    })
                    .field(Field::Choice {
                        label: "Open in".into(),
                        items: choices,
                        selected: 0,
                    })
                    .button("Open", 1, Role::Default)
                    .button("Cancel", 0, Role::Cancel),
            )
        })
        .await;
        let link = answer.values[0].text().trim().to_string();
        if answer.button != 1 || link.is_empty() {
            return Ok(());
        }
        let package = answer.values[1]
            .choice()
            .and_then(|i| i.checked_sub(1))
            .and_then(|i| apps.get(i))
            .map(|a| a.package.clone());
        session.open_link(link.clone(), package).await?;
        say(format!("Opened {link}."), Tone::Info);
        Ok(())
    });
}

/// Asks for an intent, sends it, and says what Android said.
pub fn send_intent() {
    // Asked first, so the device is checked when sending.
    let answer = forms::run(
        ui::main_window(),
        Form::new("Send an Intent")
            .field(Field::Edit {
                label: "Action, such as android.intent.action.VIEW".into(),
                value: String::new(),
            })
            .field(Field::Edit {
                label: "Data, such as a link".into(),
                value: String::new(),
            })
            .field(Field::Edit {
                label: "To: a package, or package/class for a screen or receiver".into(),
                value: String::new(),
            })
            .field(Field::Area {
                label: "Text extras, one key=value on each line".into(),
                value: String::new(),
                read_only: false,
                lines: 4,
            })
            .field(Field::Check {
                label: "Send as a broadcast, not to open a screen".into(),
                checked: false,
            })
            .button("Send", 1, Role::Default)
            .button("Cancel", 0, Role::Cancel),
    );
    if answer.button != 1 {
        return;
    }
    let text = |i: usize| Some(answer.values[i].text()).filter(|t| !t.trim().is_empty());
    let intent = IntentInfo {
        action: text(0),
        data: text(1),
        target: text(2),
        extras: answer.values[3]
            .text()
            .lines()
            .filter_map(|line| {
                let (key, value) = line.split_once('=')?;
                Some(IntentExtra {
                    key: key.trim().to_string(),
                    value: value.to_string(),
                })
            })
            .collect(),
        broadcast: answer.values[4].checked(),
    };
    if intent.action.is_none() && intent.data.is_none() && intent.target.is_none() {
        crate::app::announce(
            "Nothing sent: give an action, data, or where to send it.",
            Tone::Failure,
        );
        return;
    }
    with_session(|session| async move {
        let said = session.send_intent(intent).await?;
        say(
            if said.is_empty() {
                "Sent.".into()
            } else {
                said
            },
            Tone::Success,
        );
        Ok(())
    });
}
