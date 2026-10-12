//! The install root — where PengWM lives on disk. Every crate resolves it
//! the same way: `$PENGWM_HOME` when set (the test seam and the
//! `install.sh --home` override), else `$HOME/.pengwm`. The daemon's
//! one-copy policy blesses `<root>/bin`; the menubar drops its small
//! verdict files beside it (e.g. `skip-update`).

use std::path::PathBuf;

/// The install root: `$PENGWM_HOME` (default `~/.pengwm`). The blessed
/// binary directory is `<root>/bin`; the CLI symlinks, LaunchAgent, and
/// the daemon's one-copy policy all resolve against it.
pub fn pengwm_home() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("PENGWM_HOME") {
        if !home.is_empty() {
            return Some(PathBuf::from(home));
        }
    }
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".pengwm"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn override_moves_the_root() {
        // PENGWM_HOME is the test seam: it must move the root wholesale.
        // The only test in this crate that touches the variable.
        let dir = std::env::temp_dir().join(format!("pengwm-home-test-{}", std::process::id()));
        std::env::set_var("PENGWM_HOME", &dir);
        assert_eq!(pengwm_home(), Some(dir.clone()));
        std::env::remove_var("PENGWM_HOME");
        assert_eq!(
            pengwm_home(),
            std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".pengwm"))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
