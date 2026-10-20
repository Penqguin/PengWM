use std::io::{Read, Write};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, Semaphore};

use crate::event_loop::DaemonEvent;
use crate::socket_guard::{bind_first, SocketGuard};

/// How long a handler thread waits on a single `read` from a client
/// before giving up. Legitimate clients are request/response over a
/// local socket — connect, write, shutdown(Write), done in
/// milliseconds. The timeout exists for clients that connect and then
/// stall: without it, a client that never EOFs parks a pool thread in
/// `read` forever, and 32 wedged threads is every CLI command failing
/// until restart. Firing surfaces as `WouldBlock`, which the read loop
/// already treats as end-of-payload.
const READ_TIMEOUT: Duration = Duration::from_secs(5);

/// The accept loop polls instead of blocking on `accept`, so the per-tick
/// socket-identity check (`SocketGuard::tick`) can run and a rebound
/// listener can be swapped in. Worst-case added accept latency is one
/// poll — well inside the daemon's 50ms pump granularity.
const ACCEPT_POLL: Duration = Duration::from_millis(25);

pub use pengwm_core::ipc::COMMAND_SOCKET_PATH as DEFAULT_SOCKET_PATH;

pub fn start_ipc_server(event_tx: mpsc::Sender<DaemonEvent>) {
    start_ipc_server_with_path(event_tx, DEFAULT_SOCKET_PATH);
}

/// True when a live daemon answers on `path`.
///
/// Both servers used to `remove_file` + `bind` unconditionally, so a second
/// daemon silently stole the socket from the first — two daemons tiled at
/// once and CLI commands landed on whichever process last rebound (the exact
/// "it has access but never works" pathology). Probe-then-bind: connect
/// first; a live daemon answers immediately, a stale socket file left by a
/// crashed daemon refuses the connection and is safe to rebind.
pub fn daemon_already_running_at(path: &str) -> bool {
    use std::os::unix::net::UnixStream;
    UnixStream::connect(path).is_ok()
}

/// Check the default command socket for a live daemon.
pub fn daemon_already_running() -> bool {
    daemon_already_running_at(DEFAULT_SOCKET_PATH)
}

pub fn start_ipc_server_with_path(event_tx: mpsc::Sender<DaemonEvent>, socket_path: &str) {
    // Bind FIRST, probe only on `Address already in use` (see
    // `socket_guard::bind_first`). After binding, the accept loop runs a
    // per-tick socket-identity check: a rogue `rm` used to leave the
    // listener on an unlinked vnode — the WM kept tiling while every CLI
    // connect hit ECONNREFUSED (the ghost-daemon state). Now the guard
    // notices the missing/foreign file, logs loudly, and rebinds.
    let listener = match bind_first(socket_path, "IPC server") {
        Ok(l) => l,
        Err(_) => return,
    };

    log::info!("UDS listener bound to {}", socket_path);

    let mut guard = SocketGuard::capture(socket_path);
    let mut listener = listener;
    listener
        .set_nonblocking(true)
        .expect("set listener nonblocking");

    let sem = Arc::new(Semaphore::new(32));
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                // Accepted sockets inherit O_NONBLOCK on some platforms
                // (Linux); handle_client does blocking reads — normalize,
                // and bound each read so a client that never EOFs cannot
                // park a pool thread forever.
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(READ_TIMEOUT));
                let tx = event_tx.clone();
                let path = socket_path.to_owned();
                let sem = Arc::clone(&sem);
                if let Ok(_permit) = sem.try_acquire_owned() {
                    thread::spawn(move || {
                        let _permit = _permit; // hold until handle_client returns
                        handle_client(stream, tx, &path);
                    });
                } else {
                    log::warn!("ipc thread pool full (32), dropping connection");
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => {
                log::error!("UDS accept error: {}", e);
            }
        }
        if let Some(fresh) = guard.tick(Instant::now()) {
            let _ = fresh.set_nonblocking(true);
            listener = fresh;
            log::info!("UDS listener rebound to {}", socket_path);
        }
        thread::sleep(ACCEPT_POLL);
    }
}

fn handle_client(
    mut stream: std::os::unix::net::UnixStream,
    event_tx: mpsc::Sender<DaemonEvent>,
    _socket_path: &str,
) {
    // Read until EOF (client does shutdown(Write) after JSON). Loop so
    // payloads larger than 4096 are not truncated.
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    loop {
        match stream.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(e) => {
                log::warn!("UDS read error: {}", e);
                return;
            }
        }
    }
    if buf.is_empty() {
        return;
    }

    let cmd: pengwm_core::command::Command = match serde_json::from_slice(&buf) {
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
