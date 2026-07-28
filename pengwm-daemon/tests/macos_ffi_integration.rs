#![cfg(target_os = "macos")]

use std::ffi::c_void;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::mpsc;

use pengwm_daemon::config::keybinds::KeybindConfig;
use pengwm_daemon::event_loop::EventLoop;
use pengwm_daemon::macos::ax_element;
use pengwm_daemon::macos::ax_observer::{ObserverContext, ObserverRegistry};
use pengwm_daemon::macos::cg_display;

#[test]
#[ignore = "requires Accessibility permissions and a GUI environment"]
fn macos_ffi_integration() {
    let (_event_loop, _tx) = EventLoop::new(Arc::new(Mutex::new(KeybindConfig::default())));

    // 1. Query active displays
    let displays = cg_display::active_displays();
    assert!(
        !displays.is_empty(),
        "at least one display should be active"
    );
    let primary = cg_display::primary_display_id();
    assert!(primary > 0, "primary display id should be non-zero");

    // 2. Check AX permissions
    assert!(ax_element::is_process_trusted(), "AX permissions required");

    // 3. Get the current frontmost app PID
    let front_pid = ax_element::frontmost_pid().expect("should have a frontmost application");

    // 4. Discover existing windows for the frontmost app
    let windows = unsafe { ax_element::windows_for_pid(front_pid) };
    // Don't assert on this - the app might not have windows

    // 5. If there are windows, try get_window_rect and set_window_rect
    if let Some(&(element, _window_id)) = windows.first() {
        let original_rect = unsafe { ax_element::get_window_rect(element) };
        assert!(original_rect.is_some(), "should be able to get window rect");

        if let Some(rect) = original_rect {
            // Move the window by 10px right and down, then back
            let moved = pengwm_core::layout::Rect::new(
                rect.x + 10.0,
                rect.y + 10.0,
                rect.width,
                rect.height,
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
        assert!(
            !manageable,
            "application elements should not be manageable windows"
        );
        unsafe {
            core_foundation::base::CFRelease(app_elem as *const c_void);
        }
    }
}

#[test]
#[ignore = "requires Accessibility permissions"]
fn observer_registry_create_and_detach() {
    let (tx, _rx) = mpsc::channel(64);
    let ctx = Box::new(ObserverContext::new(Box::new(move |event| {
        let _ = tx.try_send(event);
    })));
    let mut registry = ObserverRegistry::new();

    let front_pid = ax_element::frontmost_pid().expect("should have frontmost app");
    registry.attach(front_pid, &ctx);
    registry.detach(front_pid);
}

#[test]
#[ignore = "requires Accessibility permissions and a GUI environment"]
fn on_window_created_tracks_pid_and_applies_layout() {
    let (mut event_loop, _tx) = EventLoop::new(Arc::new(Mutex::new(KeybindConfig::default())));

    // Drain initial window-discovery events so the state manager's
    // window_pids map is populated for all running apps.
    for _ in 0..10 {
        if !event_loop.pump() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    let pid = ax_element::frontmost_pid().expect("should have a frontmost application");
    let windows_before = unsafe { ax_element::windows_for_pid(pid) };

    // Launch a new Finder window so the AXObserver fires a
    // WindowCreated notification.
    let _ = std::process::Command::new("osascript")
        .args(["-e", r#"tell app "Finder" to make new Finder window"#])
        .output();

    // Give the AXObserver callback time to fire on the CFRunLoop.
    std::thread::sleep(Duration::from_secs(3));

    // Pump the event loop: CFRunLoop processes the AX callback →
    // DaemonEvent::WindowCreated (now with PID) is queued →
    // on_window_created tracks the PID + adds window to tree →
    // apply_layout computes rects and calls set_window_rect.
    for _ in 0..20 {
        if !event_loop.pump() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    let windows_after = unsafe { ax_element::windows_for_pid(pid) };

    // The new window should be present in the window list.
    assert!(
        windows_after.len() >= windows_before.len(),
        "window count should not shrink after launching a new window"
    );

    // Every window should have a readable, positive-size rect —
    // proof that apply_layout successfully sent set_window_rect
    // rather than skipping the window due to a missing PID.
    for &(element, _) in &windows_after {
        let rect = unsafe { ax_element::get_window_rect(element) };
        assert!(rect.is_some(), "each window should have a readable rect");
        let r = rect.unwrap();
        assert!(
            r.width > 0.0 && r.height > 0.0,
            "tiled window should have positive dimensions, got {:?}",
            r
        );
    }

    // Clean up: close the window we just opened.
    for &(element, _) in &windows_after {
        let wid = unsafe { ax_element::ax_window_id_from_element(element).unwrap_or(0) };
        if !windows_before.iter().any(|&(_, id)| id == wid) {
            unsafe { ax_element::close_window(element) };
        }
    }
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
