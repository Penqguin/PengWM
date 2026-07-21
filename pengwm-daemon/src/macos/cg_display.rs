//! CoreGraphics display management.
//!
//! Handles:
//!   - Enumerating active displays at startup
//!   - Registering CGDisplayRegisterReconfigurationCallback for hot-plug
//!   - Querying display bounds (origin + size in global coordinates)
//!   - Converting between local and global coordinate spaces

use tokio::sync::mpsc;
use crate::event_loop::DaemonEvent;

// ---------------------------------------------------------------------------
// Display info
// ---------------------------------------------------------------------------

/// A physical display with its global frame.
pub struct DisplayInfo {
    pub id: u32,  // CGDirectDisplayID
    pub origin: (i32, i32),
    pub size: (u32, u32),
}

/// Query all active displays and return their info.
pub fn active_displays() -> Vec<DisplayInfo> {
    //  CGGetActiveDisplayList(max, list, &count)
    //  for each display:
    //    CGDisplayBounds(display) -> CGRect { origin, size }
    //  return Vec<DisplayInfo>
    todo!()
}

/// Return the primary display's ID (the one with origin (0,0)).
pub fn primary_display_id() -> u32 {
    todo!("CGMainDisplayID()")
}

// ---------------------------------------------------------------------------
// Hot-plug callback
// ---------------------------------------------------------------------------

/// Register a callback that fires when displays are added, removed, or resized.
///
/// The C callback extracts the CGDirectDisplayID and the reconfiguration type,
/// then sends DaemonEvent::MonitorAdded / MonitorRemoved / MonitorResized.
pub fn register_hotplug_callback(event_tx: mpsc::Sender<DaemonEvent>) {
    //  let ctx = Box::into_raw(Box::new(event_tx)) as *mut c_void
    //  CGDisplayRegisterReconfigurationCallback(display_reconfig_callback, ctx)
    //
    //  NOTE: Must keep ctx alive. Leak it intentionally (daemon lifetime).
    todo!()
}

/// C callback invoked by CoreGraphics on display changes.
///
/// # Safety
/// Called from CG on its own thread.
unsafe extern "C" fn display_reconfig_callback(
    display: u32,
    flags: u32,
    refcon: *mut std::ffi::c_void,
) {
    //  match flags:
    //    kCGDisplayAddFlag        → MonitorAdded(display)
    //    kCGDisplayRemoveFlag     → MonitorRemoved(display)
    //    kCGDisplaySetModeFlag    → MonitorResized(display)
    //  send into event_tx via refcon
    todo!("handle display reconfiguration")
}
