use crate::command::Command;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

/// Command socket: one JSON `Command` per connection, one `DaemonResponse` back.
pub const COMMAND_SOCKET_PATH: &str = "/tmp/pengwm.sock";
/// Bar socket: newline-delimited `BarMessage` push stream, no response.
pub const BAR_SOCKET_PATH: &str = "/tmp/pengwm-bar.sock";

/// Send a `Command` to the daemon over the command socket and read the raw
/// `DaemonResponse` JSON back. Shared by the `pengwm` CLI client and the bar's
/// click-to-switch handler so the wire protocol has one implementation.
pub fn send_command(cmd: &Command) -> Result<String, String> {
    send_command_at(cmd, COMMAND_SOCKET_PATH)
}

/// Like [`send_command`], but against an explicit socket path (used by tests).
pub fn send_command_at(cmd: &Command, socket_path: &str) -> Result<String, String> {
    const RESPONSE_TIMEOUT: Duration = Duration::from_secs(3);

    let body = serde_json::to_string(cmd).map_err(|e| format!("serialize error: {e}"))?;

    let mut stream = UnixStream::connect(socket_path).map_err(|e| {
        format!("could not connect to daemon at {socket_path}: {e}\n       is pengwm running?")
    })?;

    stream
        .write_all(body.as_bytes())
        .map_err(|e| format!("write error: {e}"))?;
    stream
        .shutdown(std::net::Shutdown::Write)
        .map_err(|e| format!("shutdown error: {e}"))?;
    // Bound the wait: the daemon's reply is written right after the command
    // is dequeued, so 3s is generous for a healthy daemon — and a wedged one
    // (stalled event loop) must not freeze callers forever, especially the
    // menubar's Quit flow on the main thread.
    stream
        .set_read_timeout(Some(RESPONSE_TIMEOUT))
        .map_err(|e| format!("read timeout error: {e}"))?;

    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap_or_default();
    if response.is_empty() {
        return Err("daemon did not respond in time (wedged?)".to_string());
    }
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    #[test]
    fn send_command_roundtrips_over_socket() {
        let path =
            std::env::temp_dir().join(format!("pengwm-ipc-test-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let path_str = path.to_str().unwrap().to_string();
        let listener = UnixListener::bind(&path).unwrap();

        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = vec![0u8; 4096];
            let n = stream.read(&mut buf).unwrap();
            let cmd: Command = serde_json::from_slice(&buf[..n]).unwrap();
            assert!(matches!(cmd, Command::Workspace { id: 3 }));
            stream.write_all(br#"{"Ack":{}}"#).unwrap();
            stream.shutdown(std::net::Shutdown::Write).unwrap();
        });

        let resp = send_command_at(&Command::Workspace { id: 3 }, &path_str).unwrap();
        assert!(resp.contains("Ack"));
        server.join().unwrap();
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn send_command_times_out_against_a_wedged_daemon() {
        let path =
            std::env::temp_dir().join(format!("pengwm-ipc-wedge-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let path_str = path.to_str().unwrap().to_string();
        let listener = UnixListener::bind(&path).unwrap();

        // Accept and never answer: a stalled daemon. The client must fail
        // bounded (read timeout), never hang — the menubar's Quit runs on
        // its main thread through this function.
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            std::mem::forget(stream); // hold the connection open, never write
        });

        let started = std::time::Instant::now();
        let result = send_command_at(&Command::QueryState, &path_str);
        assert!(result.is_err(), "wedged daemon must produce an error");
        assert!(
            started.elapsed() < std::time::Duration::from_secs(10),
            "client must fail bounded, got {:?}",
            started.elapsed()
        );
        server.join().unwrap();
        let _ = std::fs::remove_file(&path);
    }
}
