use serde::{Serialize, Deserialize};
use interprocess::local_socket::LocalSocketListener;
use std::io::{BufRead, BufReader, Write};
use tokio::task;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "command", content = "args")]
pub enum IpcCommand {
    /// Update the maximum tiles allowed per workspace
    SetMaxTiles { workspace: u8, limit: usize },
    /// Define a new hotkey action
    RegisterHotkey { keys: Vec<String>, action: String },
    /// Force a reload of the configuration file
    ReloadConfig,
    /// Request the current state of the window tree
    GetState,
    // Add other commands as needed
}

pub async fn start_ipc_server() -> anyhow::Result<()> {
    #[cfg(target_os = "windows")]
    let pipe_name = "\\\\.\\pipe\\pengwm-ipc";
    #[cfg(target_os = "macos")]
    let pipe_name = "/tmp/pengwm.sock";

    // Clean up existing socket on unix
    #[cfg(target_os = "macos")]
    let _ = std::fs::remove_file(pipe_name);

    let listener = LocalSocketListener::bind(pipe_name)?;
    log::info!("IPC server listening on {}", pipe_name);

    task::spawn_blocking(move || {
        for conn in listener.incoming() {
            match conn {
                Ok(mut stream) => {
                    log::info!("New IPC connection established");
                    let mut reader = BufReader::new(&mut stream);
                    let mut line = String::new();
                    if let Ok(_) = reader.read_line(&mut line) {
                        match serde_json::from_str::<IpcCommand>(&line) {
                            Ok(cmd) => {
                                log::info!("Received command: {:?}", cmd);
                                // TODO: Handle command and send response
                                let response = "{\"status\": \"ok\"}\n";
                                let _ = stream.write_all(response.as_bytes());
                            }
                            Err(e) => log::error!("Failed to parse IPC command: {}", e),
                        }
                    }
                }
                Err(e) => log::error!("IPC connection failed: {}", e),
            }
        }
    });

    Ok(())
}
