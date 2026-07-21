//! Low-level wrappers around AXUIElement calls.
//!
//! These are used by StateManager to apply the computed layout.
//! Each function takes raw types (WindowId, Rect) — no macOS types leak out.

use pengwm_core::layout::Rect;
use pengwm_core::tree::WindowId;

// ---------------------------------------------------------------------------
// Window attributes
// ---------------------------------------------------------------------------

/// Set the window's position and size on screen.
///
/// # Arguments
/// * `window_id` — the CGWindowID (cast from AXUIElementRef).
/// * `rect`      — global-coordinate rectangle to apply.
pub fn set_window_rect(window_id: WindowId, rect: Rect) -> anyhow::Result<()> {
    //  cast window_id to AXUIElementRef
    //  AXValueCreate(kAXValueTypeCGPoint, &CGPoint { x, y })
    //  AXUIElementSetAttributeValue(element, CFSTR("AXPosition"), value)
    //  AXValueCreate(kAXValueTypeCGSize, &CGSize { w, h })
    //  AXUIElementSetAttributeValue(element, CFSTR("AXSize"), value)
    //  CFRelease intermediate values
    todo!()
}

/// Return the window's current position and size.
pub fn get_window_rect(window_id: WindowId) -> Option<Rect> {
    //  AXUIElementCopyAttributeValue(element, "AXPosition")
    //  AXUIElementCopyAttributeValue(element, "AXSize")
    //  combine into Rect
    todo!()
}

/// Bring the window to the front and give it focus.
pub fn focus_window(window_id: WindowId) {
    //  AXUIElementPerformAction(element, "AXRaise")
    //  AXUIElementSetAttributeValue(element, "AXFocused", kCFBooleanTrue)
    todo!()
}

// ---------------------------------------------------------------------------
// Window filtering
// ---------------------------------------------------------------------------

/// Check whether a window is a standard manageable window (role = AXWindow,
/// subrole = AXStandardWindow). Filters out tooltips, popups, menus.
pub fn is_manageable(window_id: WindowId) -> bool {
    //  get "AXRole"        → must be "AXWindow"
    //  get "AXSubrole"     → must be "AXStandardWindow"
    //  get "AXFocused"     (optional, for filtering floating windows)
    todo!()
}

// ---------------------------------------------------------------------------
// App-level queries
// ---------------------------------------------------------------------------

/// Return the PID of the frontmost (key) application.
pub fn frontmost_pid() -> Option<i32> {
    //  NSWorkspace.sharedWorkspace.frontmostApplication.processIdentifier
    todo!()
}

/// Return all WindowIds for a given PID.
pub fn windows_for_pid(pid: i32) -> Vec<WindowId> {
    //  AXUIElementCreateApplication(pid)
    //  AXUIElementCopyAttributeValue("AXWindows")
    //  collect WindowIds from the CFArray
    todo!()
}
