//! File watcher for live config reload.
//!
//! Uses the notify crate to watch config.toml for changes.
//! When a change is detected, sends a DaemonEvent::Command(ReloadConfig, ..)
//! so the state manager can re-read the file and update settings at runtime.

use tokio::sync::mpsc;
use crate::event_loop::DaemonEvent;

/// Start watching the config file for changes.
///
/// Spawns a background task that calls notify::RecommendedWatcher.
/// On any Modify event, sends DaemonEvent::ReloadConfig into the event loop.
pub async fn watch(event_tx: mpsc::Sender<DaemonEvent>) {
    //  let config_path = config::config_file_path()
    //  let mut watcher = notify::recommended_watcher(move |res| {
    //      match res {
    //          Ok(Event { kind: EventKind::Modify(_), .. }) => {
    //              event_tx.send(DaemonEvent::ReloadConfig).ok();
    //          }
    //          _ => {}
    //      }
    //  })
    //  watcher.watch(config_path.parent(), RecursiveMode::NonRecursive)
    //  keep the watcher alive (loop forever or sleep)
    todo!("file watcher")
}
