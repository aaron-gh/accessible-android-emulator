//! Making sure the device can speak.
//!
//! A screen reader is useless if its speech engine fails, and engines fail
//! quietly: Google's sometimes downloads voices that then won't load, so every
//! request fails while the screen reader's earcons still play. AAE checks the
//! default engine can synthesize speech, repairs it if it can't, and as a last
//! resort installs AAE's own build of eSpeak NG, a small open-source engine
//! that works offline. AAE builds it from source (android/build-espeak.sh), and
//! it unpacks its voice data by itself, so it speaks as soon as it's installed.

use std::path::PathBuf;
use std::time::Duration;

use crate::adb::Adb;
use crate::error::{Error, Result};
use crate::paths;

const HELPER_RECEIVER: &str = "io.github.aaron_gh.aae.helper/.CommandReceiver";
const CHECK_SPEECH: &str = "io.github.aaron_gh.aae.helper.CHECK_SPEECH";
const GOOGLE_TTS: &str = "com.google.android.tts";

/// AAE's build of eSpeak NG.
pub const ESPEAK_PACKAGE: &str = "io.github.aaron_gh.aae.espeak";
const ESPEAK_FILE: &str = "aae-espeak.apk";

/// The result of a speech check.
#[derive(Debug, Clone)]
pub struct SpeechStatus {
    pub ok: bool,
    /// The default engine's package, such as "com.google.android.tts".
    pub engine: String,
    /// What happened, in words.
    pub detail: String,
}

/// What [`ensure_speech`] had to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpeechFix {
    /// Speech already worked.
    None,
    /// Google's engine had broken voice data, which AAE cleared.
    ResetGoogleVoices,
    /// AAE installed eSpeak NG and made it the default engine.
    InstalledEspeak,
}

impl SpeechFix {
    pub fn describe(&self) -> Option<&'static str> {
        match self {
            SpeechFix::None => None,
            SpeechFix::ResetGoogleVoices => Some(
                "Google's speech engine had downloaded voices that don't work. AAE reset it to its built-in voice.",
            ),
            SpeechFix::InstalledEspeak => Some(
                "The device's speech engine didn't work, so AAE installed eSpeak NG and made it the default.",
            ),
        }
    }
}

/// Asks AAE's helper to check that the default speech engine can speak.
pub async fn check(adb: &Adb) -> Result<SpeechStatus> {
    let out = adb
        .shell(&format!(
            "am broadcast -n {HELPER_RECEIVER} -a {CHECK_SPEECH}"
        ))
        .await?;
    let ok = out.contains("result=1");
    let data = out
        .split_once("data=\"")
        .and_then(|(_, rest)| rest.rsplit_once('"'))
        .map(|(data, _)| data)
        .unwrap_or("");
    let (engine, detail) = data.split_once('|').unwrap_or(("", data));
    if !ok && data.is_empty() {
        return Err(Error::Adb(format!(
            "AAE's helper didn't answer the speech check: {}",
            out.trim()
        )));
    }
    Ok(SpeechStatus {
        ok,
        engine: engine.to_string(),
        detail: detail.to_string(),
    })
}

/// Makes sure the device can speak, repairing it if not. Returns what it had
/// to do, or an error if nothing worked.
pub async fn ensure_speech(adb: &Adb) -> Result<SpeechFix> {
    let status = check(adb).await?;
    if status.ok {
        return Ok(SpeechFix::None);
    }
    tracing::warn!(
        "speech check failed with {}: {}",
        status.engine,
        status.detail
    );

    if status.engine == GOOGLE_TTS || adb.is_installed(GOOGLE_TTS).await? {
        adb.shell(&format!("pm clear {GOOGLE_TTS}")).await?;
        tokio::time::sleep(Duration::from_secs(2)).await;
        if check(adb).await?.ok {
            return Ok(SpeechFix::ResetGoogleVoices);
        }
    }

    adb.install(&espeak_apk()?).await?;
    adb.put_setting("secure", "tts_default_synth", ESPEAK_PACKAGE)
        .await?;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let after = check(adb).await?;
    if after.ok {
        Ok(SpeechFix::InstalledEspeak)
    } else {
        Err(Error::Adb(format!(
            "The device can't speak. Even eSpeak NG failed: {}. The screen reader will be silent.",
            after.detail
        )))
    }
}

/// Where AAE looks for its eSpeak NG build: `AAE_ESPEAK_APK`, then next to
/// the program or in the Mac app's Resources folder, then AAE's data folder,
/// then the build output in a source checkout.
pub fn espeak_apk() -> Result<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("AAE_ESPEAK_APK") {
        candidates.push(PathBuf::from(path));
    }
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(PathBuf::from))
    {
        candidates.push(dir.join(ESPEAK_FILE));
        candidates.push(dir.join("../Resources").join(ESPEAK_FILE));
    }
    candidates.push(paths::data_dir().join(ESPEAK_FILE));
    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../android/espeak/build")
            .join(ESPEAK_FILE),
    );
    candidates.into_iter().find(|p| p.is_file()).ok_or_else(|| Error::Apk {
        path: paths::data_dir().join(ESPEAK_FILE),
        reason: "AAE's eSpeak NG was not found. Build it with android/build-espeak.sh, or put it at this path".into(),
    })
}
