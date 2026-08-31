mod connection;
#[cfg(target_os = "macos")]
mod macos;

use std::sync::{Arc, Mutex};

use pengwm_core::command::BarState;

fn main() {
    env_logger::init();

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
