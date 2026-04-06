mod core;
mod platform;
mod ipc;
mod config;

use crate::platform::WindowManagerBackend;
use crate::platform::macos::MacOsBackend;
use crate::core::manager::WindowManager;
use crate::config::Config;
use std::sync::Arc;
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();
    log::info!("Starting pengwm-daemon...");

    #[cfg(target_os = "windows")]
    let backend = Arc::new(WindowsBackend);
    
    #[cfg(target_os = "macos")]
    let backend = Arc::new(MacOsBackend);

    let config = Config::default();
    let (event_tx, event_rx) = mpsc::channel(100);

    // Subscribe to system events
    let backend_clone = backend.clone();
    backend_clone.subscribe(event_tx).await;

    // Start Window Manager
    let mut wm = WindowManager::new(backend, config);
    tokio::spawn(async move {
        wm.run(event_rx).await;
    });

    // Start IPC server
    tokio::spawn(async {
        if let Err(e) = ipc::start_ipc_server().await {
            log::error!("IPC server error: {}", e);
        }
    });

    log::info!("Daemon running. Press Ctrl+C to exit.");
    
    // Wait for shutdown signal
    tokio::signal::ctrl_c().await?;
    log::info!("Shutting down...");

    Ok(())
}
