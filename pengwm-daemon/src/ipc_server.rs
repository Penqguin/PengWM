use std::io::{Read, Write};
use std::os::unix::net::UnixListener;
use std::thread;
use tokio::sync::mpsc;

use crate::event_loop::DaemonEvent;

pub use pengwm_core::ipc::COMMAND_SOCKET_PATH as DEFAULT_SOCKET_PATH;

pub fn start_ipc_server(event_tx: mpsc::Sender<DaemonEvent>) {
    start_ipc_server_with_path(event_tx, DEFAULT_SOCKET_PATH);
}

pub fn start_ipc_server_with_path(event_tx: mpsc::Sender<DaemonEvent>, socket_path: &str) {
    let _ = std::fs::remove_file(socket_path);

    let listener = match UnixListener::bind(socket_path) {
        Ok(l) => l,
        Err(e) => {
            log::error!("Failed to bind UDS at {}: {}", socket_path, e);
            return;
        }
    };

    log::info!("UDS listener bound to {}", socket_path);

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let tx = event_tx.clone();
                let path = socket_path.to_owned();
                thread::spawn(move || {
                    handle_client(stream, tx, &path);
                });
            }
            Err(e) => {
                log::error!("UDS accept error: {}", e);
            }
        }
    }
}

fn handle_client(
    mut stream: std::os::unix::net::UnixStream,
    event_tx: mpsc::Sender<DaemonEvent>,
    _socket_path: &str,
) {
    let mut buf = vec![0u8; 4096];
    let n = match stream.read(&mut buf) {
        Ok(n) if n > 0 => n,
        Ok(_) => return,
        Err(e) => {
            log::warn!("UDS read error: {}", e);
            return;
        }
    };

    let cmd: pengwm_core::command::Command = match serde_json::from_slice(&buf[..n]) {
        Ok(c) => c,
        Err(e) => {
            let resp = serde_json::to_string(&pengwm_core::command::DaemonResponse::Error {
                message: format!("Invalid JSON: {}", e),
            })
            .unwrap_or_default();
            let _ = stream.write_all(resp.as_bytes());
            return;
        }
    };

    let (resp_tx, mut resp_rx) = mpsc::channel(1);
    if event_tx
        .blocking_send(DaemonEvent::Command(cmd, Some(resp_tx)))
        .is_err()
    {
        return;
    }

    if let Some(response) = resp_rx.blocking_recv() {
        let json = serde_json::to_string(&response).unwrap_or_default();
        let _ = stream.write_all(json.as_bytes());
    }
}
