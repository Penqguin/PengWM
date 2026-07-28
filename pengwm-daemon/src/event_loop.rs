use crate::adapter::OsAdapter;
use crate::adapter_macos::MacOsAdapter;
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

    AppLaunched(i32),
    AppTerminated(i32),
    AppActivated(i32),

    MonitorAdded(u32),
    MonitorRemoved(u32),
    MonitorResized(u32),

    Command(
        pengwm_core::command::Command,
        mpsc::Sender<pengwm_core::command::DaemonResponse>,
    ),

    Keybind(pengwm_core::command::Command),
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
        let state = StateManager::new(tx.clone(), keybinds, os);
        (Self { rx, state }, tx)
    }

    /// Run one iteration of the event loop: let the CFRunLoop process one source,
    /// then drain all queued mpsc messages. Returns `false` if the channel is
    /// disconnected.
    pub fn pump(&mut self) -> bool {
        unsafe {
            CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.05, 1);
            loop {
                match self.rx.try_recv() {
                    Ok(event) => self.dispatch(event),
                    Err(mpsc::error::TryRecvError::Empty) => {
                        self.state.on_tick();
                        return true;
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
    /// and drains the mpsc channel between each run loop iteration.
    pub fn run_sync(&mut self) {
        unsafe {
            loop {
                // Let the CFRunLoop process one source (event tap, AX callback, etc.)
                // with a short timeout so we can drain the mpsc channel regularly.
                CFRunLoopRunInMode(
                    kCFRunLoopDefaultMode,
                    0.05, // 50ms — balances latency vs CPU
                    1,    // returnAfterSourceHandled — return after one source is handled
                );

                // Drain all queued mpsc messages
                loop {
                    match self.rx.try_recv() {
                        Ok(event) => self.dispatch(event),
                        Err(mpsc::error::TryRecvError::Empty) => break,
                        Err(mpsc::error::TryRecvError::Disconnected) => {
                            log::error!("Event loop channel closed");
                            return;
                        }
                    }
                }
                self.state.on_tick();
            }
        }
    }

    fn dispatch(&mut self, event: DaemonEvent) {
        match event {
            DaemonEvent::WindowCreated(id, pid) => self.state.on_window_created(id, pid),
            DaemonEvent::WindowDestroyed(id) => self.state.on_window_destroyed(id),
            DaemonEvent::WindowFocused(id) => self.state.on_window_focused(id),
            DaemonEvent::WindowMoved(id, x, y) => self.state.on_window_moved(id, x, y),
            DaemonEvent::AppLaunched(pid) => self.state.on_app_launched(pid),
            DaemonEvent::AppTerminated(pid) => self.state.on_app_terminated(pid),
            DaemonEvent::AppActivated(pid) => self.state.on_app_activated(pid),
            DaemonEvent::MonitorAdded(id) => self.state.on_monitor_added(id),
            DaemonEvent::MonitorRemoved(id) => self.state.on_monitor_removed(id),
            DaemonEvent::MonitorResized(id) => self.state.on_monitor_resized(id),
            DaemonEvent::Command(cmd, rtx) => self.state.on_command(cmd, rtx),
            DaemonEvent::Keybind(cmd) => {
                let (rtx, _) = mpsc::channel(1);
                self.state.on_command(cmd, rtx);
            }
        }
    }
}
