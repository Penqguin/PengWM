//! The central mpsc-based event loop.
//!
//! All external inputs funnel into a single tokio::sync::mpsc channel:
//!
//!   - AXObserver callbacks         (window created / destroyed / focused)
//!   - NSWorkspace notifications    (app launched / activated / terminated)
//!   - CGDisplay hot-plug           (monitor added / removed)
//!   - UDS client commands          (from pengwm-cli)
//!   - CGEventTap keypresses        (global keyboard shortcuts)
//!
//! The EventLoop dispatches each event to the StateManager, which mutates
//! the workspace tree and triggers a relayout.

use tokio::sync::mpsc;
use crate::state::StateManager;

// ---------------------------------------------------------------------------
// Event enum
// ---------------------------------------------------------------------------

/// Every possible event the event loop can receive.
pub enum DaemonEvent {
    // -- macOS window lifecycle --
    WindowCreated(pengwm_core::tree::WindowId),
    WindowDestroyed(pengwm_core::tree::WindowId),
    WindowFocused(pengwm_core::tree::WindowId),

    // -- App lifecycle --
    AppLaunched(i32),
    AppTerminated(i32),
    AppActivated(i32),

    // -- Display --
    MonitorAdded(u32),
    MonitorRemoved(u32),
    MonitorResized(u32),

    // -- CLI --
    Command(pengwm_core::command::DaemonCommand, mpsc::Sender<pengwm_core::command::DaemonResponse>),

    // -- Keybind --
    Keybind(pengwm_core::command::DaemonCommand),
}

// ---------------------------------------------------------------------------
// EventLoop
// ---------------------------------------------------------------------------

/// Owns the mpsc receiver and the StateManager.
pub struct EventLoop {
    //  rx: mpsc::Receiver<DaemonEvent>
    //  state: StateManager
}

impl EventLoop {
    pub fn new() -> (Self, mpsc::Sender<DaemonEvent>) {
        //  create mpsc::channel
        //  build StateManager
        //  return (EventLoop, tx)
        todo!()
    }

    /// Run the event loop forever.
    pub async fn run(&mut self) {
        //  loop:
        //     rx.recv().await
        //     match event:
        //       WindowCreated(id)   → state.on_window_created(id)
        //       WindowDestroyed(id) → state.on_window_destroyed(id)
        //       WindowFocused(id)   → state.on_window_focused(id)
        //       AppLaunched(pid)    → state.on_app_launched(pid)
        //       AppTerminated(pid)  → state.on_app_terminated(pid)
        //       AppActivated(pid)   → state.on_app_activated(pid)
        //       MonitorAdded(id)    → state.on_monitor_added(id)
        //       MonitorRemoved(id)  → state.on_monitor_removed(id)
        //       MonitorResized(id)  → state.on_monitor_resized(id)
        //       Command(cmd, tx)    → state.on_command(cmd, tx)
        //       Keybind(cmd)        → state.on_command(cmd, ...)
        todo!("main dispatch loop")
    }
}
