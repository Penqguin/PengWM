mod connection;
#[cfg(target_os = "macos")]
mod macos;

use std::sync::{Arc, Mutex};

use pengwm_core::command::BarState;

fn main() {
    env_logger::init();

    // Single-instance guard: the daemon respawns the menubar at startup, and
    // an orphan menubar from a killed daemon can outlive it (it reconnects to
    // the next daemon's socket, so nothing asks it to leave). Without this,
    // one stray process = a second status icon. The lock is held for the
    // process lifetime; losing the try_lock means an instance already runs.
    let lock_path = std::env::temp_dir().join("pengwm-menubar.lock");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .and_then(|f| {
            if f.try_lock().is_ok() {
                Ok(f)
            } else {
                Err(std::io::Error::other("already locked"))
            }
        });
    match file {
        // Keep the file handle alive: forgetting it means the lock lasts
        // until the process exits.
        Ok(file) => std::mem::forget(file),
        Err(e) => {
            log::info!("another pengwm-menubar holds its instance lock ({}); exiting", e);
            return;
        }
    }

    let state: Arc<Mutex<Option<BarState>>> = Arc::new(Mutex::new(None));

    {
        let state = Arc::clone(&state);
        std::thread::spawn(move || connection::subscribe(state));
    }

    #[cfg(target_os = "macos")]
    {
        macos::run(state);
    }
    #[cfg(not(target_os = "macos"))]
    {
        log::error!("pengwm-menubar is macOS-only");
        std::process::exit(1);
    }
}
