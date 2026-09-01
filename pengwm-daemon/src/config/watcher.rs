//! File watcher for live config reload.
//!
//! Uses the notify crate to watch config.toml for changes.
//! When a change is detected, sends a DaemonEvent::Command(ReloadConfig, ..)
//! so the state manager can re-read the file and update settings at runtime.

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::Path;
use std::sync::mpsc as std_mpsc;
use std::thread;
use std::time::Duration;
use tokio::sync::mpsc as tokio_mpsc;

use crate::event_loop::DaemonEvent;
use pengwm_core::command::Command;

/// Start watching the config file for changes.
///
/// Spawns a background thread running a notify::RecommendedWatcher.
/// On any Modify event for the config file path, debounces 50ms and sends
/// a single DaemonEvent::Command(ReloadConfig, ..) into the event loop.
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
        while let Ok(event) = rx.recv() {
            match event {
                Ok(event) if matches!(event.kind, EventKind::Modify(_)) => {
                    // Filter: only care about the actual config file.
                    if !event.paths.iter().any(|p| p == &config_path) {
                        log::trace!("Ignoring modify for non-config path: {:?}", event.paths);
                        continue;
                    }
                    // Debounce: editor save via rename fires multiple Modify events
                    // (create+modify+remove). Sleep 50ms and drain burst, then send one.
                    thread::sleep(Duration::from_millis(50));
                    while let Ok(Ok(next)) = rx.try_recv() {
                        if matches!(next.kind, EventKind::Modify(_))
                            && next.paths.iter().any(|p| p == &config_path)
                        {
                            // Still a config modify within debounce window — coalesce
                            // and extend debounce slightly.
                            thread::sleep(Duration::from_millis(10));
                        }
                        // Drain everything else in burst without extra handling
                    }
                    // Drain any remaining queued events (non-modify) to avoid backlog
                    while rx.try_recv().is_ok() {}

                    let _ = event_tx.try_send(DaemonEvent::Command(Command::ReloadConfig, None));
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
