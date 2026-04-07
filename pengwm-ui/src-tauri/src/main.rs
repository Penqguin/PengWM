// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use interprocess::local_socket::LocalSocketStream;
use std::io::{BufRead, BufReader};
use tauri::Emitter;
use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiState {
    pub windows: Vec<WindowInfo>,
    pub focused_window: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowInfo {
    pub id: usize,
    pub title: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum UiEvent {
    StateChanged(UiState),
}

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![greet])
        .setup(|app| {
            let handle = app.handle().clone();
            
            tauri::async_runtime::spawn(async move {
                #[cfg(target_os = "windows")]
                let pipe_name = "\\\\.\\pipe\\pengwm-ipc";
                #[cfg(target_os = "macos")]
                let pipe_name = "/tmp/pengwm.sock";

                loop {
                    match LocalSocketStream::connect(pipe_name) {
                        Ok(stream) => {
                            let mut reader = BufReader::new(stream);
                            let mut line = String::new();
                            while reader.read_line(&mut line).is_ok() {
                                if line.is_empty() { break; }
                                if let Ok(event) = serde_json::from_str::<UiEvent>(&line) {
                                    let _ = handle.emit("state-changed", event);
                                }
                                line.clear();
                            }
                        }
                        Err(_) => {
                            // Daemon might not be running yet, retry in 1s
                            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                        }
                    }
                }
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
