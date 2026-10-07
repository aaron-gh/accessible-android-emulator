//! Watching an app's build output, so a new build can be installed as soon
//! as it's made. Looks every second; no file system events, so it works the
//! same on every platform and for any build tool.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// The newest APK at a path: the file itself, or the most recently changed
/// APK in a folder, looking a few levels down (Gradle puts them in
/// build/outputs/apk/debug, for example).
pub fn newest_apk(path: &Path) -> Option<PathBuf> {
    if path.is_file() {
        return Some(path.to_path_buf());
    }
    fn walk(dir: &Path, depth: usize, best: &mut Option<(SystemTime, PathBuf)>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                if depth > 0 {
                    walk(&path, depth - 1, best);
                }
            } else if path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("apk"))
            {
                let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                if best.as_ref().is_none_or(|(t, _)| modified > *t) {
                    *best = Some((modified, path));
                }
            }
        }
    }
    let mut best = None;
    walk(path, 6, &mut best);
    best.map(|(_, path)| path)
}

/// What identifies one build of an APK: where it is, when it changed, how big.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Build {
    pub path: PathBuf,
    modified: SystemTime,
    size: u64,
}

/// The current build at a path, if there is one.
pub fn current_build(path: &Path) -> Option<Build> {
    let apk = newest_apk(path)?;
    let meta = std::fs::metadata(&apk).ok()?;
    Some(Build {
        modified: meta.modified().ok()?,
        size: meta.len(),
        path: apk,
    })
}

/// Waits for a build newer than `last`, once it has stopped changing for a
/// moment, so a half-written file is never installed.
pub async fn next_build(path: &Path, last: Option<&Build>) -> Build {
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let Some(build) = current_build(path) else {
            continue;
        };
        if Some(&build) == last {
            continue;
        }
        // Settled: the same a second later.
        tokio::time::sleep(Duration::from_millis(1200)).await;
        if current_build(path).as_ref() == Some(&build) {
            return build;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_newest_apk_in_a_folder() {
        let root = std::env::temp_dir().join(format!("aae-test-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let deep = root.join("app/build/outputs/apk/debug");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(root.join("old.apk"), b"1").unwrap();
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(deep.join("app-debug.apk"), b"2").unwrap();
        std::fs::write(deep.join("notes.txt"), b"3").unwrap();
        assert_eq!(newest_apk(&root), Some(deep.join("app-debug.apk")));
        assert_eq!(
            newest_apk(&root.join("old.apk")),
            Some(root.join("old.apk"))
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
