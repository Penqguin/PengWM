use std::io::Write;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use pengwm_core::command::BarMessage;

use crate::socket_guard::{bind_first, SocketGuard};

/// The bar accept loop polls instead of blocking on `accept`, so the
/// per-tick socket-identity check can run and a rebound listener can be
/// swapped in (same contract as the command socket; see `ipc_server`).
const ACCEPT_POLL: Duration = Duration::from_millis(25);

pub use pengwm_core::ipc::BAR_SOCKET_PATH;

#[derive(Clone)]
pub struct BarSender {
    tx: tokio::sync::mpsc::Sender<BarMessage>,
}

impl BarSender {
    /// Non-blocking broadcast to every connected `pengwm-menubar` client.
    pub fn send(&self, msg: BarMessage) {
        let _ = self.tx.try_send(msg);
    }

    /// Construct a sender wired to an existing channel (used by tests).
    #[cfg(test)]
    pub(crate) fn from_channel(tx: tokio::sync::mpsc::Sender<BarMessage>) -> Self {
        Self { tx }
    }
}

/// Spawn the daemon→UI push server on its own thread using the default
/// socket. One consumer: the spawned `pengwm-menubar` child (the legacy
/// `pengwm` bar name — and the socket path — are kept for the wire format).
pub fn spawn_bar_server() -> BarSender {
    spawn_bar_server_with_path(BAR_SOCKET_PATH)
}

/// Spawn the daemon→UI push server on its own thread.
///
/// The last `State` snapshot and the last `Exit` marker are cached and
/// replayed to a freshly connected menubar, so it renders current state even
/// if it connects after a broadcast.
pub fn spawn_bar_server_with_path(socket_path: &str) -> BarSender {
    let socket_path = socket_path.to_owned();
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    thread::spawn(move || {
        // Bind FIRST, probe on `Address already in use` — same contract as
        // the command socket (see `socket_guard::bind_first` and
        // `ipc_server::start_ipc_server_with_path`): a live listener can
        // never be unlinked by a rival, only a stale file from a crashed
        // daemon is removed. The accept loop runs the same per-tick
        // socket-identity check, so a rogue `rm` of the bar socket heals
        // too (a reconnecting menubar used to hit ECONNREFUSED forever).
        let mut listener = match bind_first(&socket_path, "UI push server") {
            Ok(l) => l,
            Err(_) => return,
        };
        log::info!("Bar listener bound to {}", socket_path);
        let mut guard = SocketGuard::capture(&socket_path);
        let _ = listener.set_nonblocking(true);

        // Per-client channels: each client gets its own writer thread owning its
        // UnixStream. Broadcast only enqueues via try_send without holding a
        // lock during blocking I/O, so a slow bar never stalls others.
        let clients: Arc<Mutex<Vec<tokio::sync::mpsc::Sender<Vec<u8>>>>> =
            Arc::new(Mutex::new(Vec::new()));
        // Cache the last `State` separately so a freshly connecting
        // menubar can reconstruct the current picture. `Exit` is not
        // cached: the daemon would exit before the next menubar connects.
        let last_state: Arc<Mutex<Option<Vec<u8>>>> = Arc::new(Mutex::new(None));

        {
            let clients = Arc::clone(&clients);
            let last_state = Arc::clone(&last_state);
            thread::spawn(move || loop {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        let (client_tx, mut client_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(32);
                        // Writer thread owns the UnixStream
                        thread::spawn(move || {
                            while let Some(payload) = client_rx.blocking_recv() {
                                if stream.write_all(&payload).is_err() {
                                    break;
                                }
                                let _ = stream.flush();
                            }
                        });

                        // Replay the cached state to the new client
                        let state = last_state.lock().unwrap().clone();
                        if let Some(payload) = state {
                            let _ = client_tx.try_send(payload);
                        }

                        clients.lock().unwrap().push(client_tx);
                        log::info!("bar connected ({} clients)", clients.lock().unwrap().len());

                        // Prune disconnected senders (channel closed)
                        clients.lock().unwrap().retain(|tx| !tx.is_closed());
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(e) => log::error!("bar socket accept error: {}", e),
                }
                if let Some(fresh) = guard.tick(Instant::now()) {
                    let _ = fresh.set_nonblocking(true);
                    listener = fresh;
                    log::info!("Bar listener rebound to {}", socket_path);
                }
                thread::sleep(ACCEPT_POLL);
            });
        }

        while let Some(msg) = rx.blocking_recv() {
            let mut payload = serde_json::to_vec(&msg).unwrap_or_default();
            payload.push(b'\n');
            match &msg {
                BarMessage::State(_) => {
                    *last_state.lock().unwrap() = Some(payload.clone());
                }
                BarMessage::Exit => {}
            }

            // Broadcast without holding the lock during I/O — clone the senders,
            // then try_send to each. Slow clients drop payloads (bounded 32), but
            // don't block others. Closed senders are pruned.
            let senders = clients.lock().unwrap().clone();
            let mut any_closed = false;
            for tx in &senders {
                if tx.try_send(payload.clone()).is_err() {
                    // Channel full or closed — drop this payload for this
                    // client (menubar catches up on next State push). If
                    // closed, prune.
                    if tx.is_closed() {
                        any_closed = true;
                    }
                }
            }
            if any_closed {
                clients.lock().unwrap().retain(|tx| !tx.is_closed());
            }
        }
    });
    BarSender { tx }
}
