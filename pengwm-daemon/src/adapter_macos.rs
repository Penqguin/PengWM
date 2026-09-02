use std::ffi::c_void;

use crate::adapter::{DisplayInfo, ObserverRegistry as AdapterObserverRegistry, OsAdapter};
use crate::event_loop::DaemonEvent;
use crate::macos::ax_element;
use crate::macos::ax_observer::{ObserverContext, ObserverRegistry as AxObserverRegistry};
use crate::macos::cg_display;
use crate::macos::ns_workspace;
use accessibility_sys::*;
use pengwm_core::layout::Rect;
use pengwm_core::tree::WindowId;



pub struct MacOsAdapter {
    observer_registry: AxObserverRegistry,
    ctx: Box<ObserverContext>,
}

impl MacOsAdapter {
    pub fn with_callback(callback: Box<dyn Fn(DaemonEvent) + Send>) -> Self {
        Self {
            observer_registry: AxObserverRegistry::new(),
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

    /// Fallback scan: try to discover `window_id` by polling every running app.
    /// Used when the cache has no entry (e.g., window created before daemon start
    /// but poll missed, or cache was evicted on stale element). Returns the
    /// retained element and its pid, and caches it.
    fn discover_window(&self, window_id: WindowId) -> Option<(AXUIElementRef, i32)> {
        use core_foundation::base::{CFRelease, CFTypeRef};
        for pid in ns_workspace::running_app_pids() {
            let windows = unsafe { ax_element::windows_for_pid(pid) };
            let mut found: Option<AXUIElementRef> = None;
            for (elem, wid) in windows {
                if wid == window_id {
                    found = Some(elem);
                } else {
                    unsafe { CFRelease(elem as CFTypeRef) };
                }
            }
            if let Some(elem) = found {
                log::debug!("discover_window hit {} pid {}", window_id, pid);
                self.ctx.cache_insert(window_id, elem, pid);
                return Some((elem, pid));
            }
        }
        None
    }
}

impl OsAdapter for MacOsAdapter {
    fn running_app_pids(&self) -> Vec<i32> {
        ns_workspace::running_app_pids()
    }

    fn frontmost_pid(&self) -> Option<i32> {
        ax_element::frontmost_pid()
    }

    fn poll_windows_for_pid(&self, pid: i32) -> Vec<WindowId> {
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

    fn set_window_rect(&self, window_id: WindowId, rect: Rect) -> anyhow::Result<()> {
        let (element, pid) = match self.cache_get_element(window_id) {
            Some(v) => v,
            None => match self.discover_window(window_id) {
                Some(v) => v,
                None => {
                    log::debug!("set_window_rect cache miss for {} rect {:?}", window_id, rect);
                    anyhow::bail!("element not found in cache for window {}", window_id)
                }
            },
        };
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

    fn focus_window(&self, window_id: WindowId) {
        let (element, pid) = match self.cache_get_element(window_id)
            .or_else(|| self.discover_window(window_id))
        {
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

    fn close_window(&self, window_id: WindowId) {
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

    fn hide_windows(&self, window_ids: &[WindowId]) {
        // Try 0×0 far off-screen first — 1×1 still leaves a 1px sliver
        // because macOS clamps `AXPosition` to keep the title bar on-screen
        // (your “slightly visible” report). 0×0 is fully invisible even when
        // clamped; layout restores the correct tiled size on switch.
        let offscreen_zero = Rect {
            x: -100_000.0,
            y: -100_000.0,
            width: 0.0,
            height: 0.0,
        };
        let offscreen_one = Rect {
            x: -100_000.0,
            y: -100_000.0,
            width: 1.0,
            height: 1.0,
        };
        for &wid in window_ids {
            // Prefer 0×0 for true invisibility; fall back to 1×1 if the app
            // rejects zero size (some apps enforce minimum window size).
            if self.set_window_rect(wid, offscreen_zero).is_ok() {
                continue;
            }
            if let Err(e) = self.set_window_rect(wid, offscreen_one) {
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

    fn app_bundle_id(&self, pid: i32) -> Option<String> {
        ns_workspace::bundle_id_for_pid(pid)
    }

    fn app_name(&self, pid: i32) -> Option<String> {
        ns_workspace::localized_name_for_pid(pid)
    }

    #[cfg(test)]
    fn inject_window(&self, _pid: i32, _window_id: pengwm_core::tree::WindowId) {
        unimplemented!("inject_window only for TestAdapter")
    }

    #[cfg(test)]
    fn inject_app_name(&self, _pid: i32, _name: String) {
        unimplemented!("inject_app_name only for TestAdapter")
    }

    #[cfg(test)]
    fn inject_bundle_id(&self, _pid: i32, _bundle: String) {
        unimplemented!("inject_bundle_id only for TestAdapter")
    }
}

impl AdapterObserverRegistry for MacOsAdapter {
    fn attach_observer(&mut self, pid: i32) {
        self.observer_registry.attach(pid, &self.ctx);
    }

    fn detach_observer(&mut self, pid: i32) {
        self.observer_registry.detach(pid);
    }
}
