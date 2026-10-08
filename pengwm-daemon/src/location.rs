//! One-copy policy: the daemon must run from the installed binary location.
//!
//! History: a dev checkout and the installed copy both ended up running —
//! launchd kept the installed one alive while a manual run from
//! `target/debug` lived too — and the two daemons fought over the IPC
//! socket, each stealing it from the other so windows tiled twice. One
//! blessed install root is now the only non-dev location; development
//! builds must opt back in with `PENGWM_DEV=1`.
//!
//! The blessed root is a plain binary directory (ADR-0001, which retired
//! the `.app` bundle): `$PENGWM_HOME/bin` with `PENGWM_HOME` defaulting to
//! `~/.pengwm`. Homebrew Cellar pengwm paths are also blessed so a
//! `brew install pengwm` copy satisfies the same policy without config.
//!
//! Refusals exit 0 so launchd's `KeepAlive` (`SuccessfulExit=false`)
//! does not resurrect a loser that is intentionally not running.

use std::path::{Path, PathBuf};

/// The install root: `$PENGWM_HOME` (default `~/.pengwm`). The blessed
/// binary directory is `<root>/bin`; the CLI symlinks, LaunchAgent, and
/// this policy all resolve against it.
pub fn pengwm_home() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("PENGWM_HOME") {
        if !home.is_empty() {
            return Some(PathBuf::from(home));
        }
    }
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".pengwm"))
}

/// The installed-binary directory the one-copy policy accepts.
pub fn installed_bin_dirs() -> Vec<PathBuf> {
    match pengwm_home() {
        Some(home) => vec![home.join("bin")],
        None => Vec::new(),
    }
}

/// True when `path` lives in the installed bin directory or in a Homebrew
/// Cellar pengwm path (`.../Cellar/pengwm/<version>/...`).
pub fn path_is_installed_entry(path: &Path) -> bool {
    // Canonicalize so symlinks on either side (e.g. a $PREFIX symlink into
    // the bin dir, or brew's Cellar symlinks) still match; fall back to the
    // literal path when the root is not on disk yet.
    let resolved = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if installed_bin_dirs().iter().any(|bin| {
        let bin = bin.canonicalize().unwrap_or_else(|_| bin.clone());
        resolved.starts_with(&bin)
    }) {
        return true;
    }
    // Homebrew installs live in `.../Cellar/pengwm/<version>/bin/...` and
    // move with every upgrade; bless the Cellar path shape so brew users
    // satisfy the policy too. Component-wise so only *pengwm's* Cellar
    // matches (not "Cellar/pengwm-x" neighbors).
    let mut components = resolved.components();
    while let Some(comp) = components.next() {
        if comp.as_os_str() == "Cellar"
            && components.next().map(|c| c.as_os_str() == "pengwm") == Some(true)
        {
            return true;
        }
    }
    false
}

/// `PENGWM_DEV=1` (any non-empty value except "0") opts dev builds back in.
pub fn dev_override_enabled() -> bool {
    match std::env::var("PENGWM_DEV") {
        Ok(value) => !value.is_empty() && value != "0",
        Err(_) => false,
    }
}

/// Refuse (exit 0) unless this is a development build or the running binary
/// sits in the installed bin dir. `action` names the operation being
/// refused ("start", "update"), `guidance` is the one-line advice printed
/// after the offending path.
fn ensure_installed_location(action: &str, guidance: &str) {
    if dev_override_enabled() {
        return;
    }
    match std::env::current_exe().and_then(|p| p.canonicalize()).ok() {
        Some(exe) if path_is_installed_entry(&exe) => {}
        exe => {
            let exe = exe.unwrap_or_else(|| PathBuf::from("<unknown>"));
            let root = pengwm_home()
                .map(|h| h.join("bin").display().to_string())
                .unwrap_or_else(|| "~/.pengwm/bin".to_string());
            eprintln!("error: refusing to {action}: this looks like a non-installed PengWM copy");
            eprintln!("  at: {}", exe.display());
            eprintln!("{guidance}");
            eprintln!("The installed copy lives in {root} — start that one.");
            eprintln!("If this is a development build, set PENGWM_DEV=1 to allow it.");
            // Exit 0: KeepAlive restarts only on nonzero exits, so an
            // intentionally refused daemon must not look like a crash.
            std::process::exit(0);
        }
    }
}

/// Called by the daemon start path, right after the single-instance guard.
pub fn enforce_installed_location() {
    ensure_installed_location("start", "Run install.sh to install PengWM properly.");
}

/// Called by `pengwm update`: it modifies the installed copy, so a dev
/// checkout must not run the installer on its behalf.
pub fn enforce_installed_for_update() {
    ensure_installed_location("update", "updates apply to the installed copy — install first.");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exe_inside_install_root_is_installed() {
        // Build the candidate from the same source as the check so the test
        // stays correct whether or not the root exists on this machine.
        let bin = installed_bin_dirs()
            .into_iter()
            .next()
            .expect("PENGWM_HOME/HOME always resolvable in tests");
        let exe = bin.join("pengwm");
        assert!(path_is_installed_entry(&exe));
    }

    #[test]
    fn lookalike_directory_is_not_installed() {
        // starts_with compares components, so a sibling directory whose
        // name merely shares the bin dir's name as a prefix must not pass.
        let bin = installed_bin_dirs().into_iter().next().unwrap();
        let exe = bin.with_file_name({
            let mut name = bin.file_name().unwrap().to_os_string();
            name.push(".bak");
            name
        }).join("pengwm");
        assert!(!path_is_installed_entry(&exe));
    }

    #[test]
    fn dev_target_is_not_installed() {
        let exe = Path::new("/Users/dev/PengWM/target/debug/pengwm");
        assert!(!path_is_installed_entry(exe));
    }

    #[test]
    fn homebrew_cellar_path_is_installed() {
        let exe = Path::new(
            "/opt/homebrew/Cellar/pengwm/0.6.0/bin/pengwm",
        );
        assert!(path_is_installed_entry(exe));
    }

    #[test]
    fn other_cellar_formulas_are_not_installed() {
        // Only *pengwm's* Cellar is blessed — not Cellar/pengwm-x, not
        // some other formula's bin.
        let exe = Path::new("/opt/homebrew/Cellar/pengwm-x/0.6.0/bin/pengwm");
        assert!(!path_is_installed_entry(exe));
        let exe = Path::new("/opt/homebrew/Cellar/wget/1.21/bin/wget");
        assert!(!path_is_installed_entry(exe));
    }

    #[test]
    fn pengwm_home_override_redirects_the_root() {
        // PENGWM_HOME is the test seam for the policy: it must move the
        // blessed bin dir wholesale. Serialized crudely by being the only
        // test that touches PENGWM_HOME.
        let dir = std::env::temp_dir().join(format!("pengwm-location-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        // The files must exist so canonicalization runs on both sides (the
        // /var -> /private/var symlink on macOS only resolves for real
        // paths).
        std::fs::write(dir.join("bin/pengwm"), b"stub").unwrap();
        std::fs::write(dir.join("bin/pengwm-menubar"), b"stub").unwrap();
        std::env::set_var("PENGWM_HOME", &dir);
        assert!(path_is_installed_entry(&dir.join("bin/pengwm")));
        assert!(path_is_installed_entry(&dir.join("bin/pengwm-menubar")));
        std::env::remove_var("PENGWM_HOME");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dev_override_defaults_off() {
        // The predicate itself is env-independent; just exercise the
        // default-off path when the variable is unset.
        std::env::remove_var("PENGWM_DEV");
        assert!(!super::dev_override_enabled());
    }
}
