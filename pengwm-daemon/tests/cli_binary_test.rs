use std::process::Command;

use pengwm_daemon::ipc_server;

#[test]
fn cli_binary_dispatches_to_server() {
    let _ = std::fs::remove_file(ipc_server::DEFAULT_SOCKET_PATH);
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let st = tx.clone();
    std::thread::spawn(move || ipc_server::start_ipc_server(st));
    std::thread::sleep(std::time::Duration::from_millis(150));

    let server = std::thread::spawn(move || {
        if let Some(pengwm_daemon::event_loop::DaemonEvent::Command(_cmd, resp_tx)) =
            rx.blocking_recv()
        {
            let _ = resp_tx.try_send(pengwm_core::command::DaemonResponse::Ack);
        }
    });

    let out = Command::new(env!("CARGO_BIN_EXE_pengwm"))
        .args(["focus", "left"])
        .output()
        .expect("run pengwm");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "\"Ack\"");
    server.join().unwrap();
}
