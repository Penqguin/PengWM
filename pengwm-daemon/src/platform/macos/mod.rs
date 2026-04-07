//! macOS implementation of the WindowManagerBackend trait.
//! Uses Accessibility APIs (AXUIElement) to observe and control windows.

use async_trait::async_trait;
use crate::platform::WindowManagerBackend;
use crate::core::geometry::Rect;
use crate::core::types::{WindowId, SystemEvent};
use tokio::sync::mpsc::Sender;
use anyhow::Result;
use core_foundation::runloop::{CFRunLoop, kCFRunLoopDefaultMode, CFRunLoopSource};
use core_foundation::string::CFString;
use core_foundation::base::{TCFType, CFRelease};
use core_foundation::dictionary::CFDictionary;
use core_foundation::boolean::CFBoolean;
use std::os::raw::c_void;
use std::ptr;
use std::thread;

use accessibility_sys::{
    AXUIElementRef, AXObserverRef, AXIsProcessTrusted, AXIsProcessTrustedWithOptions,
};

use objc2_app_kit::{NSWorkspace, NSApplicationActivationPolicy};

/// macOS specific window manager backend.
pub struct MacOsBackend;

/// Represents a 2D point for CoreGraphics compatibility.
#[repr(C)]
struct CGPoint { x: f64, y: f64 }

/// Represents a 2D size for CoreGraphics compatibility.
#[repr(C)]
struct CGSize { width: f64, height: f64 }

/// A wrapper for the event sender to allow passing it across thread boundaries.
struct RawSender(*mut Sender<SystemEvent>);
unsafe impl Send for RawSender {}
unsafe impl Sync for RawSender {}

impl MacOsBackend {
    /// Checks if the current process has accessibility permissions.
    /// If `prompt` is true, triggers a system dialog if permissions are missing.
    pub fn is_trusted(prompt: bool) -> bool {
        unsafe {
            if prompt {
                let key = CFString::new("AXTrustedCheckOptionPrompt");
                let value = CFBoolean::true_value();
                let options = CFDictionary::from_CFType_pairs(&[(key.as_CFType(), value.as_CFType())]);
                AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef())
            } else {
                AXIsProcessTrusted()
            }
        }
    }
}

/// Callback function invoked by the macOS Accessibility observer when a window event occurs.
unsafe extern "C" fn observer_callback(
    _observer: AXObserverRef,
    element: AXUIElementRef,
    notification: core_foundation::string::CFStringRef,
    refcon: *mut c_void,
) {
    if element.is_null() { return; }

    let sender = &*(refcon as *const Sender<SystemEvent>);
    let notification_str = CFString::wrap_under_get_rule(notification).to_string();
    
    // We use the pointer address as the unique WindowId.
    let window_id = WindowId(element as usize);

    let event = match notification_str.as_str() {
        "AXWindowCreated" => {
            // Retain the window element to ensure it stays alive while we manage it.
            core_foundation::base::CFRetain(element as _);
            Some(SystemEvent::WindowCreated(window_id))
        }
        "AXUIElementDestroyed" => Some(SystemEvent::WindowDestroyed(window_id)),
        "AXFocusedWindowChanged" => Some(SystemEvent::WindowFocused(window_id)),
        _ => None,
    };

    if let Some(e) = event {
        // Use try_send to avoid blocking the OS callback thread.
        let _ = sender.try_send(e);
    }
}

#[async_trait]
impl WindowManagerBackend for MacOsBackend {
    /// Subscribes to window system events (creation, destruction, focus changes).
    async fn subscribe(&self, event_sender: Sender<SystemEvent>) {
        // Ensure we have permissions before starting.
        if !Self::is_trusted(true) {
            log::error!("Accessibility permissions not granted! A prompt should have appeared.");
            log::info!("Please enable accessibility permissions for this app in System Settings > Privacy & Security > Accessibility.");
            return;
        }

        // NSWorkspace::sharedWorkspace() and runningApplications() should be called from the main thread.
        // We capture the PIDs of running applications here before spawning the background loop.
        let pids: Vec<i32> = {
            let workspace = NSWorkspace::sharedWorkspace();
            let apps = workspace.runningApplications();
            let mut pids = Vec::with_capacity(apps.count());
            for i in 0..apps.count() {
                let app = apps.objectAtIndex(i);
                if app.activationPolicy() == NSApplicationActivationPolicy::Regular {
                    pids.push(app.processIdentifier() as i32);
                }
            }
            pids
        };

        let sender_ptr = RawSender(Box::into_raw(Box::new(event_sender)));

        // Run the macOS event loop in a dedicated background thread.
        thread::spawn(move || {
            let inner_ptr = sender_ptr; 
            unsafe {
                // 1. Setup observers for the captured PIDs.
                for pid in pids {
                    Self::setup_observer(pid, inner_ptr.0);
                    
                    // Discover windows that were already open before the observer was attached.
                    Self::discover_existing_windows(pid, inner_ptr.0);
                }
                
                log::info!("macOS Accessibility integration active.");
                // Start the CoreFoundation RunLoop to keep the observers alive.
                CFRunLoop::run_current();
            }
        });
    }

    /// Moves and resizes a window to the specified rectangle using Accessibility APIs.
    fn set_window_rect(&self, window: WindowId, rect: Rect) -> Result<()> {
        if window.0 == 0 { return Ok(()); }
        log::info!("macOS: Moving window {:?} to {:?}", window, rect);
        
        unsafe {
            let window_ref = window.0 as AXUIElementRef;
            
            // Defensive check: is the window still valid?
            let mut pid: i32 = 0;
            if accessibility_sys::AXUIElementGetPid(window_ref, &mut pid) != 0 {
                return Ok(());
            }

            // Set window position (AXPosition attribute).
            let pos = CGPoint { x: rect.min_x() as f64, y: rect.min_y() as f64 };
            let pos_value = accessibility_sys::AXValueCreate(
                accessibility_sys::kAXValueTypeCGPoint,
                &pos as *const _ as *const _,
            );
            if !pos_value.is_null() {
                let attr = CFString::new("AXPosition");
                accessibility_sys::AXUIElementSetAttributeValue(
                    window_ref,
                    attr.as_concrete_TypeRef(),
                    pos_value as _,
                );
                CFRelease(pos_value as _);
            }

            // Set window size (AXSize attribute).
            let size = CGSize { width: rect.width() as f64, height: rect.height() as f64 };
            let size_value = accessibility_sys::AXValueCreate(
                accessibility_sys::kAXValueTypeCGSize,
                &size as *const _ as *const _,
            );
            if !size_value.is_null() {
                let attr = CFString::new("AXSize");
                accessibility_sys::AXUIElementSetAttributeValue(
                    window_ref,
                    attr.as_concrete_TypeRef(),
                    size_value as _,
                );
                CFRelease(size_value as _);
            }
        }
        
        Ok(())
    }

    /// Determines if a window should be managed by the window manager.
    /// Filters out tooltips, popups, and other non-standard windows.
    fn is_manageable(&self, window: WindowId) -> bool {
        if window.0 == 0 { return false; }
        unsafe {
            let window_ref = window.0 as AXUIElementRef;
            
            // Defensive check: is the window still valid?
            let mut pid: i32 = 0;
            if accessibility_sys::AXUIElementGetPid(window_ref, &mut pid) != 0 {
                return false;
            }

            // Check Role (should be AXWindow).
            let mut role: *const c_void = ptr::null();
            if accessibility_sys::AXUIElementCopyAttributeValue(
                window_ref,
                CFString::new("AXRole").as_concrete_TypeRef(),
                &mut role,
            ) == 0 {
                if !role.is_null() {
                    let role_str = CFString::wrap_under_create_rule(role as _).to_string();
                    if role_str != "AXWindow" {
                        log::debug!("Filtering window {:?} with role {}", window, role_str);
                        return false;
                    }
                }
            }

            // Check Subrole (should be AXStandardWindow).
            // This filters out things like "AXSystemDialog" or "AXUnknown".
            let mut subrole: *const c_void = ptr::null();
            if accessibility_sys::AXUIElementCopyAttributeValue(
                window_ref,
                CFString::new("AXSubrole").as_concrete_TypeRef(),
                &mut subrole,
            ) == 0 {
                if !subrole.is_null() {
                    let subrole_str = CFString::wrap_under_create_rule(subrole as _).to_string();
                    if subrole_str != "AXStandardWindow" {
                        log::debug!("Filtering window {:?} with subrole {}", window, subrole_str);
                        return false;
                    }
                }
            }

            true
        }
    }

    /// Not currently implemented on macOS. Returns a stub.
    fn get_focused_window(&self) -> Result<WindowId> {
        Ok(WindowId(0))
    }

    /// Releases our retained reference to the window element.
    fn release_window(&self, window: WindowId) {
        if window.0 != 0 {
            unsafe {
                core_foundation::base::CFRelease(window.0 as _);
            }
        }
    }
}

impl MacOsBackend {
    /// Attaches an accessibility observer to a specific process PID.
    unsafe fn setup_observer(pid: i32, sender_ptr: *mut Sender<SystemEvent>) {
        let mut observer: AXObserverRef = ptr::null_mut();
        let err = accessibility_sys::AXObserverCreate(pid, observer_callback, &mut observer);

        if err == 0 && !observer.is_null() {
            let app_element = accessibility_sys::AXUIElementCreateApplication(pid);
            if app_element.is_null() {
                CFRelease(observer as _);
                return;
            }

            // We listen for creation, destruction, and focus changes.
            let notifications = [
                "AXWindowCreated",
                "AXUIElementDestroyed",
                "AXFocusedWindowChanged",
            ];

            for note in notifications {
                let note_cf = CFString::new(note);
                accessibility_sys::AXObserverAddNotification(
                    observer,
                    app_element,
                    note_cf.as_concrete_TypeRef(),
                    sender_ptr as *mut c_void,
                );
            }

            // Integrate the observer's runloop source into the background thread's runloop.
            let source = accessibility_sys::AXObserverGetRunLoopSource(observer);
            if !source.is_null() {
                let source_ref = CFRunLoopSource::wrap_under_get_rule(source as _);
                CFRunLoop::get_current().add_source(&source_ref, kCFRunLoopDefaultMode);
            }
            
            // Note: In a complete implementation, we'd need to store the observer
            // and app_element to release them properly when the app exits.
            // For now, they live until the process terminates.
            CFRelease(app_element as _);
        } else {
            log::error!("Failed to create AXObserver for PID {}: {}", pid, err);
        }
    }

    /// Iterates through all existing windows for a process and notifies the WindowManager.
    unsafe fn discover_existing_windows(pid: i32, sender_ptr: *mut Sender<SystemEvent>) {
        let app_element = accessibility_sys::AXUIElementCreateApplication(pid);
        if app_element.is_null() { return; }

        let mut windows: *const c_void = ptr::null();
        
        // Query the "AXWindows" attribute for the application.
        if accessibility_sys::AXUIElementCopyAttributeValue(
            app_element,
            CFString::new("AXWindows").as_concrete_TypeRef(),
            &mut windows,
        ) == 0 {
            if !windows.is_null() {
                let windows_cf = core_foundation::array::CFArray::<*const c_void>::wrap_under_create_rule(windows as _);
                let sender = &*(sender_ptr as *const Sender<SystemEvent>);
                
                for win in windows_cf.iter() {
                    if win.is_null() { continue; }
                    
                    // Retain the window reference so it survives being sent to the WindowManager.
                    core_foundation::base::CFRetain(*win);
                    
                    let window_id = WindowId(*win as usize);
                    // Artificially trigger a WindowCreated event for existing windows.
                    let _ = sender.try_send(SystemEvent::WindowCreated(window_id));
                }
            }
        }
        CFRelease(app_element as _);
    }
}
