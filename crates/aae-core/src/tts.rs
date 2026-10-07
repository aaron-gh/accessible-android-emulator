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
use crate::device::Device;
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

/// How long a speech engine may take to start answering, before AAE decides
/// it's broken. Right after Android starts, and on slower computers, such as
/// Windows PCs running Android for Intel processors, an engine can miss the
/// helper's eight seconds several times; eSpeak NG also unpacks its voices
/// the first time it runs.
const SLOW_ENGINE: Duration = Duration::from_secs(30);
const NEW_ENGINE: Duration = Duration::from_secs(60);

/// True when a failed check may only mean the engine is slow, not broken.
fn maybe_slow(status: &SpeechStatus) -> bool {
    // "none" is a device with no speech engine at all, as plain Android has:
    // waiting won't bring one.
    status.engine != "none"
        && (status.detail.contains("in time") || status.detail.contains("did not start"))
}

/// Checks speech, trying again while the engine seems only slow, for up to
/// `patience`.
async fn check_patiently(adb: &Adb, patience: Duration) -> Result<SpeechStatus> {
    let started = std::time::Instant::now();
    loop {
        let status = check(adb).await;
        let slow = match &status {
            Ok(status) => !status.ok && maybe_slow(status),
            // The helper itself didn't answer, as when Android is busy.
            Err(_) => true,
        };
        if !slow || started.elapsed() >= patience {
            return status;
        }
        if let Ok(status) = &status {
            tracing::info!(
                "speech check: {} is slow ({}); trying again",
                status.engine,
                status.detail
            );
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}

/// Makes sure the device can speak, repairing it if not. Returns what it had
/// to do, or an error if nothing worked.
pub async fn ensure_speech(adb: &Adb) -> Result<SpeechFix> {
    let status = check_patiently(adb, SLOW_ENGINE).await?;
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
        if check_patiently(adb, SLOW_ENGINE).await?.ok {
            return Ok(SpeechFix::ResetGoogleVoices);
        }
    }

    adb.install_own(&espeak_apk()?, ESPEAK_PACKAGE).await?;
    adb.put_setting("secure", "tts_default_synth", ESPEAK_PACKAGE)
        .await?;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let after = check_patiently(adb, NEW_ENGINE).await?;
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

// The speech log.

/// AAE's helper, which is also the speech relay engine.
const RELAY_ENGINE: &str = "io.github.aaron_gh.aae.helper";
const SPEECH_RELAY: &str = "io.github.aaron_gh.aae.helper.SPEECH_RELAY";
const SPEECH_LOG: &str = "io.github.aaron_gh.aae.helper.SPEECH_LOG";

/// One thing the screen reader said.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct Utterance {
    /// When, in milliseconds since 1970.
    pub time: u64,
    pub text: String,
}

/// The device's default speech engine, as Android's settings record it.
async fn default_engine(adb: &Adb) -> Result<Option<String>> {
    adb.setting("secure", "tts_default_synth").await
}

/// Makes AAE's speech relay the default speech engine, passing every request
/// to the real one, recording the text if `log`. Returns the real engine,
/// which the caller keeps to restore later, and what was checked (see
/// below). The screen reader is expected to follow the default engine, as
/// TalkBack and those built on it do; AAE doesn't restart it.
///
/// Speech is then checked through the relay, and the real engine put back if
/// it fails, so the device is never left silent. The check takes seconds,
/// as the relay starts its real engine cold, and once it has passed for a
/// helper version and real engine it keeps passing, so it's skipped when
/// `verified` says it already passed for them.
async fn start_relay(adb: &Adb, log: bool, verified: Option<&str>) -> Result<(String, String)> {
    let current = default_engine(adb).await?;
    let target = match current {
        Some(engine) if engine != RELAY_ENGINE => engine,
        // Already on, or no default recorded: Google's engine, else AAE's eSpeak NG.
        _ if adb.is_installed(GOOGLE_TTS).await? => GOOGLE_TTS.to_string(),
        _ => ESPEAK_PACKAGE.to_string(),
    };
    let out = adb
        .shell(&format!(
            "am broadcast -n {HELPER_RECEIVER} -a {SPEECH_RELAY} --es target {target} --ez log {log}"
        ))
        .await?;
    if !out.contains("result=1") {
        return Err(Error::Adb(format!(
            "AAE's helper couldn't set up the speech log: {}",
            out.trim()
        )));
    }
    adb.put_setting("secure", "tts_default_synth", RELAY_ENGINE)
        .await?;
    let helper = adb
        .version_code("io.github.aaron_gh.aae.helper")
        .await?
        .unwrap_or(0);
    let checked = format!("{helper} {target}");
    if verified == Some(checked.as_str()) {
        return Ok((target, checked));
    }
    let status = check(adb).await?;
    if !status.ok {
        // Never leave the device silent: go back to the real engine.
        adb.put_setting("secure", "tts_default_synth", &target)
            .await?;
        return Err(Error::Adb(format!(
            "Speech didn't work through AAE's speech relay ({}), so it was turned off.",
            status.detail
        )));
    }
    Ok((target, checked))
}

/// Makes `engine` the default speech engine again, instead of the relay.
async fn stop_relay(adb: &Adb, engine: &str) -> Result<()> {
    adb.put_setting("secure", "tts_default_synth", engine).await
}

/// Sets the speech relay's roles: speech log, speech bridge, both or neither.
/// The relay is the default engine while either is on; otherwise the real
/// engine is restored. Saves the device metadata. Needs the current helper.
pub async fn set_relay(
    adb: &Adb,
    device: &mut Device,
    speech_log: bool,
    bridge: bool,
) -> Result<()> {
    match (speech_log || bridge, device.meta.speech_log_engine.clone()) {
        (true, None) => {
            let verified = device.meta.relay_verified.clone();
            let (engine, checked) = start_relay(adb, speech_log, verified.as_deref()).await?;
            device.meta.speech_log_engine = Some(engine);
            device.meta.relay_verified = Some(checked);
        }
        (true, Some(_)) => {
            let out = adb
                .shell(&format!(
                    "am broadcast -n {HELPER_RECEIVER} -a {SPEECH_RELAY} --ez log {speech_log}"
                ))
                .await?;
            if !out.contains("result=1") {
                return Err(Error::Adb(format!(
                    "AAE's helper couldn't change the speech log: {}",
                    out.trim()
                )));
            }
        }
        (false, Some(engine)) => {
            stop_relay(adb, &engine).await?;
            device.meta.speech_log_engine = None;
        }
        (false, None) => {}
    }
    device.meta.speech_log = Some(speech_log);
    device.meta.speech_bridge = bridge;
    device.save_meta()
}

/// What the screen reader said after `since` (milliseconds since 1970).
pub async fn speech_log(adb: &Adb, since: u64, clear: bool) -> Result<Vec<Utterance>> {
    // Android reads the number as a signed 64-bit value; newer versions reject
    // anything larger.
    let since = since.min(i64::MAX as u64);
    let out = adb
        .shell(&format!(
            "am broadcast -n {HELPER_RECEIVER} -a {SPEECH_LOG} --el since {since} --ez clear {clear}"
        ))
        .await?;
    let json = out
        .split_once("data=\"")
        .and_then(|(_, rest)| rest.rsplit_once('"'))
        .map(|(json, _)| json)
        .ok_or_else(|| Error::Adb("AAE's helper didn't send the speech log.".into()))?;
    serde_json::from_str(json)
        .map_err(|e| Error::Adb(format!("The speech log couldn't be read: {e}")))
}

/// A time in milliseconds since 1970 as a local clock time, such as "17:42:06.250".
pub fn clock_time(ms: u64) -> String {
    let millis = ms % 1000;
    if let Some(t) = crate::platform::local_time(ms / 1000) {
        return format!("{:02}:{:02}:{:02}.{millis:03}", t.hour, t.minute, t.second);
    }
    let secs = ms / 1000;
    format!(
        "{:02}:{:02}:{:02}.{millis:03} UTC",
        secs / 3600 % 24,
        secs / 60 % 60,
        secs % 60
    )
}
