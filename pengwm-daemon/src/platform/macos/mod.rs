use async_trait::async_trait;
use crate::platform::WindowManagerBackend;
use crate::core::geometry::Rect;
use crate::core::types::{WindowId, SystemEvent};
use tokio::sync::mpsc::Sender;
use anyhow::Result;
use core_foundation::runloop::{CFRunLoop, kCFRunLoopDefaultMode, CFRunLoopSource};
use core_foundation::string::CFString;
use core_foundation::base::TCFType;
use std::os::raw::c_void;
use std::ptr;
use std::thread;

use accessibility_sys::{
    AXUIElementRef, AXObserverRef, AXIsProcessTrusted,
};

use objc2_app_kit::{NSWorkspace, NSApplicationActivationPolicy};

pub struct MacOsBackend;

#[repr(C)]
struct CGPoint { x: f64, y: f64 }
#[repr(C)]
struct CGSize { width: f64, height: f64 }

struct RawSender(*mut Sender<SystemEvent>);
unsafe impl Send for RawSender {}
unsafe impl Sync for RawSender {}

impl MacOsBackend {
    pub fn is_trusted() -> bool {
        unsafe { AXIsProcessTrusted() }
    }
}

unsafe extern "C" fn observer_callback(
    _observer: AXObserverRef,
    element: AXUIElementRef,
    notification: core_foundation::string::CFStringRef,
    refcon: *mut c_void,
) {
    let sender = &*(refcon as *const Sender<SystemEvent>);
    let notification_str = CFString::wrap_under_get_rule(notification).to_string();
    
    let window_id = WindowId(element as usize);

    let event = match notification_str.as_str() {
        "AXWindowCreated" => Some(SystemEvent::WindowCreated(window_id)),
        "AXUIElementDestroyed" => Some(SystemEvent::WindowDestroyed(window_id)),
        "AXFocusedWindowChanged" => Some(SystemEvent::WindowFocused(window_id)),
        _ => None,
    };

    if let Some(e) = event {
        let _ = sender.try_send(e);
    }
}

#[async_trait]
impl WindowManagerBackend for MacOsBackend {
    async fn subscribe(&self, event_sender: Sender<SystemEvent>) {
        if !Self::is_trusted() {
            log::error!("Accessibility permissions not granted! Please enable them in System Settings.");
            return;
        }

        let sender_ptr = RawSender(Box::into_raw(Box::new(event_sender)));

        thread::spawn(move || {
            let inner_ptr = sender_ptr; // Capture the whole Send-able wrapper
            unsafe {
                let workspace = NSWorkspace::sharedWorkspace();
                let apps = workspace.runningApplications();

                log::info!("Found {} running applications", apps.count());

                for i in 0..apps.count() {
                    let app = apps.objectAtIndex(i);
                    let pid = app.processIdentifier();

                    if app.activationPolicy() == NSApplicationActivationPolicy::Regular {
                        Self::setup_observer(pid as i32, inner_ptr.0);
                    }
                }

                log::info!("macOS Accessibility integration active.");
                CFRunLoop::run_current();
            }
        });
    }

    fn set_window_rect(&self, window: WindowId, rect: Rect) -> Result<()> {
        log::info!("macOS: Moving window {:?} to {:?}", window, rect);
        
        unsafe {
            let window_ref = window.0 as AXUIElementRef;
            
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
            }

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
            }
        }
        
        Ok(())
    }

    fn is_manageable(&self, _window: WindowId) -> bool {
        true
    }

    fn get_focused_window(&self) -> Result<WindowId> {
        Ok(WindowId(0))
    }
}

impl MacOsBackend {
    unsafe fn setup_observer(pid: i32, sender_ptr: *mut Sender<SystemEvent>) {
        let mut observer: AXObserverRef = ptr::null_mut();
        let err = accessibility_sys::AXObserverCreate(pid, observer_callback, &mut observer);

        if err == 0 && !observer.is_null() {
            let app_element = accessibility_sys::AXUIElementCreateApplication(pid);

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

            let source = accessibility_sys::AXObserverGetRunLoopSource(observer);
            let source_ref = CFRunLoopSource::wrap_under_get_rule(source as _);
            CFRunLoop::get_current().add_source(&source_ref, kCFRunLoopDefaultMode);
        }
    }
}
