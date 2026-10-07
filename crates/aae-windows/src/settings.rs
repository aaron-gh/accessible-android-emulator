//! The Windows app's settings, kept in a file in AAE's data folder.

use std::sync::Mutex;

use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Play a sound before each announcement.
    pub play_sounds: bool,
    /// When several devices are running, play only the one in use.
    pub play_only_in_use: bool,
    /// Raise the pitch of older Android versions, which play slow.
    pub correct_pitch: bool,
    /// When AAE last said tool updates are available, in seconds since 1970.
    pub last_update_mention: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            play_sounds: true,
            play_only_in_use: true,
            correct_pitch: true,
            last_update_mention: 0,
        }
    }
}

static SETTINGS: Mutex<Option<Settings>> = Mutex::new(None);

fn path() -> std::path::PathBuf {
    aae_core::paths::data_dir().join("windows-app.json")
}

pub fn get() -> Settings {
    let mut settings = SETTINGS.lock().unwrap();
    settings
        .get_or_insert_with(|| {
            std::fs::read(path())
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .unwrap_or_default()
        })
        .clone()
}

/// Changes the settings and saves them.
pub fn change(edit: impl FnOnce(&mut Settings)) {
    let mut current = get();
    edit(&mut current);
    let _ = std::fs::create_dir_all(aae_core::paths::data_dir());
    if let Ok(json) = serde_json::to_vec_pretty(&current) {
        let _ = std::fs::write(path(), json);
    }
    *SETTINGS.lock().unwrap() = Some(current);
}
