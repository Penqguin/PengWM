//! LaunchAgent awareness.
//!
//! The daemon runs under a per-user LaunchAgent (`com.pengwm.daemon`,
//! installed by `install.sh`). One quirk drives everything here: on current
//! macOS, `KeepAlive { SuccessfulExit = false }` restarts the job even after
//! a **clean exit 0** (observed live: `runs` climbing with
//! `last exit code = 0` right after a successful `pengwm quit`). So
//! "restart on crash, stay down on quit" cannot be expressed by the plist
//! alone — quitting must remove the job registration itself. `RunAtLoad`
//! re-registers it at the next login, and a crash while the job is registered
//! still triggers the respawn, so nothing else is lost.

pub const AGENT_LABEL: &str = "com.pengwm.daemon";

pub fn agent_plist_path() -> Option<std::path::PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(std::path::PathBuf::from(home)
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{AGENT_LABEL}.plist")))
}

/// Remove the LaunchAgent job registration. Called on clean shutdown
/// (`pengwm quit`, menubar Quit, SIGTERM). When the daemon is not running
/// under launchd the `launchctl bootout` fails non-fatally and is ignored.
pub fn remove_agent_job() {
    let Some(plist) = agent_plist_path() else { return };
    if !plist.exists() {
        // No agent installed (manual run) — nothing to deregister.
        return;
    }
    let sh = format!(
        "launchctl bootout gui/$(id -u)/{AGENT_LABEL} 2>/dev/null || true"
    );
    match std::process::Command::new("sh").arg("-c").arg(&sh).status() {
        Ok(_) => log::info!("LaunchAgent job deregistered ({AGENT_LABEL}) — quit stays quit"),
        Err(e) => log::debug!("launchctl bootout failed to spawn: {e}"),
    }
}
