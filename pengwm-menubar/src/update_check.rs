//! Background release check. Polls the release seam on a thread so the
//! network never touches the UI thread; the menu reads the shared result on
//! every open. A user's "Not now" verdict for one tag (`skip-update` in
//! `$PENGWM_HOME`) keeps that tag quiet; a different tag re-arms the ask.
//!
//! All of the work — fetch, tag parse, version compare — lives in
//! `pengwm_core::release` (the same module `pengwm update` asks); this file
//! owns only the cadence and the shared `latest` cell.
//!
//! Version source: this crate's own `CARGO_PKG_VERSION`. The workspace does
//! NOT pin versions (`workspace.package` is unused), so pengwm-core,
//! pengwm-daemon, and pengwm-menubar each carry their own `version` field
//! that is kept in sync manually — release tags and this comparison depend
//! on that staying true.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pengwm_core::release;

const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
/// Name of the "Not now" verdict file in `$PENGWM_HOME`. One line: the
/// skipped release tag. See `write_skipped_tag`.
const SKIP_FILE_NAME: &str = "skip-update";

/// Spawn the watcher thread. Runs forever; on every tick it overwrites
/// `latest` with the newest release tag whenever that release is strictly
/// newer than this build, and leaves it untouched on network/parse trouble.
pub fn watch(latest: Arc<Mutex<Option<String>>>) {
    loop {
        check(&latest);
        std::thread::sleep(CHECK_INTERVAL);
    }
}

fn check(latest: &Mutex<Option<String>>) {
    match release::check(env!("CARGO_PKG_VERSION")) {
        release::UpdateCheck::UpdateAvailable { latest: tag } => {
            let skipped = read_skipped_tag();
            if is_skipped(&tag, &skipped) {
                log::debug!("update {tag} is skipped by user verdict; staying quiet");
                *latest.lock().unwrap() = None;
            } else {
                log::info!(
                    "update available: {tag} (running {})",
                    env!("CARGO_PKG_VERSION")
                );
                *latest.lock().unwrap() = Some(tag);
            }
        }
        release::UpdateCheck::UpToDate { latest } => {
            log::debug!("no newer release than {} (latest {latest})", env!("CARGO_PKG_VERSION"));
        }
        release::UpdateCheck::Failed { reason } => {
            log::debug!("update check failed: {reason}");
        }
    }
}

/// `$PENGWM_HOME/skip-update` — the file recording a declined release.
fn skip_file_path() -> Option<PathBuf> {
    pengwm_core::home::pengwm_home().map(|home| home.join(SKIP_FILE_NAME))
}

/// Read the skipped release tag, if the user declined one and the file
/// survived (missing file, unreadable file: nothing skipped).
fn read_skipped_tag() -> Option<String> {
    let path = skip_file_path()?;
    match std::fs::read_to_string(&path) {
        Ok(s) => Some(s.trim().to_string()).filter(|t| !t.is_empty()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            log::debug!("skip file {}: {e}", path.display());
            None
        }
    }
}

/// Record the user's "Not now" for `tag`: `$PENGWM_HOME` is the layout
/// seam, so the verdict survives menubar restarts. Only silences this one
/// tag — a different, newer tag re-arms the ask.
pub fn write_skipped_tag(tag: &str) -> Result<(), String> {
    let path = skip_file_path().ok_or("PENGWM_HOME/HOME not resolvable")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("failed to create {}: {e}", dir.display()))?;
    }
    std::fs::write(&path, format!("{tag}\n"))
        .map_err(|e| format!("failed to write {}: {e}", path.display()))
}

/// The skip rule, pure so it tests without a filesystem: a declined tag
/// silences itself and nothing else.
fn is_skipped(seen: &str, skipped: &Option<String>) -> bool {
    skipped.as_deref() == Some(seen)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skip_is_per_tag() {
        assert!(is_skipped("v0.7.0", &Some("v0.7.0".to_string())));
        assert!(!is_skipped("v0.7.1", &Some("v0.7.0".to_string())));
        assert!(!is_skipped("v0.7.0", &None));
    }

    #[test]
    fn skip_file_roundtrip() {
        // PENGWM_HOME is the test seam; the only menubar test touching it.
        let dir = std::env::temp_dir().join(format!("pengwm-skip-test-{}", std::process::id()));
        std::env::set_var("PENGWM_HOME", &dir);
        assert_eq!(read_skipped_tag(), None);
        write_skipped_tag("v0.7.0").unwrap();
        assert_eq!(read_skipped_tag(), Some("v0.7.0".to_string()));
        std::env::remove_var("PENGWM_HOME");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
