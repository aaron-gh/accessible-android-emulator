//! Recording the device's screen with its sound, through the emulator's own
//! recorder, as a WebM file: VP9 video and Vorbis sound. The emulator stops a
//! recording by itself after three minutes.

use std::path::{Path, PathBuf};

use crate::adb::Adb;
use crate::error::{Error, Result};

/// The longest recording the emulator makes, in seconds.
pub const LONGEST: u32 = 180;

/// The file a recording goes to: the path given, ending in .webm, which is
/// the only kind the emulator makes.
pub fn file_for(path: &Path) -> PathBuf {
    match path.extension().and_then(|e| e.to_str()) {
        Some(e) if e.eq_ignore_ascii_case("webm") => path.to_path_buf(),
        _ => path.with_extension("webm"),
    }
}

/// Starts recording into `path` (see [`file_for`]), for at most `seconds`,
/// up to [`LONGEST`]. Returns the file it records into.
pub async fn start(adb: &Adb, path: &Path, seconds: u32) -> Result<PathBuf> {
    let file = file_for(path);
    if file.to_string_lossy().contains('"') {
        return Err(Error::Message(
            "The recording's file name can't have a double quote in it.".into(),
        ));
    }
    if let Some(folder) = file.parent()
        && !folder.as_os_str().is_empty()
        && !folder.is_dir()
    {
        return Err(Error::Message(format!(
            "There's no folder {} to record into.",
            folder.display()
        )));
    }
    let limit = seconds.clamp(1, LONGEST).to_string();
    // The console splits on spaces, unless the path is in quotes.
    let quoted = format!("\"{}\"", file.display());
    let out = adb
        .raw(&[
            "emu",
            "screenrecord",
            "start",
            "--time-limit",
            &limit,
            &quoted,
        ])
        .await?;
    if out.contains("KO") {
        return Err(Error::Message(format!(
            "The emulator wouldn't record: {}",
            out.trim().trim_start_matches("KO:").trim()
        )));
    }
    Ok(file)
}

/// Stops recording. Saying "already stopped" isn't an error: a recording
/// stops by itself at its time limit.
pub async fn stop(adb: &Adb) -> Result<()> {
    let out = adb.raw(&["emu", "screenrecord", "stop"]).await?;
    if out.contains("KO") && !out.contains("already") {
        return Err(Error::Message(format!(
            "The emulator couldn't stop recording: {}",
            out.trim()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recordings_are_webm_files() {
        assert_eq!(file_for(Path::new("/a/b.mp4")), PathBuf::from("/a/b.webm"));
        assert_eq!(file_for(Path::new("/a/b")), PathBuf::from("/a/b.webm"));
        assert_eq!(file_for(Path::new("/a/b.WEBM")), PathBuf::from("/a/b.WEBM"));
    }
}
