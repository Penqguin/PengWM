use std::process::Command;

use pengwm_daemon::ipc_server;

/// A socket the CLI child process targets instead of the real daemon path.
/// Bind-first probe-then-bind (see `ipc_server`) protects a *running* daemon,
/// but this test used to `remove_file(DEFAULT_SOCKET_PATH)` before it —
/// which, with a live dev daemon up, unlinked the live socket (listener
/// orphaned on an unlinked vnode: WM healthy, every CLI call ECONNREFUSED),
/// then rebound the real path for the scope of the test. The CLI binds
/// `command_socket_path()` at connect time, so the test points the child at
/// a disposable path instead — per-test socket path ownership, like the
/// other integration tests.
fn test_socket_path() -> String {
    std::env::temp_dir()
        .join(format!("pengwm_cli_test-{}.sock", std::process::id()))
        .to_str()
        .unwrap()
        .to_string()
}

#[test]
fn cli_binary_dispatches_to_server() {
    let socket_path = test_socket_path();
    let _ = std::fs::remove_file(&socket_path);
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let st = tx.clone();
    let server_socket = socket_path.clone();
    std::thread::spawn(move || ipc_server::start_ipc_server_with_path(st, &server_socket));
    // Server readiness is the bound socket file, not a sleep.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !std::path::Path::new(&socket_path).exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "test ipc server did not bind {socket_path}"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    let server = std::thread::spawn(move || {
        if let Some(pengwm_daemon::event_loop::DaemonEvent::Command(_cmd, resp_tx)) =
            rx.blocking_recv()
        {
            let _ = resp_tx
                .expect("IPC command carries a reply slot")
                .try_send(pengwm_core::command::DaemonResponse::Ack);
        }
    });

    let out = Command::new(env!("CARGO_BIN_EXE_pengwm"))
        .args(["focus", "left"])
        .env("PENGWM_COMMAND_SOCKET", &socket_path)
        .output()
        .expect("run pengwm");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "\"Ack\"");
    server.join().unwrap();
    let _ = std::fs::remove_file(&socket_path);
}
