//! macOS virtual desktop (Space) detection.
//!
//! Uses the private CGSCopyManagedDisplaySpaces API via dlsym to query
//! which spaces exist and which displays they belong to.
//!
//! This is the *one* place where we use a private API — it's required to
//! detect Spaces because there's no public alternative. The daemon still
//! does NOT require SIP to be disabled.

use tokio::sync::mpsc;
use crate::event_loop::DaemonEvent;

// ---------------------------------------------------------------------------
// Space info
// ---------------------------------------------------------------------------

/// A macOS virtual desktop (Space).
pub struct SpaceInfo {
    /// Display this space is on.
    pub display_id: u32,
    /// 0-based space index within the display.
    pub space_index: u8,
    /// The display UUID for matching across reboots.
    pub display_uuid: String,
}

// ---------------------------------------------------------------------------
// Queries
// ---------------------------------------------------------------------------

/// Return all current spaces and the windows assigned to each.
///
/// Uses CGSPrivate.h functions (loaded at runtime via dlsym to avoid
/// link-time dependency on private frameworks).
pub fn all_spaces() -> Vec<SpaceInfo> {
    //  dlsym(RTLD_DEFAULT, "CGSCopyManagedDisplaySpaces") -> fn ptr
    //  call it → CFArray of display-space mappings
    //  iterate and build SpaceInfo for each
    todo!("query Spaces via CGSCopyManagedDisplaySpaces")
}

/// Return the space index currently active on the given display.
pub fn active_space_for_display(display_id: u32) -> Option<u8> {
    //  dlsym("CGSGetActiveSpace") or similar
    todo!()
}

/// Switch to a specific space on a display.
pub fn switch_to_space(display_id: u32, space_index: u8) {
    //  CGSPrivate: CGSSetWorkspace / CGSSetManagedDisplayWorkspace
    todo!()
}
