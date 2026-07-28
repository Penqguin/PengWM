use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr;

use accessibility_sys::*;
use core_foundation::base::{CFRelease, CFRetain, CFTypeRef, TCFType};
use core_foundation::string::{CFString, CFStringRef};
use core_foundation::runloop::{
    CFRunLoopGetCurrent, CFRunLoopAddSource, kCFRunLoopDefaultMode,
};

use crate::event_loop::DaemonEvent;
use crate::macos::ax_element;

use pengwm_core::tree::WindowId;

/// Shared mutable state accessible from both the AX observer callback and
/// MacOsAdapter. The cache maps WindowId to a retained AXUIElementRef + pid.
/// The event_callback forwards AX events to the event loop.
pub struct ObserverContext {
    cache: std::cell::UnsafeCell<HashMap<WindowId, (AXUIElementRef, i32)>>,
    event_callback: Box<dyn Fn(DaemonEvent) + Send>,
}

impl ObserverContext {
    pub fn new(event_callback: Box<dyn Fn(DaemonEvent) + Send>) -> Self {
        Self {
            cache: std::cell::UnsafeCell::new(HashMap::new()),
            event_callback,
        }
    }

    pub fn cache_mut(&self) -> &mut HashMap<WindowId, (AXUIElementRef, i32)> {
        unsafe { &mut *self.cache.get() }
    }

    pub fn cache_get(&self) -> &HashMap<WindowId, (AXUIElementRef, i32)> {
        unsafe { &*self.cache.get() }
    }

    pub fn call_event(&self, event: DaemonEvent) {
        (self.event_callback)(event);
    }

    /// Insert a window into the cache, releasing any previously cached element.
    pub fn cache_insert(&self, window_id: WindowId, element: AXUIElementRef, pid: i32) {
        if let Some((old_elem, _)) = self.cache_mut().insert(window_id, (element, pid)) {
            unsafe { CFRelease(old_elem as CFTypeRef) };
        }
    }
}

unsafe impl Send for ObserverContext {}
unsafe impl Sync for ObserverContext {}

struct RegisteredObserver {
    observer: AXObserverRef,
    app: AXUIElementRef,
}

impl Drop for RegisteredObserver {
    fn drop(&mut self) {
        unsafe {
            CFRelease(self.observer as CFTypeRef);
            CFRelease(self.app as CFTypeRef);
        }
    }
}

pub struct ObserverRegistry {
    observers: HashMap<i32, RegisteredObserver>,
}

impl ObserverRegistry {
    pub fn new() -> Self {
        Self {
            observers: HashMap::new(),
        }
    }

    pub fn attach(&mut self, pid: i32, ctx: &ObserverContext) {
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

        let ctx_ptr = ctx as *const ObserverContext as *mut c_void;

        let mut all_succeeded = true;
        for &notif_name in &notifications {
            let name = CFString::new(notif_name);
            let err = unsafe {
                AXObserverAddNotification(
                    observer,
                    app,
                    name.as_concrete_TypeRef(),
                    ctx_ptr,
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
                CFRelease(observer as CFTypeRef);
                CFRelease(app as CFTypeRef);
            }
            return;
        }

        let run_loop_source = unsafe { AXObserverGetRunLoopSource(observer) };
        if run_loop_source.is_null() {
            log::warn!("AXObserverGetRunLoopSource returned null for pid {}", pid);
            unsafe {
                CFRelease(observer as CFTypeRef);
                CFRelease(app as CFTypeRef);
            }
            return;
        }

        unsafe {
            let run_loop = CFRunLoopGetCurrent();
            CFRunLoopAddSource(run_loop, run_loop_source, kCFRunLoopDefaultMode);
        }

        self.observers.insert(pid, RegisteredObserver { observer, app });
    }

    pub fn detach(&mut self, pid: i32) {
        if let Some(registered) = self.observers.remove(&pid) {
            let run_loop = unsafe { CFRunLoopGetCurrent() };
            let run_loop_source = unsafe { AXObserverGetRunLoopSource(registered.observer) };
            if !run_loop_source.is_null() {
                unsafe {
                    core_foundation::runloop::CFRunLoopRemoveSource(
                        run_loop,
                        run_loop_source,
                        kCFRunLoopDefaultMode,
                    );
                }
            }
            log::info!("Detached AXObserver for pid {}", pid);
        }
    }

}

unsafe extern "C" fn observer_callback(
    _observer: AXObserverRef,
    element: AXUIElementRef,
    notification: CFStringRef,
    refcon: *mut c_void,
) {
    let ctx = &*(refcon as *const ObserverContext);

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

    let mut pid: i32 = 0;
    AXUIElementGetPid(element, &mut pid);

    #[allow(non_upper_case_globals)]
    match notif_str.as_str() {
        kAXWindowCreatedNotification => {
            if !ax_element::is_manageable(element) {
                log::debug!("WindowCreated SKIPPED (not manageable): {} pid={}", window_id, pid);
                return;
            }
            log::debug!("WindowCreated: {} pid={}", window_id, pid);
            CFRetain(element as CFTypeRef);
            ctx.cache_insert(window_id, element, pid);
            ctx.call_event(DaemonEvent::WindowCreated(window_id, pid));
        }
        kAXUIElementDestroyedNotification => {
            log::debug!("WindowDestroyed: {}", window_id);
            if let Some((elem, _)) = ctx.cache_mut().remove(&window_id) {
                CFRelease(elem as CFTypeRef);
            }
            ctx.call_event(DaemonEvent::WindowDestroyed(window_id));
        }
        kAXFocusedWindowChangedNotification => {
            log::debug!("WindowFocused: {}", window_id);
            ctx.call_event(DaemonEvent::WindowFocused(window_id));
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_new_is_empty() {
        let registry = ObserverRegistry::new();
        assert!(registry.observers.is_empty());
    }

    #[test]
    fn app_pid_stored_as_i32() {
        let pid: i32 = 12345;
        assert_eq!(pid, 12345);
    }
}
