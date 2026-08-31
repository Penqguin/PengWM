use std::ffi::c_void;

use crate::adapter::{DisplayInfo, OsAdapter};
use crate::event_loop::DaemonEvent;
use crate::macos::ax_element;
use crate::macos::ax_observer::{ObserverContext, ObserverRegistry};
use crate::macos::cg_display;
use crate::macos::ns_workspace;
use accessibility_sys::*;
use pengwm_core::layout::Rect;
use pengwm_core::tree::WindowId;

const OFFSCREEN: Rect = Rect {
    x: -9999.0,
    y: 0.0,
    width: 1.0,
    height: 1.0,
};

pub struct MacOsAdapter {
    observer_registry: ObserverRegistry,
    ctx: Box<ObserverContext>,
}

impl MacOsAdapter {
    pub fn with_callback(callback: Box<dyn Fn(DaemonEvent) + Send>) -> Self {
        Self {
            observer_registry: ObserverRegistry::new(),
            ctx: Box::new(ObserverContext::new(callback)),
        }
    }

    fn cache_get_element(&self, window_id: WindowId) -> Option<(AXUIElementRef, i32)> {
        self.ctx.cache_get().get(&window_id).copied()
    }

    /// Search for a fresh AXUIElementRef for `window_id` by re-querying the app.
    /// Returns the new element (already +1 retained), releasing all others.
    fn refresh_element(&self, window_id: WindowId, pid: i32) -> Option<AXUIElementRef> {
        use core_foundation::base::{CFRelease, CFTypeRef};
        let windows = unsafe { ax_element::windows_for_pid(pid) };
        let mut result = None;
        for (elem, wid) in windows {
            if wid == window_id {
                result = Some(elem);
            } else {
                unsafe { CFRelease(elem as CFTypeRef) };
            }
        }
        result
    }
}

impl OsAdapter for MacOsAdapter {
    fn running_app_pids(&self) -> Vec<i32> {
        ns_workspace::running_app_pids()
    }

    fn frontmost_pid(&self) -> Option<i32> {
        ax_element::frontmost_pid()
    }

    fn poll_windows_for_pid(&mut self, pid: i32) -> Vec<WindowId> {
        let windows = unsafe { ax_element::windows_for_pid(pid) };
        let mut result = Vec::new();
        for (element, window_id) in windows {
            // element is already CFRetained by windows_for_pid
            self.ctx.cache_insert(window_id, element, pid);
            unsafe {
                self.observer_registry.register_window_destroyed(
                    pid,
                    element,
                    &*self.ctx as *const ObserverContext as *mut c_void,
                );
                self.observer_registry.register_window_moved(
                    pid,
                    element,
                    &*self.ctx as *const ObserverContext as *mut c_void,
                );
            }
            result.push(window_id);
        }
        result
    }

    fn focused_window_for_pid(&self, pid: i32) -> Option<WindowId> {
        unsafe { ax_element::focused_window_for_pid(pid) }
    }

    fn active_displays(&self) -> Vec<DisplayInfo> {
        cg_display::active_displays()
    }

    fn primary_display_id(&self) -> u32 {
        cg_display::primary_display_id()
    }

    fn set_window_rect(&mut self, window_id: WindowId, rect: Rect) -> anyhow::Result<()> {
        let (element, pid) = self.cache_get_element(window_id).ok_or_else(|| {
            anyhow::anyhow!("element not found in cache for window {}", window_id)
        })?;
        match unsafe { ax_element::set_window_rect(element, rect) } {
            Ok(()) => Ok(()),
            Err(e) => {
                let err_str = e.to_string();
                if err_str.contains("kAXErrorInvalidUIElement") {
                    log::warn!("stale element for window {}, re-discovering", window_id);
                    if let Some(new_elem) = self.refresh_element(window_id, pid) {
                        self.ctx.cache_insert(window_id, new_elem, pid);
                        unsafe {
                            self.observer_registry.register_window_destroyed(
                                pid,
                                new_elem,
                                &*self.ctx as *const ObserverContext as *mut c_void,
                            );
                            self.observer_registry.register_window_moved(
                                pid,
                                new_elem,
                                &*self.ctx as *const ObserverContext as *mut c_void,
                            );
                        }
                        return unsafe { ax_element::set_window_rect(new_elem, rect) };
                    }
                    // Window is gone — evict from cache so we don't keep retrying.
                    if let Some((elem, _)) = unsafe { self.ctx.cache_mut() }.remove(&window_id) {
                        unsafe {
                            core_foundation::base::CFRelease(
                                elem as core_foundation::base::CFTypeRef,
                            )
                        };
                    }
                }
                Err(e)
            }
        }
    }

    fn focus_window(&mut self, window_id: WindowId) {
        let (element, pid) = match self.cache_get_element(window_id) {
            Some(v) => v,
            None => {
                log::warn!(
                    "focus_window: element not found in cache for window {}",
                    window_id
                );
                return;
            }
        };
        unsafe { ax_element::focus_window(element, pid) };
    }

    fn close_window(&mut self, window_id: WindowId) {
        if let Some((element, _pid)) = self.cache_get_element(window_id) {
            unsafe { ax_element::close_window(element) };
            // Remove from cache regardless — the element is no longer valid
            if let Some((elem, _)) = unsafe { self.ctx.cache_mut() }.remove(&window_id) {
                unsafe {
                    core_foundation::base::CFRelease(elem as core_foundation::base::CFTypeRef)
                };
            }
        }
    }

    fn hide_windows(&mut self, window_ids: &[WindowId]) {
        for &wid in window_ids {
            if let Err(e) = self.set_window_rect(wid, OFFSCREEN) {
                log::warn!("hide_windows: failed to hide window {}: {}", wid, e);
            }
        }
    }

    fn window_is_hidden(&self, window_id: WindowId) -> bool {
        let (element, pid) = match self.cache_get_element(window_id) {
            Some(v) => v,
            None => return false,
        };
        unsafe {
            // Per-window state: minimized to the Dock or hidden in place.
            if ax_element::bool_attribute(element, kAXMinimizedAttribute)
                || ax_element::bool_attribute(element, kAXHiddenAttribute)
            {
                return true;
            }
            // The whole app hidden (Cmd-H) hides the window too.
            let app = AXUIElementCreateApplication(pid);
            if app.is_null() {
                return false;
            }
            let hidden = ax_element::bool_attribute(app, kAXHiddenAttribute);
            core_foundation::base::CFRelease(app as core_foundation::base::CFTypeRef);
            hidden
        }
    }

    fn attach_observer(&mut self, pid: i32) {
        self.observer_registry.attach(pid, &self.ctx);
    }

    fn detach_observer(&mut self, pid: i32) {
        self.observer_registry.detach(pid);
    }

    fn app_bundle_id(&self, pid: i32) -> Option<String> {
        ns_workspace::bundle_id_for_pid(pid)
    }

    fn app_name(&self, pid: i32) -> Option<String> {
        ns_workspace::localized_name_for_pid(pid)
    }

    fn with_callback(callback: Box<dyn Fn(DaemonEvent) + Send>) -> Self
    where
        Self: Sized,
    {
        MacOsAdapter::with_callback(callback)
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
