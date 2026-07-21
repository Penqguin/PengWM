//! CGEventTap for global keyboard shortcuts.
//!
//! Creates an event tap that intercepts key-down events before they reach any
//! application. If the key combination matches a configured keybind, the event
//! is swallowed and a DaemonEvent::Keybind is sent into the event loop.
//! Otherwise the event passes through normally.

use tokio::sync::mpsc;
use crate::event_loop::DaemonEvent;

/// Start the CGEventTap on a background thread.
///
/// # Arguments
/// * `event_tx` — sender to forward matched keybinds.
/// * `keybinds` — the parsed keybind config (Vec<(KeyCode, ModifierFlags, DaemonCommand)>)
///
/// The event tap runs on its own CFRunLoop thread. It never exits.
pub fn start(event_tx: mpsc::Sender<DaemonEvent>, keybinds: Vec<(u16, u64, DaemonCommand)>) {
    //  create CGEventMask for kCGEventKeyDown
    //  CGEventTapCreate(kCGSessionEventTap, kCGHeadInsertEventTap, ..., eventMask, callback, refcon)
    //  CFRunLoopSource from the tap
    //  add to current CFRunLoop
    //  run the run loop
    todo!("start event tap")
}

/// The C callback invoked for each keyboard event.
///
/// # Safety
/// Called from CG's event tap thread.
unsafe extern "C" fn event_tap_callback(
    _proxy: *mut std::ffi::c_void,
    _type: u32,
    event: *mut std::ffi::c_void,
    refcon: *mut std::ffi::c_void,
) -> *mut std::ffi::c_void {
    //  CGEventGetIntegerValueField(event, kCGKeyboardEventKeycode) -> keycode
    //  CGEventGetFlags(event) -> modifiers
    //  look up (keycode, modifiers) in the keybinds list
    //  if match found:
    //       send DaemonEvent::Keybind(command) into refcon's mpsc sender
    //       return null (swallow event)
    //  else:
    //       return event (pass through)
    todo!("handle key event")
}
