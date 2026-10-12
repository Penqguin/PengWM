//! Background release check. Polls the release seam on a thread so the
//! network never touches the UI thread; the menu reads the shared result on
//! every open.
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

use std::sync::{Arc, Mutex};
use std::time::Duration;

use pengwm_core::release;

const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

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
            log::info!(
                "update available: {tag} (running {})",
                env!("CARGO_PKG_VERSION")
            );
            *latest.lock().unwrap() = Some(tag);
        }
        release::UpdateCheck::UpToDate { latest } => {
            log::debug!("no newer release than {} (latest {latest})", env!("CARGO_PKG_VERSION"));
        }
        release::UpdateCheck::Failed { reason } => {
            log::debug!("update check failed: {reason}");
        }
    }
}
