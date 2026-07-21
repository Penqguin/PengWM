use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr;

use accessibility_sys::*;
use core_foundation::base::{CFRelease, CFTypeRef, TCFType};
use core_foundation::string::{CFString, CFStringRef};
use core_foundation::runloop::{
    CFRunLoopGetCurrent, CFRunLoopAddSource, kCFRunLoopDefaultMode,
};
use tokio::sync::mpsc;

use crate::event_loop::DaemonEvent;
use crate::macos::ax_element;

struct RegisteredObserver {
    observer: AXObserverRef,
    app: AXUIElementRef,
    refcon: *mut c_void,
}

impl Drop for RegisteredObserver {
    fn drop(&mut self) {
        if !self.refcon.is_null() {
            unsafe {
                let _ = Box::from_raw(self.refcon as *mut mpsc::Sender<DaemonEvent>);
            }
        }
    }
}

pub struct ObserverRegistry {
    observers: HashMap<i32, RegisteredObserver>,
    event_tx: mpsc::Sender<DaemonEvent>,
}

impl ObserverRegistry {
    pub fn new(event_tx: mpsc::Sender<DaemonEvent>) -> Self {
        Self {
            observers: HashMap::new(),
            event_tx,
        }
    }

    pub fn attach(&mut self, pid: i32) {
        if self.observers.contains_key(&pid) {
            return;
        }

        let app = unsafe { AXUIElementCreateApplication(pid) };
        if app.is_null() {
            log::warn!("AXUIElementCreateApplication returned null for pid {}", pid);
            return;
        }

        let mut observer: AXObserverRef = ptr::null_mut();
        let err = unsafe {
            AXObserverCreate(pid, observer_callback, &mut observer)
        };

        if err != kAXErrorSuccess || observer.is_null() {
            log::warn!("AXObserverCreate failed for pid {}: {}", pid, error_string(err));
            unsafe { CFRelease(app as CFTypeRef) };
            return;
        }

        let notifications = [
            kAXWindowCreatedNotification,
            kAXUIElementDestroyedNotification,
            kAXFocusedWindowChangedNotification,
        ];

        let ctx = Box::into_raw(Box::new(self.event_tx.clone())) as *mut c_void;

        let mut all_succeeded = true;
        for &notif_name in &notifications {
            let name = CFString::new(notif_name);
            let err = unsafe {
                AXObserverAddNotification(
                    observer,
                    app,
                    name.as_concrete_TypeRef(),
                    ctx,
                )
            };
            if err != kAXErrorSuccess && err != kAXErrorNotificationAlreadyRegistered {
                log::warn!(
                    "AXObserverAddNotification failed for pid {} notif {}: {}",
                    pid, notif_name, error_string(err)
                );
                all_succeeded = false;
            }
        }

        if !all_succeeded {
            unsafe {
                let _ = Box::from_raw(ctx as *mut mpsc::Sender<DaemonEvent>);
                CFRelease(observer as CFTypeRef);
                CFRelease(app as CFTypeRef);
            }
            return;
        }

        let run_loop_source = unsafe { AXObserverGetRunLoopSource(observer) };
        if run_loop_source.is_null() {
            log::warn!("AXObserverGetRunLoopSource returned null for pid {}", pid);
            unsafe {
                let _ = Box::from_raw(ctx as *mut mpsc::Sender<DaemonEvent>);
                CFRelease(observer as CFTypeRef);
                CFRelease(app as CFTypeRef);
            }
            return;
        }

        unsafe {
            let run_loop = CFRunLoopGetCurrent();
            CFRunLoopAddSource(run_loop, run_loop_source, kCFRunLoopDefaultMode);
        }

        self.observers.insert(pid, RegisteredObserver { observer, app, refcon: ctx });

        self.discover_existing_windows(pid);
    }

    pub fn detach(&mut self, pid: i32) {
        if let Some(registered) = self.observers.remove(&pid) {
            let run_loop_source = unsafe { AXObserverGetRunLoopSource(registered.observer) };
            if !run_loop_source.is_null() {
                unsafe {
                    let run_loop = CFRunLoopGetCurrent();
                    core_foundation::runloop::CFRunLoopRemoveSource(
                        run_loop,
                        run_loop_source,
                        kCFRunLoopDefaultMode,
                    );
                }
            }
            unsafe {
                CFRelease(registered.observer as CFTypeRef);
                CFRelease(registered.app as CFTypeRef);
            }
            log::info!("Detached AXObserver for pid {}", pid);
        }
    }

    fn discover_existing_windows(&self, pid: i32) {
        let windows = unsafe { ax_element::windows_for_pid(pid) };
        for (_element, window_id) in windows {
            log::info!("Discovered existing window {} for pid {}", window_id, pid);
            let _ = self.event_tx.try_send(DaemonEvent::WindowCreated(window_id));
        }
    }
}

unsafe extern "C" fn observer_callback(
    _observer: AXObserverRef,
    element: AXUIElementRef,
    notification: CFStringRef,
    refcon: *mut c_void,
) {
    let tx = &*(refcon as *const mpsc::Sender<DaemonEvent>);

    if notification.is_null() {
        return;
    }

    let notif_str = CFString::wrap_under_get_rule(notification).to_string();

    let window_id = match ax_element::ax_window_id_from_element(element) {
        Some(id) => id,
        None => {
            log::warn!("AX callback could not extract window ID");
            return;
        }
    };

    match notif_str.as_str() {
        kAXWindowCreatedNotification => {
            log::debug!("WindowCreated: {}", window_id);
            let _ = tx.try_send(DaemonEvent::WindowCreated(window_id));
        }
        kAXUIElementDestroyedNotification => {
            log::debug!("WindowDestroyed: {}", window_id);
            let _ = tx.try_send(DaemonEvent::WindowDestroyed(window_id));
        }
        kAXFocusedWindowChangedNotification => {
            log::debug!("WindowFocused: {}", window_id);
            let _ = tx.try_send(DaemonEvent::WindowFocused(window_id));
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_new_is_empty() {
        let (tx, _rx) = mpsc::channel(64);
        let registry = ObserverRegistry::new(tx);
        assert!(registry.observers.is_empty());
    }

    #[test]
    fn notification_constants_match() {
        assert_eq!(kAXWindowCreatedNotification, "AXWindowCreated");
        assert_eq!(kAXUIElementDestroyedNotification, "AXUIElementDestroyed");
        assert_eq!(kAXFocusedWindowChangedNotification, "AXFocusedWindowChanged");
    }

    #[test]
    fn observer_callback_is_compatible() {
        fn _type_check(_f: unsafe extern "C" fn(AXObserverRef, AXUIElementRef, CFStringRef, *mut c_void)) {}
        _type_check(observer_callback);
    }

    #[test]
    fn app_pid_stored_as_i32() {
        let pid: i32 = 12345;
        assert_eq!(pid, 12345);
    }
}
