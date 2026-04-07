//! Inter-Process Communication (IPC) for pengwm.
//! Enables communication between the daemon and external UI clients (Tauri).

use serde::{Serialize, Deserialize};
use interprocess::local_socket::{LocalSocketListener, LocalSocketStream};
use std::io::Write;
use tokio::sync::broadcast;
use tokio::task;
use std::sync::{Arc, Mutex};

/// Commands that external clients can send to the daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "command", content = "args")]
pub enum IpcCommand {
    /// Update the maximum tiles allowed per workspace.
    SetMaxTiles { workspace: u8, limit: usize },
    /// Define a new hotkey action.
    RegisterHotkey { keys: Vec<String>, action: String },
    /// Force a reload of the configuration file.
    ReloadConfig,
    /// Request the current state of the window tree.
    GetState,
}

/// Events sent from the daemon to all connected UI clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum UiEvent {
    /// Broadcasted when the window manager's layout or state changes.
    StateChanged(UiState),
}

/// Current state of the window manager, synchronized with the UI.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UiState {
    /// List of managed windows and their current geometries.
    pub windows: Vec<WindowInfo>,
    /// The ID of the currently focused window, if any.
    pub focused_window: Option<usize>,
}

/// Metadata about a single managed window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowInfo {
    /// Unique identifier for the window (pointer address).
    pub id: usize,
    /// The display title of the window.
    pub title: String,
    /// X coordinate of the window's top-left corner.
    pub x: i32,
    /// Y coordinate of the window's top-left corner.
    pub y: i32,
    /// Width of the window in pixels.
    pub width: u32,
    /// Height of the window in pixels.
    pub height: u32,
}

/// The IPC server that handles multiple client connections and broadcasts updates.
pub struct IpcServer {
    /// Channel used for broadcasting UI events to all connected streams.
    tx: broadcast::Sender<UiEvent>,
    /// Cached latest state for new connections.
    state: Arc<Mutex<UiState>>,
}

impl IpcServer {
    /// Initializes a new IPC server.
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(16);
        Self {
            tx,
            state: Arc::new(Mutex::new(UiState::default())),
        }
    }

    /// Broadcasts a new state to all connected UI clients.
    pub fn broadcast_state(&self, new_state: UiState) {
        let mut state = self.state.lock().unwrap();
        *state = new_state.clone();
        // Send the state update across the broadcast channel.
        let _ = self.tx.send(UiEvent::StateChanged(new_state));
    }
}

/// Starts the IPC server and listens for incoming local socket connections.
pub async fn start_ipc_server(server: Arc<IpcServer>) -> anyhow::Result<()> {
    #[cfg(target_os = "windows")]
    let pipe_name = "\\\\.\\pipe\\pengwm-ipc";
    #[cfg(target_os = "macos")]
    let pipe_name = "/tmp/pengwm.sock";

    // Clean up existing socket file on Unix-like systems.
    #[cfg(target_os = "macos")]
    let _ = std::fs::remove_file(pipe_name);

    let listener = LocalSocketListener::bind(pipe_name)?;
    log::info!("IPC server listening on {}", pipe_name);

    let server_clone = server.clone();
    // Spawn a blocking task to accept new connections.
    task::spawn_blocking(move || {
        for conn in listener.incoming() {
            match conn {
                Ok(stream) => {
                    log::info!("New IPC connection established");
                    let server_inner = server_clone.clone();
                    // Handle each connection in its own asynchronous task.
                    task::spawn(handle_connection(stream, server_inner));
                }
                Err(e) => log::error!("IPC connection failed: {}", e),
            }
        }
    });

    Ok(())
}

/// Manages a single client connection, streaming broadcasted events.
async fn handle_connection(mut stream: LocalSocketStream, server: Arc<IpcServer>) {
    let mut rx = server.tx.subscribe();
    
    tokio::spawn(async move {
        // Send updates as JSON-encoded lines to the client.
        while let Ok(event) = rx.recv().await {
            let json = serde_json::to_string(&event).unwrap() + "\n";
            if let Err(e) = stream.write_all(json.as_bytes()) {
                log::error!("Failed to send broadcast to UI: {}", e);
                break;
            }
        }
    });
}
