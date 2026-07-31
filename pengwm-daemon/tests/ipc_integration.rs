use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::thread;
use std::time::Duration;

use tokio::sync::mpsc;

use pengwm_core::command::{BarMessage, BarState, BarWorkspace, Command, DaemonResponse};
use pengwm_core::tree::SplitDirection;
use pengwm_daemon::event_loop::DaemonEvent;
use pengwm_daemon::ipc_server;

const TEST_SOCKET: &str = "/tmp/pengwm_test.sock";
const TEST_SOCKET_2: &str = "/tmp/pengwm_test2.sock";
const TEST_BAR_SOCKET: &str = "/tmp/pengwm_test_bar.sock";

#[test]
fn ipc_send_command_receives_event() {
    let _ = std::fs::remove_file(TEST_SOCKET);
    let _ = std::fs::remove_file(TEST_SOCKET_2);

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
    let event = event_rx
        .blocking_recv()
        .expect("channel closed before event received");

    match event {
        DaemonEvent::Command(received_cmd, resp_tx) => {
            assert!(
                matches!(
                    received_cmd,
                    Command::Split {
                        direction: SplitDirection::Horizontal
                    }
                ),
                "expected Split(Horizontal), got {:?}",
                received_cmd
            );
            // Simulate the state manager: the daemon acks every handled command.
            resp_tx.try_send(DaemonResponse::Ack).unwrap();
        }
        other => panic!("expected DaemonEvent::Command, got {:?}", other),
    }

    // The client should receive the ack and then EOF.
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert_eq!(response, "\"Ack\"", "client should receive Ack JSON");

    // Clean up.
    let _ = std::fs::remove_file(TEST_SOCKET);
}

#[test]
fn cli_roundtrip_receives_ack() {
    let _ = std::fs::remove_file(TEST_SOCKET_2);

    let (event_tx, mut event_rx) = mpsc::channel(256);

    // Start the IPC server on a background thread with a test socket.
    let server_tx = event_tx.clone();
    thread::spawn(move || {
        ipc_server::start_ipc_server_with_path(server_tx, TEST_SOCKET_2);
    });
    thread::sleep(Duration::from_millis(100));

    // Spawn a consumer that mirrors StateManager::on_command: handle the
    // command and send an Ack back through the response channel.
    let consumer = thread::spawn(move || {
        if let Some(DaemonEvent::Command(_cmd, resp_tx)) = event_rx.blocking_recv() {
            let _ = resp_tx.try_send(DaemonResponse::Ack);
        }
    });

    // Send a command the same way the CLI does, then read the response.
    let body = serde_json::to_string(&Command::Focus {
        direction: pengwm_core::tree::Direction::Right,
    })
    .unwrap();
    let mut stream = UnixStream::connect(TEST_SOCKET_2).expect("connect to test socket");
    stream.write_all(body.as_bytes()).unwrap();
    stream.shutdown(std::net::Shutdown::Write).unwrap();

    let mut response = String::new();
    let read_ok = stream.read_to_string(&mut response).is_ok();
    assert!(read_ok, "client should receive a response, not EOF");
    assert_eq!(
        response, "\"Ack\"",
        "non-QueryState commands should be acked"
    );

    consumer.join().unwrap();
    let _ = std::fs::remove_file(TEST_SOCKET_2);
}

#[test]
fn bar_socket_receives_cached_state_on_connect() {
    let _ = std::fs::remove_file(TEST_BAR_SOCKET);

    let sender = pengwm_daemon::bar_server::spawn_bar_server_with_path(TEST_BAR_SOCKET);
    thread::sleep(Duration::from_millis(100));

    let state = BarState {
        workspaces: vec![BarWorkspace {
            name: "ws-1".into(),
            monitor_id: 1,
            window_count: 2,
            active: true,
        }],
        active_workspace: 0,
        split_direction: Some(SplitDirection::Vertical),
        rect: None,
    };
    sender.send(BarMessage::State(state));
    thread::sleep(Duration::from_millis(100));

    let mut stream = UnixStream::connect(TEST_BAR_SOCKET).expect("connect to bar socket");
    stream
        .set_read_timeout(Some(Duration::from_millis(500)))
        .unwrap();
    let mut buf = [0u8; 4096];
    let n = stream.read(&mut buf).expect("read cached message");
    let line = String::from_utf8_lossy(&buf[..n]);
    let msg: BarMessage = serde_json::from_str(line.trim()).expect("valid JSON message");
    assert!(
        matches!(&msg, BarMessage::State(s) if s.workspaces[0].name == "ws-1"),
        "bar should receive the last broadcast State on connect, got {:?}",
        msg
    );

    // Broadcast a follow-up and read it on the live connection.
    sender.send(BarMessage::Hide);
    let mut buf = [0u8; 4096];
    let n = stream.read(&mut buf).expect("read follow-up message");
    let line = String::from_utf8_lossy(&buf[..n]);
    let msg: BarMessage = serde_json::from_str(line.trim()).expect("valid JSON message");
    assert!(
        matches!(msg, BarMessage::Hide),
        "bar should receive live broadcasts, got {:?}",
        msg
    );

    let _ = std::fs::remove_file(TEST_BAR_SOCKET);
}
