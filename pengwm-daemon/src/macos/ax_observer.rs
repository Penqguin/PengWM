//! Per-application AXObserver management.
//!
//! macOS requires one AXObserver per process. This module handles:
//!   - AXObserverCreate for a given PID
//!   - Registering for AXWindowCreated, AXUIElementDestroyed, AXFocusedWindowChanged
//!   - The C callback that translates AX notifications into DaemonEvents
//!   - Discovering existing windows at startup via AXUIElementCopyAttributeValue("AXWindows")
//!   - Cleaning up observers when an app terminates

use std::collections::HashMap;
use tokio::sync::mpsc;
use crate::event_loop::DaemonEvent;

/// Manages one AXObserver per running application.
pub struct ObserverRegistry {
    //  observers: HashMap<i32, AXObserverRef>,
    //  event_tx: mpsc::Sender<DaemonEvent>,
}

impl ObserverRegistry {
    pub fn new(event_tx: mpsc::Sender<DaemonEvent>) -> Self {
        todo!("create empty registry")
    }

    /// Attach an observer to a process. Called at startup and on app launch.
    pub fn attach(&mut self, pid: i32) {
        //  AXObserverCreate(pid, callback, &observer)
        //  for each notification name:
        //    AXObserverAddNotification(observer, app_element, name, refcon)
        //  CFRunLoopSource from AXObserverGetRunLoopSource
        //  add source to current CFRunLoop
        //  store observer in self.observers
        //  call discover_existing_windows(pid) to sync already-open windows
        todo!()
    }

    /// Detach and release an observer. Called on app termination.
    pub fn detach(&mut self, pid: i32) {
        //  CFRunLoopSourceInvalidate
        //  CFRelease(observer)
        //  remove from self.observers
        todo!()
    }

    /// Scan all open windows for the given PID and fire WindowCreated events.
    fn discover_existing_windows(&self, pid: i32) {
        //  AXUIElementCreateApplication(pid)
        //  AXUIElementCopyAttributeValue(app, "AXWindows", &windows)
        //  for each window ref:
        //    CFRetain it
        //    send DaemonEvent::WindowCreated into event_tx
        todo!()
    }
}

/// The C callback invoked on the AXObserver run loop thread.
///
/// # Safety
/// This is called from macOS, not directly.
unsafe extern "C" fn observer_callback(
    _observer: *mut std::ffi::c_void,
    element: *mut std::ffi::c_void,
    notification: *mut std::ffi::c_void,
    refcon: *mut std::ffi::c_void,
) {
    //  convert notification CFString to &str
    //  match notification name:
    //    "AXWindowCreated"       → DaemonEvent::WindowCreated
    //    "AXUIElementDestroyed"  → DaemonEvent::WindowDestroyed
    //    "AXFocusedWindowChanged"→ DaemonEvent::WindowFocused
    //  send into event_tx via refcon
    todo!("AX callback")
}
