#![cfg(target_os = "macos")]

use std::ffi::c_void;
use std::ptr;
use std::time::Duration;

use tokio::sync::mpsc;

use pengwm_daemon::event_loop::{DaemonEvent, EventLoop};
use pengwm_daemon::macos::ax_element;
use pengwm_daemon::macos::cg_display;
use pengwm_daemon::macos::ns_workspace;
use pengwm_daemon::macos::ax_observer::ObserverRegistry;

#[test]
#[ignore = "requires Accessibility permissions and a GUI environment"]
fn macos_ffi_integration() {
    let (_event_loop, tx) = EventLoop::new();

    // 1. Query active displays
    let displays = cg_display::active_displays();
    assert!(!displays.is_empty(), "at least one display should be active");
    let primary = cg_display::primary_display_id();
    assert!(primary > 0, "primary display id should be non-zero");

    // 2. Check AX permissions
    assert!(ax_element::is_process_trusted(), "AX permissions required");

    // 3. Get the current frontmost app PID
    let front_pid = ax_element::frontmost_pid()
        .expect("should have a frontmost application");

    // 4. Discover existing windows for the frontmost app
    let windows = unsafe { ax_element::windows_for_pid(front_pid) };
    // Don't assert on this - the app might not have windows

    // 5. If there are windows, try get_window_rect and set_window_rect
    if let Some(&(element, window_id)) = windows.first() {
        let original_rect = unsafe { ax_element::get_window_rect(element) };
        assert!(original_rect.is_some(), "should be able to get window rect");

        if let Some(rect) = original_rect {
            // Move the window by 10px right and down, then back
            let moved = pengwm_core::layout::Rect::new(
                rect.x + 10.0, rect.y + 10.0, rect.width, rect.height,
            );
            let result = unsafe { ax_element::set_window_rect(element, moved) };
            assert!(result.is_ok(), "set_window_rect should succeed");

            let readback = unsafe { ax_element::get_window_rect(element) };
            assert!(readback.is_some(), "should read back window rect");

            // Move back to original position
            let _ = unsafe { ax_element::set_window_rect(element, rect) };
        }

        // Focus the window
        unsafe { ax_element::focus_window(element) };
    }

    // 6. Check is_manageable on application element
    let app_elem = unsafe { accessibility_sys::AXUIElementCreateApplication(front_pid) };
    if !app_elem.is_null() {
        let manageable = unsafe { ax_element::is_manageable(app_elem) };
        assert!(!manageable, "application elements should not be manageable windows");
        unsafe {
            core_foundation::base::CFRelease(app_elem as *const c_void);
        }
    }
}

#[test]
#[ignore = "requires Accessibility permissions"]
fn observer_registry_create_and_detach() {
    let (tx, mut rx) = mpsc::channel(64);
    let mut registry = ObserverRegistry::new(tx);

    let front_pid = ax_element::frontmost_pid()
        .expect("should have frontmost app");
    registry.attach(front_pid);
    registry.detach(front_pid);

    // After detach, no more events should come
    let result = rx.try_recv();
    assert!(result.is_err(), "should not receive events after detach");
}

#[test]
fn display_info_primary_exists() {
    let primary = cg_display::primary_display_id();
    assert!(primary > 0);
}

#[test]
fn active_displays_non_empty() {
    let displays = cg_display::active_displays();
    assert!(!displays.is_empty());
    for d in &displays {
        assert!(d.id > 0, "display id should be non-zero");
    }
}
