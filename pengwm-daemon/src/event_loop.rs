use crate::adapter::OsAdapter;
use crate::adapter_macos::MacOsAdapter;
use crate::bar_server::{spawn_bar_server, BarSender};
use crate::config::keybinds::KeybindConfig;
use crate::state::StateManager;
use core_foundation::runloop::{kCFRunLoopDefaultMode, CFRunLoopRunInMode};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

#[derive(Debug)]
pub enum DaemonEvent {
    WindowCreated(pengwm_core::tree::WindowId, i32),
    WindowDestroyed(pengwm_core::tree::WindowId),
    WindowFocused(pengwm_core::tree::WindowId),
    WindowMoved(pengwm_core::tree::WindowId, f64, f64),
    WindowHidden(pengwm_core::tree::WindowId),
    WindowShown(pengwm_core::tree::WindowId),

    AppLaunched(i32),
    AppTerminated(i32),
    AppActivated(i32),

    MonitorAdded(u32),
    MonitorRemoved(u32),
    MonitorResized(u32),

    /// A `Command` from any source. `Some(tx)` when the caller expects a
    /// `DaemonResponse` (the CLI/IPC client); `None` for fire-and-forget
    /// sources (keybinds, config watcher) that don't get a reply slot.
    Command(
        pengwm_core::command::Command,
        Option<mpsc::Sender<pengwm_core::command::DaemonResponse>>,
    ),
}

pub struct EventLoop {
    rx: mpsc::Receiver<DaemonEvent>,
    state: StateManager,
}

impl EventLoop {
    pub fn new(keybinds: Arc<Mutex<KeybindConfig>>) -> (Self, mpsc::Sender<DaemonEvent>) {
        let (tx, rx) = mpsc::channel(256);
        let event_tx = tx.clone();
        let os: Box<dyn OsAdapter> =
            Box::new(MacOsAdapter::with_callback(Box::new(move |event| {
                let _ = event_tx.try_send(event);
            })));
        Self::new_with_adapter(keybinds, os, rx, tx.clone())
    }

    /// Test seam: inject a pre-built adapter (e.g. `TestAdapter`) so the loop
    /// can be exercised without macOS FFI. The bar/menubar spawn is still
    /// gated on `Settings::load()` so tests can control it via the config file.
    pub fn new_with_adapter(
        keybinds: Arc<Mutex<KeybindConfig>>,
        os: Box<dyn OsAdapter>,
        rx: mpsc::Receiver<DaemonEvent>,
        tx: mpsc::Sender<DaemonEvent>,
    ) -> (Self, mpsc::Sender<DaemonEvent>) {
        let bar_sender: BarSender = spawn_bar_server();
        let settings = crate::config::Settings::load();
        let bar_pid = if settings.bar.enabled {
            spawn_bar_process()
        } else {
            log::info!("bar.enabled=false — not spawning pengwm-bar");
            None
        };
        let menubar_pid = if settings.menubar.enabled {
            spawn_menubar_process()
        } else {
            log::info!("menubar.enabled=false — not spawning pengwm-menubar");
            None
        };
        let state = StateManager::new(
            tx.clone(),
            keybinds,
            os,
            bar_sender,
            bar_pid,
            menubar_pid.into_iter().collect::<Vec<_>>(),
        );
        (Self { rx, state }, tx)
    }

    /// Run one iteration of the event loop: let the CFRunLoop process one source,
    /// then drain all queued mpsc messages. Returns `false` if the channel is
    /// disconnected or a shutdown was requested.
    pub fn pump(&mut self) -> bool {
        unsafe {
            CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.05, 1);
            loop {
                match self.rx.try_recv() {
                    Ok(event) => {
                        self.dispatch(event);
                        if self.state.shutdown_requested() {
                            return false;
                        }
                    }
                    Err(mpsc::error::TryRecvError::Empty) => {
                        self.state.on_tick();
                        return !self.state.shutdown_requested();
                    }
                    Err(mpsc::error::TryRecvError::Disconnected) => {
                        log::error!("Event loop channel closed");
                        return false;
                    }
                }
            }
        }
    }

    /// Run the event loop synchronously on the current thread.
    ///
    /// Dispatches macOS events from the CFRunLoop (AXObserver, CGEventTap, NSWorkspace)
    /// and drains the mpsc channel between each run loop iteration. Returns when
    /// the channel disconnects or a shutdown was requested (`Command::Quit`).
    pub fn run_sync(&mut self) {
        while self.pump() {}
    }

    fn dispatch(&mut self, event: DaemonEvent) {
        match event {
            DaemonEvent::WindowCreated(id, pid) => self.state.on_window_created(id, pid),
            DaemonEvent::WindowDestroyed(id) => self.state.on_window_destroyed(id),
            DaemonEvent::WindowFocused(id) => self.state.on_window_focused(id),
            DaemonEvent::WindowMoved(id, x, y) => self.state.on_window_moved(id, x, y),
            DaemonEvent::WindowHidden(id) => self.state.on_window_hidden(id),
            DaemonEvent::WindowShown(id) => self.state.on_window_shown(id),
            DaemonEvent::AppLaunched(pid) => self.state.on_app_launched(pid),
            DaemonEvent::AppTerminated(pid) => self.state.on_app_terminated(pid),
            DaemonEvent::AppActivated(pid) => self.state.on_app_activated(pid),
            DaemonEvent::MonitorAdded(id) => self.state.on_monitor_added(id),
            DaemonEvent::MonitorRemoved(id) => self.state.on_monitor_removed(id),
            DaemonEvent::MonitorResized(id) => self.state.on_monitor_resized(id),
            DaemonEvent::Command(cmd, rtx) => self.state.on_command(cmd, rtx),
        }
    }
}

fn candidate_paths(binary: &str, env_var: &str) -> Vec<std::path::PathBuf> {
    let mut v = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            v.push(dir.join(binary));
        }
    }
    if let Ok(path) = std::env::var(env_var) {
        v.push(path.into());
    }
    v.push(binary.into());
    v
}

/// Launch the `pengwm-bar` binary as a child process and return its pid so the
/// WM can exclude its window. Prefers a sibling of the current executable,
/// then `$PATH`, then `PENGWM_BAR_PATH`. Returns `None` when no candidate runs.
fn spawn_bar_process() -> Option<i32> {
    spawn_child_process(
        "pengwm-bar",
        &candidate_paths("pengwm-bar", "PENGWM_BAR_PATH"),
    )
}

/// Launch the `pengwm-menubar` binary as a child process and return its pid so
/// the WM can exclude its window. Prefers a sibling of the current executable,
/// then `$PATH`, then `PENGWM_MENUBAR_PATH`. Returns `None` when no candidate
/// runs.
fn spawn_menubar_process() -> Option<i32> {
    spawn_child_process(
        "pengwm-menubar",
        &candidate_paths("pengwm-menubar", "PENGWM_MENUBAR_PATH"),
    )
}

/// Try each candidate in order and spawn the first that runs, returning its
/// pid. Reaps the child when the daemon exits so it doesn't linger.
fn spawn_child_process(name: &str, candidates: &[std::path::PathBuf]) -> Option<i32> {
    for candidate in candidates {
        match std::process::Command::new(candidate).spawn() {
            Ok(mut child) => {
                let pid = child.id() as i32;
                log::info!(
                    "Spawned {} (pid {}) from {}",
                    name,
                    pid,
                    candidate.display()
                );
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                return Some(pid);
            }
            Err(e) => {
                log::debug!(
                    "Could not spawn {} from {}: {}",
                    name,
                    candidate.display(),
                    e
                );
            }
        }
    }
    log::warn!(
        "{} not found. Install it next to pengwm or set its _PATH env var.",
        name
    );
    None
}
