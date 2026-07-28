use std::io::{Read, Write};
use std::os::unix::net::UnixStream;

use pengwm_core::command::Command;

pub const SOCKET_PATH: &str = "/tmp/pengwm.sock";

pub fn send_command(cmd: &Command) -> Result<String, String> {
    let body = serde_json::to_string(cmd).map_err(|e| format!("serialize error: {e}"))?;

    let mut stream = UnixStream::connect(SOCKET_PATH).map_err(|e| {
        format!(
            "could not connect to daemon at {SOCKET_PATH}: {e}\n       is pengwm-daemon running?"
        )
    })?;

    stream
        .write_all(body.as_bytes())
        .map_err(|e| format!("write error: {e}"))?;
    stream
        .shutdown(std::net::Shutdown::Write)
        .map_err(|e| format!("shutdown error: {e}"))?;

    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap_or_default();
    Ok(response)
}
