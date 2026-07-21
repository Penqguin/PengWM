use tokio::net::UnixListener;
use tokio::sync::mpsc;
use crate::event_loop::DaemonEvent;

const SOCKET_PATH: &str = "/tmp/pengwm.sock";

pub async fn listen(event_tx: mpsc::Sender<DaemonEvent>) {
    let _ = std::fs::remove_file(SOCKET_PATH);

    let listener = match UnixListener::bind(SOCKET_PATH) {
        Ok(l) => l,
        Err(e) => {
            log::error!("Failed to bind UDS at {}: {}", SOCKET_PATH, e);
            return;
        }
    };

    log::info!("UDS listener bound to {}", SOCKET_PATH);

    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                let tx = event_tx.clone();
                tokio::spawn(async move {
                    handle_client(stream, tx).await;
                });
            }
            Err(e) => {
                log::error!("UDS accept error: {}", e);
            }
        }
    }
}

async fn handle_client(
    mut stream: tokio::net::UnixStream,
    event_tx: mpsc::Sender<DaemonEvent>,
) {
    use tokio::io::AsyncReadExt;
    use tokio::io::AsyncWriteExt;

    let mut buf = vec![0u8; 4096];
    let n = match stream.read(&mut buf).await {
        Ok(n) if n > 0 => n,
        Ok(_) => return,
        Err(e) => {
            log::warn!("UDS read error: {}", e);
            return;
        }
    };

    let cmd: pengwm_core::command::DaemonCommand = match serde_json::from_slice(&buf[..n]) {
        Ok(c) => c,
        Err(e) => {
            let resp = serde_json::to_string(&pengwm_core::command::DaemonResponse::Error {
                message: format!("Invalid JSON: {}", e),
            })
            .unwrap_or_default();
            let _ = stream.write_all(resp.as_bytes()).await;
            return;
        }
    };

    let (resp_tx, mut resp_rx) = mpsc::channel(1);
    if event_tx.send(DaemonEvent::Command(cmd, resp_tx)).await.is_err() {
        return;
    }

    if let Some(response) = resp_rx.recv().await {
        let json = serde_json::to_string(&response).unwrap_or_default();
        let _ = stream.write_all(json.as_bytes()).await;
    }
}
