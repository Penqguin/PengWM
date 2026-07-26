use std::io::Write;
use std::os::unix::net::UnixStream;
use std::thread;
use std::time::Duration;

use tokio::sync::mpsc;

use pengwm_core::command::Command;
use pengwm_core::tree::SplitDirection;
use pengwm_daemon::event_loop::DaemonEvent;
use pengwm_daemon::ipc_server;

const TEST_SOCKET: &str = "/tmp/pengwm_test.sock";

#[test]
fn ipc_send_command_receives_event() {
    let _ = std::fs::remove_file(TEST_SOCKET);

    let (event_tx, mut event_rx) = mpsc::channel(256);

    // Start the IPC server on a background thread with a test socket.
    let server_tx = event_tx.clone();
    thread::spawn(move || {
        ipc_server::start_ipc_server_with_path(server_tx, TEST_SOCKET);
    });

    // Give the server a moment to bind.
    thread::sleep(Duration::from_millis(100));

    // Connect as a CLI client and send a Split command.
    let cmd = Command::Split {
        direction: SplitDirection::Horizontal,
    };
    let body = serde_json::to_string(&cmd).unwrap();

    let mut stream = UnixStream::connect(TEST_SOCKET).expect("connect to test socket");
    stream.write_all(body.as_bytes()).unwrap();
    stream.shutdown(std::net::Shutdown::Write).unwrap();

    // Read the event off the mpsc channel (blocks until received).
    let event = event_rx.blocking_recv().expect("channel closed before event received");

    match event {
        DaemonEvent::Command(received_cmd, resp_tx) => {
            assert!(
                matches!(received_cmd, Command::Split { direction: SplitDirection::Horizontal }),
                "expected Split(Horizontal), got {:?}",
                received_cmd
            );
            // Send an Ack so the server thread doesn't hang.
            let _ = resp_tx.try_send(pengwm_core::command::DaemonResponse::Ack);
        }
        other => panic!("expected DaemonEvent::Command, got {:?}", other),
    }

    // Clean up.
    let _ = std::fs::remove_file(TEST_SOCKET);
}
