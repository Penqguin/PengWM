//! Unix Domain Socket listener.
//!
//! Binds to /tmp/pengwm.sock (or $XDG_RUNTIME_DIR/pengwm.sock).
//! Accepts incoming JSON messages, deserializes them as DaemonCommand,
//! and forwards them into the event loop via an mpsc channel.

use tokio::net::UnixListener;
use tokio::sync::mpsc;
use crate::event_loop::DaemonEvent;

/// Path where the daemon listens for CLI connections.
const SOCKET_PATH: &str = "/tmp/pengwm.sock";

/// Start the UDS listener in a background tokio task.
///
/// # Arguments
/// * `event_tx` — clone of the event loop's mpsc sender.
pub async fn listen(event_tx: mpsc::Sender<DaemonEvent>) {
    //  remove old socket file if it exists (std::fs::remove_file)
    //  bind UnixListener to SOCKET_PATH
    //  loop:
    //    accept() -> stream
    //    spawn a handler task per connection:
    //      - read incoming bytes
    //      - serde_json::from_slice::<DaemonCommand>
    //      - create a oneshot response channel
    //      - send DaemonEvent::Command(cmd, tx) into event_tx
    //      - await response and write it back to the stream
    todo!("UDS listener")
}
