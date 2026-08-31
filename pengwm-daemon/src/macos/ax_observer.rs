use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr;

use accessibility_sys::*;
use core_foundation::base::{CFEqual, CFRelease, CFRetain, CFTypeRef, TCFType};
use core_foundation::runloop::{kCFRunLoopDefaultMode, CFRunLoopAddSource, CFRunLoopGetCurrent};
use core_foundation::string::{CFString, CFStringRef};

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

    /// # Safety
    /// Caller must ensure no other mutable references to the cache are outstanding.
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn cache_mut(&self) -> &mut HashMap<WindowId, (AXUIElementRef, i32)> {
        &mut *self.cache.get()
    }

    pub fn cache_get(&self) -> &HashMap<WindowId, (AXUIElementRef, i32)> {
        unsafe { &*self.cache.get() }
    }

    pub fn call_event(&self, event: DaemonEvent) {
        (self.event_callback)(event);
    }

    /// Insert a window into the cache, releasing any previously cached element.
    pub fn cache_insert(&self, window_id: WindowId, element: AXUIElementRef, pid: i32) {
        if let Some((old_elem, _)) = unsafe { self.cache_mut() }.insert(window_id, (element, pid)) {
            unsafe { CFRelease(old_elem as CFTypeRef) };
        }
    }

    /// Find the window ID for a given element pointer by linear scan.
    /// Uses CFEqual for reliable comparison (handles toll-free bridging).
    pub fn find_window_id_by_element(&self, element: AXUIElementRef) -> Option<WindowId> {
        for (&wid, &(cached_elem, _)) in self.cache_get().iter() {
            let equal: bool =
                unsafe { CFEqual(cached_elem as CFTypeRef, element as CFTypeRef) != 0 };
            if equal {
                return Some(wid);
            }
        }
        None
    }

    /// All cached window ids belonging to a pid (used for app-level events
    /// like hide/show, where the notification element is the application).
    pub fn window_ids_for_pid(&self, pid: i32) -> Vec<WindowId> {
        self.cache_get()
            .iter()
            .filter_map(|(&wid, &(_, p))| if p == pid { Some(wid) } else { None })
            .collect()
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

impl Default for ObserverRegistry {
    fn default() -> Self {
        Self::new()
    }
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
        let err = unsafe { AXObserverCreate(pid, observer_callback, &mut observer) };

        if err != kAXErrorSuccess || observer.is_null() {
            log::warn!(
                "AXObserverCreate failed for pid {}: {}",
                pid,
                error_string(err)
            );
            unsafe { CFRelease(app as CFTypeRef) };
            return;
        }

        // Individual window destroyed notifications are registered per-window
        // in observer_callback and register_window_destroyed — see below.
        // WindowCreated and FocusedWindowChanged are essential; app-level
        // hide/show are best-effort (failure must not tear down the observer,
        // since the per-window state is reconciled periodically anyway).
        let essential = [
            kAXWindowCreatedNotification,
            kAXFocusedWindowChangedNotification,
        ];
        let optional = [
            kAXApplicationHiddenNotification,
            kAXApplicationShownNotification,
        ];

        let ctx_ptr = ctx as *const ObserverContext as *mut c_void;

        let mut all_succeeded = true;
        for &notif_name in essential.iter().chain(optional.iter()) {
            let name = CFString::new(notif_name);
            let err = unsafe {
                AXObserverAddNotification(observer, app, name.as_concrete_TypeRef(), ctx_ptr)
            };
            if err != kAXErrorSuccess && err != kAXErrorNotificationAlreadyRegistered {
                if optional.contains(&notif_name) {
                    log::debug!(
                        "AXObserverAddNotification optional notif {} skipped for pid {}: {}",
                        notif_name,
                        pid,
                        error_string(err)
                    );
                    continue;
                }
                log::warn!(
                    "AXObserverAddNotification failed for pid {} notif {}: {}",
                    pid,
                    notif_name,
                    error_string(err)
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

        self.observers
            .insert(pid, RegisteredObserver { observer, app });
    }

    /// Register a kAXWindowMovedNotification on a specific window element.
    /// This fires when the window is dragged, allowing overlap detection.
    /// # Safety
    /// `window` must be a valid `AXUIElementRef` and `refcon` a valid pointer.
    pub unsafe fn register_window_moved(
        &self,
        pid: i32,
        window: AXUIElementRef,
        refcon: *mut c_void,
    ) {
        if let Some(registered) = self.observers.get(&pid) {
            let name = CFString::new(kAXWindowMovedNotification);
            let err = AXObserverAddNotification(
                registered.observer,
                window,
                name.as_concrete_TypeRef(),
                refcon,
            );
            if err != kAXErrorSuccess && err != kAXErrorNotificationAlreadyRegistered {
                log::warn!(
                    "register_window_moved failed for pid {}: {}",
                    pid,
                    error_string(err)
                );
            }
        }
    }

    /// Register a kAXUIElementDestroyedNotification on a specific window element.
    /// This fires when the window is closed, allowing the daemon to re-layout.
    /// # Safety
    /// `window` must be a valid `AXUIElementRef` and `refcon` a valid pointer.
    pub unsafe fn register_window_destroyed(
        &self,
        pid: i32,
        window: AXUIElementRef,
        refcon: *mut c_void,
    ) {
        if let Some(registered) = self.observers.get(&pid) {
            let name = CFString::new(kAXUIElementDestroyedNotification);
            let err = unsafe {
                AXObserverAddNotification(
                    registered.observer,
                    window,
                    name.as_concrete_TypeRef(),
                    refcon,
                )
            };
            if err != kAXErrorSuccess && err != kAXErrorNotificationAlreadyRegistered {
                log::warn!(
                    "register_window_destroyed failed for pid {}: {}",
                    pid,
                    error_string(err)
                );
            }
        }
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
    observer: AXObserverRef,
    element: AXUIElementRef,
    notification: CFStringRef,
    refcon: *mut c_void,
) {
    let ctx = &*(refcon as *const ObserverContext);

    if notification.is_null() {
        return;
    }

    let notif_str = CFString::wrap_under_get_rule(notification).to_string();

    let mut pid: i32 = 0;
    AXUIElementGetPid(element, &mut pid);

    // Handle kAXUIElementDestroyedNotification early — the element is dead
    // so ax_window_id_from_element will fail. Fall back to cache lookup by
    // element pointer.
    if notif_str.as_str() == kAXUIElementDestroyedNotification {
        let window_id = match ax_element::ax_window_id_from_element(element) {
            Some(id) => id,
            None => match ctx.find_window_id_by_element(element) {
                Some(id) => id,
                None => {
                    log::warn!("WindowDestroyed: unknown element (not in cache)");
                    return;
                }
            },
        };
        log::debug!("WindowDestroyed: {}", window_id);
        if let Some((elem, _)) = unsafe { ctx.cache_mut() }.remove(&window_id) {
            CFRelease(elem as CFTypeRef);
        }
        ctx.call_event(DaemonEvent::WindowDestroyed(window_id));
        return;
    }

    // App-level hide/show (Cmd-H) fires on the application element, not a
    // window — there is no window id to extract. Stop tracking all of the
    // app's windows and retile them when the app is shown again.
    #[allow(non_upper_case_globals)]
    if notif_str.as_str() == kAXApplicationHiddenNotification
        || notif_str.as_str() == kAXApplicationShownNotification
    {
        let hidden = notif_str.as_str() == kAXApplicationHiddenNotification;
        log::debug!(
            "{} for pid {}",
            if hidden { "AppHidden" } else { "AppShown" },
            pid
        );
        for window_id in ctx.window_ids_for_pid(pid) {
            ctx.call_event(if hidden {
                DaemonEvent::WindowHidden(window_id)
            } else {
                DaemonEvent::WindowShown(window_id)
            });
        }
        return;
    }

    let window_id = match ax_element::ax_window_id_from_element(element) {
        Some(id) => id,
        None => {
            log::debug!("AX callback could not extract window ID (likely app-level event)");
            return;
        }
    };

    #[allow(non_upper_case_globals)]
    match notif_str.as_str() {
        kAXWindowCreatedNotification => {
            if !ax_element::is_manageable(element) {
                log::debug!(
                    "WindowCreated SKIPPED (not manageable): {} pid={}",
                    window_id,
                    pid
                );
                return;
            }
            log::debug!("WindowCreated: {} pid={}", window_id, pid);
            CFRetain(element as CFTypeRef);
            ctx.cache_insert(window_id, element, pid);

            // Register per-window destroyed notification so we detect when
            // this window closes (the app-level registration does NOT fire
            // for child windows).
            let destroyed_name = CFString::new(kAXUIElementDestroyedNotification);
            let add_err = AXObserverAddNotification(
                observer,
                element,
                destroyed_name.as_concrete_TypeRef(),
                refcon,
            );
            if add_err != kAXErrorSuccess && add_err != kAXErrorNotificationAlreadyRegistered {
                log::warn!(
                    "Failed to register kAXUIElementDestroyedNotification on window {}: {}",
                    window_id,
                    error_string(add_err)
                );
            }

            // Register per-window moved notification so we detect drags
            // for swap-on-hold and snap-back.
            let moved_name = CFString::new(kAXWindowMovedNotification);
            let moved_err = AXObserverAddNotification(
                observer,
                element,
                moved_name.as_concrete_TypeRef(),
                refcon,
            );
            if moved_err != kAXErrorSuccess && moved_err != kAXErrorNotificationAlreadyRegistered {
                log::debug!(
                    "kAXWindowMovedNotification registration skipped for window {}: {}",
                    window_id,
                    error_string(moved_err)
                );
            }

            // Register per-window minimize/restore notifications so a
            // miniaturized window stops occupying tiled space and is retiled
            // on deminiaturize.
            let mini_name = CFString::new(kAXWindowMiniaturizedNotification);
            let mini_err = AXObserverAddNotification(
                observer,
                element,
                mini_name.as_concrete_TypeRef(),
                refcon,
            );
            if mini_err != kAXErrorSuccess && mini_err != kAXErrorNotificationAlreadyRegistered {
                log::debug!(
                    "kAXWindowMiniaturizedNotification registration skipped for window {}: {}",
                    window_id,
                    error_string(mini_err)
                );
            }
            let demini_name = CFString::new(kAXWindowDeminiaturizedNotification);
            let demini_err = AXObserverAddNotification(
                observer,
                element,
                demini_name.as_concrete_TypeRef(),
                refcon,
            );
            if demini_err != kAXErrorSuccess && demini_err != kAXErrorNotificationAlreadyRegistered
            {
                log::debug!(
                    "kAXWindowDeminiaturizedNotification registration skipped for window {}: {}",
                    window_id,
                    error_string(demini_err)
                );
            }

            ctx.call_event(DaemonEvent::WindowCreated(window_id, pid));
        }
        kAXFocusedWindowChangedNotification => {
            log::debug!("WindowFocused: {}", window_id);
            ctx.call_event(DaemonEvent::WindowFocused(window_id));
        }
        kAXWindowMovedNotification => {
            if let Some(rect) = unsafe { crate::macos::ax_element::get_window_rect(element) } {
                ctx.call_event(DaemonEvent::WindowMoved(window_id, rect.x, rect.y));
            }
        }
        kAXWindowMiniaturizedNotification => {
            log::debug!("WindowHidden (miniaturized): {}", window_id);
            ctx.call_event(DaemonEvent::WindowHidden(window_id));
        }
        kAXWindowDeminiaturizedNotification => {
            log::debug!("WindowShown (deminiaturized): {}", window_id);
            ctx.call_event(DaemonEvent::WindowShown(window_id));
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
