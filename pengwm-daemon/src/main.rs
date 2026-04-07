//! Main entry point for the pengwm-daemon.
//! Initializes the window manager backend, configuration, and IPC server.

/// Core window management logic and types.
mod core;
/// Platform-specific windowing system integrations.
mod platform;
/// Inter-process communication for the UI.
mod ipc;
/// Configuration management and file watching.
mod config;

use crate::platform::WindowManagerBackend;
use crate::platform::macos::MacOsBackend;
use crate::core::manager::WindowManager;
use crate::config::Config;
use crate::ipc::IpcServer;
use std::sync::Arc;
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logger with environment variables (e.g., RUST_LOG=info)
    env_logger::init();
    log::info!("Starting pengwm-daemon...");

    // Select the appropriate window manager backend based on the target OS.
    #[cfg(target_os = "windows")]
    let backend = Arc::new(crate::platform::windows::WindowsBackend);
    
    #[cfg(target_os = "macos")]
    let backend = Arc::new(MacOsBackend);

    // Load user configuration from config.toml or use defaults.
    let config = Config::load();
    
    // Communication channel between the OS backend and the Window Manager.
    let (event_tx, event_rx) = mpsc::channel(100);

    // The IPC server allows the Tauri UI to receive updates.
    let ipc_server = Arc::new(IpcServer::new());

    // Subscribe to system events (window creation, destruction, etc.).
    let backend_clone = backend.clone();
    backend_clone.subscribe(event_tx).await;

    // Initialize and start the core Window Manager loop.
    let mut wm = WindowManager::new(backend, config, ipc_server.clone());
    tokio::spawn(async move {
        wm.run(event_rx).await;
    });

    // Start the IPC server to communicate with the UI.
    let ipc_server_clone = ipc_server.clone();
    tokio::spawn(async move {
        if let Err(e) = ipc::start_ipc_server(ipc_server_clone).await {
            log::error!("IPC server error: {}", e);
        }
    });

    log::info!("Daemon running. Press Ctrl+C to exit.");
    
    // Block the main thread until a termination signal is received.
    tokio::signal::ctrl_c().await?;
    log::info!("Shutting down...");

    Ok(())
}
