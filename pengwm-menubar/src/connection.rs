use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pengwm_core::command::{BarMessage, BarState};
use pengwm_core::ipc::BAR_SOCKET_PATH;

const INITIAL_BACKOFF: Duration = Duration::from_millis(250);
const MAX_BACKOFF: Duration = Duration::from_secs(2);
/// How long the daemon may be unreachable before the menubar gives up and
/// exits. Covers `pengwm quit`, a daemon crash, and the menubar's own Quit
/// flow; the daemon respawns the menubar on its next start.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(10);

/// Blocking subscribe loop. Connects to the daemon's bar socket and stashes the
/// latest `BarState` into `state`. The menu is rebuilt from that shared state on
/// every open (`NSMenuDelegate::menuWillOpen`), so no UI work happens here.
/// While the daemon is down the snapshot is cleared and, after a grace period,
/// the menubar exits so no orphaned icon lingers without a daemon.
pub fn subscribe(state: Arc<Mutex<Option<BarState>>>) {
    let mut backoff = INITIAL_BACKOFF;
    let mut down_since: Option<Instant> = None;
    loop {
        match connect() {
            Ok(stream) => {
                log::info!("menubar connected to bar socket {BAR_SOCKET_PATH}");
                // The daemon is reachable again; give it a fresh grace window.
                down_since = None;
                backoff = INITIAL_BACKOFF;
                if let Err(e) = read_messages(stream, &state) {
                    log::debug!("menubar bar connection: {e}");
                    *state.lock().unwrap() = None;
                    std::thread::sleep(backoff);
                    backoff = (backoff * 2).min(MAX_BACKOFF);
                }
            }
            Err(e) => {
                log::debug!("menubar bar connection: {e}");
                // Drop the last snapshot so the menu reports "Daemon not
                // running" while the daemon is down instead of stale data.
                // The bar server replays its cached state on reconnect.
                *state.lock().unwrap() = None;
                let now = Instant::now();
                let down = down_since.get_or_insert(now);
                if now.duration_since(*down) >= SHUTDOWN_GRACE {
                    log::info!("daemon unreachable for {SHUTDOWN_GRACE:?}; exiting menubar");
                    std::process::exit(0);
                }
                std::thread::sleep(backoff);
                backoff = (backoff * 2).min(MAX_BACKOFF);
            }
        }
    }
}

fn connect() -> Result<UnixStream, String> {
    UnixStream::connect(BAR_SOCKET_PATH).map_err(|e| format!("connect {BAR_SOCKET_PATH}: {e}"))
}

fn read_messages(stream: UnixStream, state: &Arc<Mutex<Option<BarState>>>) -> Result<(), String> {
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
            Ok(BarMessage::State(s)) => {
                *state.lock().unwrap() = Some(s);
            }
            Ok(_) => {}
            Err(e) => log::warn!("menubar skipping malformed bar message: {e}"),
        }
    }
}
