use eframe::egui;
use pengwm_core::command::{BarMessage, Command};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::mpsc::Sender;
use std::time::Duration;

pub const BAR_SOCKET_PATH: &str = "/tmp/pengwm-bar.sock";
pub const COMMAND_SOCKET_PATH: &str = "/tmp/pengwm.sock";

const MAX_BACKOFF: Duration = Duration::from_secs(2);
const INITIAL_BACKOFF: Duration = Duration::from_millis(250);

/// Block forever: keep a connection to the bar socket open, parsing
/// newline-delimited `BarMessage`s and forwarding them to `tx`, repainting the
/// UI after each one. Reconnects with exponential backoff when the daemon is
/// down or restarts.
pub fn subscribe(tx: Sender<BarMessage>, ctx: egui::Context) {
    let mut backoff = INITIAL_BACKOFF;
    loop {
        match read_messages(&tx, &ctx) {
            Ok(()) => {
                backoff = INITIAL_BACKOFF;
            }
            Err(e) => {
                log::debug!("bar connection: {e}");
                std::thread::sleep(backoff);
                backoff = (backoff * 2).min(MAX_BACKOFF);
            }
        }
    }
}

fn read_messages(tx: &Sender<BarMessage>, ctx: &egui::Context) -> Result<(), String> {
    let stream = UnixStream::connect(BAR_SOCKET_PATH).map_err(|e| format!("connect: {e}"))?;
    log::info!("connected to bar socket {}", BAR_SOCKET_PATH);

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader
            .read_line(&mut line)
            .map_err(|e| format!("read: {e}"))?;
        if n == 0 {
            return Err("bar socket closed".into());
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match serde_json::from_str::<BarMessage>(trimmed) {
            Ok(msg) => {
                if tx.send(msg).is_ok() {
                    ctx.request_repaint();
                } else {
                    return Err("UI channel closed".into());
                }
            }
            Err(e) => log::warn!("skipping malformed bar message: {e}"),
        }
    }
}

/// Send a `Command` to the daemon over the command socket (same wire format as
/// `pengwm`'s own CLI client). Used for click-to-switch-workspace.
pub fn send_command(cmd: &Command) -> Result<String, String> {
    let body = serde_json::to_string(cmd).map_err(|e| format!("serialize: {e}"))?;

    let mut stream = UnixStream::connect(COMMAND_SOCKET_PATH)
        .map_err(|e| format!("could not connect to daemon at {COMMAND_SOCKET_PATH}: {e}"))?;
    stream
        .write_all(body.as_bytes())
        .map_err(|e| format!("write: {e}"))?;
    stream
        .shutdown(std::net::Shutdown::Write)
        .map_err(|e| format!("shutdown: {e}"))?;

    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap_or_default();
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pengwm_core::command::BarState;

    #[test]
    fn parses_newline_delimited_bar_messages() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut daemon = pengwm_daemon_bar_reader::BarMessageReader::new(tx);
        daemon.push("\"Show\"\n\"Hide\"\n");
        daemon.push(
            "{\"State\":{\"workspaces\":[],\"active_workspace\":0,\"split_direction\":null,\"rect\":null}}\n",
        );
        daemon.push("garbage\n");
        daemon.finish();

        let mut received = Vec::new();
        while let Ok(m) = rx.try_recv() {
            received.push(m);
        }
        assert_eq!(
            received.len(),
            3,
            "valid lines parse, malformed lines are skipped"
        );
        assert!(matches!(received[0], BarMessage::Show));
        assert!(matches!(received[1], BarMessage::Hide));
        assert!(matches!(&received[2], BarMessage::State(s) if s.workspaces.is_empty()));
    }

    #[test]
    fn command_serializes_as_json() {
        let cmd = Command::Workspace { id: 3 };
        let json = serde_json::to_string(&cmd).unwrap();
        assert_eq!(json, r#"{"Workspace":{"id":3}}"#);
        let back: Command = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, Command::Workspace { id: 3 }));
    }

    #[test]
    fn bar_state_with_rect_deserializes() {
        let s = r#"{"workspaces":[],"active_workspace":0,"split_direction":null,"rect":{"x":0.0,"y":24.0,"width":1920.0,"height":32.0}}"#;
        let state: BarState = serde_json::from_str(s).unwrap();
        assert_eq!(state.rect.unwrap().y, 24.0);
    }
}

/// Small test-only helper that decodes the same newline-delimited protocol
/// without a real socket.
#[cfg(test)]
mod pengwm_daemon_bar_reader {
    use super::*;

    pub struct BarMessageReader {
        tx: Sender<BarMessage>,
        pending: String,
    }

    impl BarMessageReader {
        pub fn new(tx: Sender<BarMessage>) -> Self {
            Self {
                tx,
                pending: String::new(),
            }
        }

        pub fn push(&mut self, bytes: &str) {
            self.pending.push_str(bytes);
            self.drain();
        }

        pub fn finish(&mut self) {
            self.drain();
        }

        fn drain(&mut self) {
            while let Some(idx) = self.pending.find('\n') {
                let line: String = self.pending.drain(..=idx).collect();
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if let Ok(msg) = serde_json::from_str::<BarMessage>(trimmed) {
                    let _ = self.tx.send(msg);
                }
            }
        }
    }
}
