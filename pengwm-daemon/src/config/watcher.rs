//! File watcher for live config reload.
//!
//! Uses the notify crate to watch config.toml for changes.
//! When a change is detected, sends a DaemonEvent::Command(ReloadConfig, ..)
//! so the state manager can re-read the file and update settings at runtime.

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::Path;
use std::sync::mpsc as std_mpsc;
use std::thread;
use tokio::sync::mpsc as tokio_mpsc;

use crate::event_loop::DaemonEvent;
use pengwm_core::command::Command;

/// Start watching the config file for changes.
///
/// Spawns a background thread running a notify::RecommendedWatcher.
/// On any Modify event, sends DaemonEvent::Command(ReloadConfig, ..) into the event loop.
pub fn watch(event_tx: tokio_mpsc::Sender<DaemonEvent>) {
    let config_path = crate::config::config_file_path();
    let config_dir = config_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| Path::new(".").to_path_buf());

    thread::spawn(move || {
        let (tx, rx) = std_mpsc::channel();

        let mut watcher: RecommendedWatcher = notify::recommended_watcher(move |res| {
            let _ = tx.send(res);
        })
        .expect("failed to create file watcher");

        if let Err(e) = watcher.watch(&config_dir, RecursiveMode::NonRecursive) {
            log::warn!(
                "Failed to watch config directory '{}': {e}",
                config_dir.display()
            );
            return;
        }
        log::info!(
            "Watching config directory '{}' for changes",
            config_dir.display()
        );

        // Keep the watcher alive and forward modify events to the event loop.
        for event in rx {
            match event {
                Ok(event) if matches!(event.kind, EventKind::Modify(_)) => {
                    let (rtx, _) = tokio_mpsc::channel(1);
                    let _ = event_tx.try_send(DaemonEvent::Command(Command::ReloadConfig, rtx));
                }
                Ok(event) => {
                    log::trace!("Ignoring non-modify file event: {:?}", event.kind);
                }
                Err(e) => {
                    log::warn!("File watcher error: {e}");
                }
            }
        }
    });
}
