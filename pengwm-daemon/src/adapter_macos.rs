use std::collections::HashMap;
use std::ffi::c_void;

use crate::adapter::{
    DisplayInfo, ObserverRegistry as AdapterObserverRegistry, OsAdapter, WindowClass, WindowListing,
};
use crate::event_loop::DaemonEvent;
use crate::macos::ax_element;
use crate::macos::ax_observer::{ObserverContext, ObserverRegistry as AxObserverRegistry};
use crate::macos::cf_ref::CFRef;
use crate::macos::cg_display;
use crate::macos::cg_window_level;
use crate::macos::ns_workspace;
use accessibility_sys::*;
use pengwm_core::layout::{HidePlacement, Rect, WriteOutcome};
use pengwm_core::tree::WindowId;

/// The window's native-fullscreen flag. Not in `accessibility_sys`'s
/// constant set (only the green button's `AXFullScreenButton`
/// attribute/subrole are), so it lives here like `ax_element.rs` handles
/// its own out-of-set constants.
const AX_FULLSCREEN_ATTRIBUTE: &str = "AXFullScreen";

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

    fn cache_get_element(&self, window_id: WindowId) -> Option<(CFRef<AXUIElementRef>, i32)> {
        self.ctx
            .cache_get()
            .get(&window_id)
            .map(|(elem, pid, _)| (elem.clone(), *pid))
    }

    /// Search for a fresh AXUIElementRef for `window_id` by re-querying the app.
    /// Returns the owned fresh element (`CFRef`, +1); every non-matching and
    /// duplicate-matching candidate it drops itself — the duplicate-window-id
    /// overwrite that used to leak one ref is fixed by construction.
    fn refresh_element(&self, window_id: WindowId, pid: i32) -> Option<CFRef<AXUIElementRef>> {
        let windows = unsafe { ax_element::windows_for_pid(pid) };
        // No answer is the same as no match: the caller's stale-ref
        // handling owns what happens next.
        let mut result = None;
        for (elem, wid) in windows.unwrap_or_default() {
            if wid == window_id {
                // Reassignment drops any earlier duplicate match — and the
                // reassignment itself is released by the next one.
                result = Some(elem);
            }
            // Non-matches fall out of scope here: dropped, released.
        }
        result
    }

    /// Resolve `window_id` to a live owned `(element, pid)`: cache hit, else
    /// a full discover scan. One definition for every path that touches an
    /// element (tile, hide, focus) — no caller re-implements cache-then-discover.
    fn resolve_element(&self, window_id: WindowId) -> Option<(CFRef<AXUIElementRef>, i32)> {
        match self.cache_get_element(window_id) {
            Some(v) => Some(v),
            None => self.discover_window(window_id),
        }
    }

    /// Re-resolve after a stale-element failure: re-poll the owning app,
    /// re-cache + re-register observers on hit, evict the dead entry on miss.
    /// Returns the fresh element for one retry.
    fn refresh_after_stale(&self, window_id: WindowId, pid: i32) -> Option<CFRef<AXUIElementRef>> {
        if let Some(new_elem) = self.refresh_element(window_id, pid) {
            // First-classification-wins (Q10): a refreshed element keeps the
            // class the window was discovered with; re-classifying here
            // could silently flip a settled popup's cached kind.
            let prior = self
                .ctx
                .cache_get()
                .get(&window_id)
                .map(|(_, _, c)| *c)
                .unwrap_or_else(|| unsafe { ax_element::classify(new_elem.ptr()) });
            self.ctx
                .cache_insert(window_id, new_elem.clone(), pid, prior);
            unsafe {
                // Force: the old element died, so the fresh one must carry
                // its own registration records.
                self.observer_registry.re_register_window_destroyed(
                    pid,
                    window_id,
                    new_elem.ptr(),
                    &*self.ctx as *const ObserverContext as *mut c_void,
                );
                self.observer_registry.re_register_window_moved(
                    pid,
                    window_id,
                    new_elem.ptr(),
                    &*self.ctx as *const ObserverContext as *mut c_void,
                );
            }
            return Some(new_elem);
        }
        self.evict_element(window_id);
        None
    }

    /// Drop a dead window's cached element (remove + release via the
    /// `CFRef`). One eviction rule for every path — tile, hide, close, and
    /// the trait's `forget_window` delegate.
    fn evict_element(&self, window_id: WindowId) {
        unsafe { self.ctx.cache_mut() }.remove(&window_id);
    }

    /// Position-only write with stale-element refresh. Hide never resizes:
    /// moving offscreen is sufficient and avoids the Firefox reflow storm
    /// (plus the 3x verify loop in `set_window_rect`). Resolution, refresh,
    /// and eviction all run through the shared funnel — only the attempt op
    /// is position-only.
    fn set_position_refreshed(&self, window_id: WindowId, x: f64, y: f64) -> anyhow::Result<()> {
        let (element, pid) = match self.resolve_element(window_id) {
            Some(v) => v,
            None => {
                anyhow::bail!("element not found in cache for window {}", window_id)
            }
        };
        // Hiding animates under enhanced UI just like tiling does; see
        // `set_window_rect`.
        let _enhanced_ui = unsafe { ax_element::EnhancedUiGuard::suspend(pid) };
        match unsafe { ax_element::set_window_position(element.ptr(), x, y) } {
            Ok(()) => Ok(()),
            Err(e) if is_stale_element_message(&e.to_string()) => {
                log::warn!("stale element for window {}, re-discovering", window_id);
                match self.refresh_after_stale(window_id, pid) {
                    Some(new_elem) => unsafe {
                        ax_element::set_window_position(new_elem.ptr(), x, y)
                    },
                    None => Err(e),
                }
            }
            Err(e) => Err(e),
        }
    }

    /// Fallback scan: try to discover `window_id` by polling every running app.
    /// Used when the cache has no entry (e.g., window created before daemon start
    /// but poll missed, or cache was evicted on stale element). Returns the owned
    /// element and its pid, and caches it.
    fn discover_window(&self, window_id: WindowId) -> Option<(CFRef<AXUIElementRef>, i32)> {
        for pid in ns_workspace::running_app_pids() {
            let windows = unsafe { ax_element::windows_for_pid(pid) };
            let mut found: Option<CFRef<AXUIElementRef>> = None;
            for (elem, wid) in windows.unwrap_or_default() {
                if wid == window_id {
                    found = Some(elem);
                }
                // Non-matches drop in scope — released, no manual loop.
            }
            if let Some(elem) = found {
                log::debug!("discover_window hit {} pid {}", window_id, pid);
                // Discovery saw the window but not its class; classify on
                // the owned element before handing it back — through the
                // ADR-0002 level gate, same as every other discovery path.
                let class = WindowClass::apply_level_gate(
                    unsafe { ax_element::classify(elem.ptr()) },
                    cg_window_level::window_level(window_id),
                );
                self.ctx.cache_insert(window_id, elem.clone(), pid, class);
                return Some((elem, pid));
            }
        }
        None
    }

    /// Write `rect` through an already-resolved element, refreshing a
    /// stale element once. Split out of `set_window_rect` so the caller can
    /// wrap the whole attempt — including the refresh retry — in one
    /// `EnhancedUiGuard` and one timing measurement.
    fn write_resolved(
        &self,
        window_id: WindowId,
        element: AXUIElementRef,
        pid: i32,
        rect: Rect,
    ) -> WriteOutcome {
        match unsafe { ax_element::set_window_rect(element, rect) } {
            outcome @ (WriteOutcome::Ok
            | WriteOutcome::Pinned { .. }
            | WriteOutcome::Drift { .. }) => {
                if !outcome.is_ok() {
                    log_write_diagnostics(window_id, pid, element, rect, &outcome);
                }
                outcome
            }
            WriteOutcome::Gone => WriteOutcome::Gone,
            WriteOutcome::Transient(msg) => {
                if !is_stale_element_message(&msg) {
                    return WriteOutcome::Transient(msg);
                }
                log::warn!("stale element for window {}, re-discovering", window_id);
                match self.refresh_after_stale(window_id, pid) {
                    Some(new_elem) => unsafe { ax_element::set_window_rect(new_elem.ptr(), rect) },
                    None => {
                        // Refresh missed too. Before giving up, say what we know:
                        // during a post-wake AX blackout this fires for windows
                        // that are perfectly alive, and the distinction only shows
                        // up in whether the app answers anything at all.
                        if log::log_enabled!(log::Level::Debug) {
                            // A non-zero listing with our window absent is a real
                            // close; zero across the board is the AX blackout, and
                            // the window is probably alive. The owned refs release
                            // as the vec drops — no hand-rolled release loop.
                            let listed = unsafe { ax_element::windows_for_pid(pid) };
                            let ids: Vec<WindowId> = listed
                                .unwrap_or_default()
                                .iter()
                                .map(|(_, wid)| *wid)
                                .collect();
                            log::debug!(
                                "write_diag win={} outcome=Gone target={:?}\n  refresh_element found no match\n  app: {:?} pid {}\n  app still lists {} window(s): {:?}",
                                window_id,
                                rect,
                                ns_workspace::localized_name_for_pid(pid),
                                pid,
                                ids.len(),
                                ids,
                            );
                        }
                        // Window is gone — evict from cache so we don't keep retrying.
                        self.evict_element(window_id);
                        WriteOutcome::Gone
                    }
                }
            }
        }
    }
}

/// True when a transient AX message means the cached element is stale
/// (window re-created under the same id, pid recycled). This is
/// cache-coherence classification local to the adapter — not write-policy:
/// AX failures surface as C ints whose only channel is the message string,
/// so one helper names the check instead of scattering it.
fn is_stale_element_message(msg: &str) -> bool {
    msg.contains("kAXErrorInvalidUIElement")
}

/// One line describing the element a failed write actually landed on.
///
/// A write that never converges has three plausible causes and the rect
/// numbers alone cannot tell them apart: the cached element belongs to a
/// different window than its key (aliasing), the window refuses geometry
/// writes (fullscreen / zoomed / fixed-size / off on an inactive Space),
/// or the app is simply too busy to service the write. This answers all
/// three at once — `elem_wid` vs `window_id` is the aliasing check,
/// `settable` is the refusal check, and the app name says who to blame.
///
/// Only called on a non-`Ok` outcome, which the pin backoff already rate
/// limits to roughly one per window per `PIN_BACKOFF`.
fn log_write_diagnostics(
    window_id: WindowId,
    pid: i32,
    element: AXUIElementRef,
    target: Rect,
    outcome: &WriteOutcome,
) {
    if !log::log_enabled!(log::Level::Debug) {
        return;
    }
    unsafe {
        let elem_wid = ax_element::ax_window_id_from_element(element);
        log::debug!(
            "write_diag win={} outcome={:?} target={:?}\n  element reports window id {:?} (aliased: {})\n  app: {:?} pid {}\n  settable: position={} size={}\n  state: minimized={} hidden={} subrole={:?}",
            window_id,
            outcome,
            target,
            elem_wid,
            elem_wid.is_some_and(|w| w != window_id),
            ns_workspace::localized_name_for_pid(pid),
            pid,
            ax_element::is_attribute_settable(element, kAXPositionAttribute),
            ax_element::is_attribute_settable(element, kAXSizeAttribute),
            ax_element::bool_attribute(element, kAXMinimizedAttribute),
            ax_element::bool_attribute(element, kAXHiddenAttribute),
            ax_element::string_attribute(element, kAXSubroleAttribute),
        );
    }
}

impl MacOsAdapter {
    /// The per-pid poll body, batched under the sweep's one CG snapshot.
    /// Classification is owned here, at the element cache: an id already
    /// cached keeps its class (first-classification-wins — the sweep
    /// never re-reads role/subrole for it); a fresh id classifies now,
    /// through the ADR-0002 level gate fed from the sweep's snapshot.
    /// Unmanageable classes (sheets, unknowns) are filtered exactly where
    /// the FFI used to filter them — never cached, never listed.
    fn poll_one_pid(&self, pid: i32, levels: &HashMap<WindowId, i32>) -> WindowListing {
        let Some(windows) = (unsafe { ax_element::windows_for_pid(pid) }) else {
            // AX errored or the app element would not answer: the blackout
            // signature, not death evidence.
            return WindowListing::NoAnswer;
        };
        let mut result = Vec::new();
        for (element, window_id) in windows {
            let class = match self.cached_class(window_id) {
                Some(cached) => cached,
                None => {
                    let classified = WindowClass::apply_level_gate(
                        unsafe { ax_element::classify(element.ptr()) },
                        levels.get(&window_id).copied(),
                    );
                    if !classified.is_manageable() {
                        continue;
                    }
                    classified
                }
            };
            unsafe {
                // Once per (observer, window id, notification): repeated
                // registrations on every sweep allocate AX-side records with
                // no removal call — the churn that grows the daemon at
                // idle. The age-bounded observer refresh resets the
                // bookkeeping. (The WindowCreated callback also registers
                // destroyed/moved directly and does not record here — it
                // holds only the ObserverContext, not this registry — so
                // the first poll after a callback registration adds each
                // notification once more; AX tolerates the duplicate with
                // AlreadyRegistered and the age refresh bounds the records.)
                self.observer_registry.register_window_destroyed(
                    pid,
                    window_id,
                    element.ptr(),
                    &*self.ctx as *const ObserverContext as *mut c_void,
                );
                self.observer_registry.register_window_moved(
                    pid,
                    window_id,
                    element.ptr(),
                    &*self.ctx as *const ObserverContext as *mut c_void,
                );
            }
            // Ownership moves into the cache; overwrite releases the old ref.
            self.ctx.cache_insert(window_id, element, pid, class);
            result.push(window_id);
        }
        WindowListing::Answered(result)
    }

    /// The class the cache already holds for `window_id`, if any — the
    /// first classification that wins and is never re-derived.
    fn cached_class(&self, window_id: WindowId) -> Option<WindowClass> {
        self.ctx
            .cache_get()
            .get(&window_id)
            .map(|(_, _, class)| *class)
    }
}

impl OsAdapter for MacOsAdapter {
    fn running_app_pids(&self) -> Vec<i32> {
        ns_workspace::running_app_pids()
    }

    fn running_pids_any_policy(&self) -> Vec<i32> {
        ns_workspace::running_pids_any_policy()
    }

    fn frontmost_pid(&self) -> Option<i32> {
        ax_element::frontmost_pid()
    }

    fn poll_windows_for_pids(&self, pids: &[i32]) -> Vec<(i32, WindowListing)> {
        // One CG snapshot covers the whole sweep: level per on-screen
        // window, consumed only by windows being classified for the first
        // time. Cached ids never touch it.
        let levels = cg_window_level::window_levels();
        pids.iter()
            .map(|&pid| (pid, self.poll_one_pid(pid, &levels)))
            .collect()
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

    fn set_window_rect(&self, window_id: WindowId, rect: Rect) -> WriteOutcome {
        let (element, pid) = match self.resolve_element(window_id) {
            Some(v) => v,
            None => {
                log::debug!(
                    "set_window_rect cache miss for {} rect {:?}",
                    window_id,
                    rect
                );
                return WriteOutcome::Gone;
            }
        };
        // Suspend `AXEnhancedUserInterface` for the duration of the write.
        // With it on, Chromium- and Gecko-based apps animate every AX frame
        // change, so a workspace switch visibly crawls the window into place
        // — and the writer's readback lands mid-animation, reads as drift,
        // and re-writes, restarting the animation. yabai, Rectangle and
        // Amethyst all toggle it off around frame writes for this reason.
        // Restored on drop, since assistive tech (VoiceOver) relies on it.
        let started = std::time::Instant::now();
        let enhanced_ui = unsafe { ax_element::EnhancedUiGuard::suspend(pid) };
        let outcome = self.write_resolved(window_id, element.ptr(), pid, rect);
        let enhanced_was_on = enhanced_ui.was_enabled();
        drop(enhanced_ui);
        log::debug!(
            "set_window_rect win={} pid={} took {:?} outcome={} enhanced_ui={}",
            window_id,
            pid,
            started.elapsed(),
            if outcome.is_ok() { "Ok" } else { "failed" },
            if enhanced_was_on {
                "ON (suspended for write)"
            } else {
                "off"
            },
        );
        outcome
    }

    fn window_rect(&self, window_id: WindowId) -> Option<Rect> {
        let (element, _) = self.cache_get_element(window_id)?;
        unsafe { ax_element::get_window_rect(element.ptr()) }
    }

    fn window_kind(&self, window_id: WindowId) -> Option<WindowClass> {
        self.ctx.cache_get().get(&window_id).map(|(_, _, c)| *c)
    }

    fn window_title(&self, window_id: WindowId) -> Option<String> {
        let (element, _) = self.cache_get_element(window_id)?;
        unsafe { ax_element::string_attribute(element.ptr(), kAXTitleAttribute) }
    }

    fn raise_window(&self, window_id: WindowId) {
        if let Some((element, _pid)) = self.cache_get_element(window_id) {
            unsafe { ax_element::raise_window(element.ptr()) };
        }
    }

    fn focus_window(&self, window_id: WindowId) {
        let (element, pid) = match self.resolve_element(window_id) {
            Some(v) => v,
            None => {
                log::warn!(
                    "focus_window: element not found in cache for window {}",
                    window_id
                );
                return;
            }
        };
        unsafe { ax_element::focus_window(element.ptr(), pid) };
    }

    fn close_window(&self, window_id: WindowId) {
        if let Some((element, _pid)) = self.cache_get_element(window_id) {
            unsafe { ax_element::close_window(element.ptr()) };
            // Remove from cache regardless — the element is no longer valid
            self.evict_element(window_id);
        }
    }

    fn hide_windows(&self, placements: &std::collections::HashMap<WindowId, HidePlacement>) {
        // Position-only, always: hiding moves the window offscreen, size is
        // irrelevant. Any size write triggers Firefox reflow (and the 3x
        // verify loop in `set_window_rect`), so hide never resizes.
        for (&wid, placement) in placements {
            let rect = placement.rect();
            if let Err(e) = self.set_position_refreshed(wid, rect.x, rect.y) {
                log::warn!(
                    "hide_windows: failed to hide window {} at {:?}: {}",
                    wid,
                    placement,
                    e
                );
            }
        }
    }

    fn windows_are_hidden(&self, window_ids: &[WindowId]) -> HashMap<WindowId, bool> {
        let mut result: HashMap<WindowId, bool> = HashMap::with_capacity(window_ids.len());
        // Windows whose own attributes say visible, keyed by pid, awaiting
        // the app-hidden verdict.
        let mut app_check: HashMap<i32, Vec<WindowId>> = HashMap::new();
        for &window_id in window_ids {
            let Some((element, pid)) = self.cache_get_element(window_id) else {
                // Same answer the per-window probe gave: no element, no
                // hidden verdict.
                result.insert(window_id, false);
                continue;
            };
            unsafe {
                // Per-window state: minimized to the Dock or hidden in place.
                if ax_element::bool_attribute(element.ptr(), kAXMinimizedAttribute)
                    || ax_element::bool_attribute(element.ptr(), kAXHiddenAttribute)
                {
                    result.insert(window_id, true);
                    continue;
                }
            }
            app_check.entry(pid).or_default().push(window_id);
        }
        // The whole app hidden (Cmd-H) hides every window too — one app
        // element and one `kAXHidden` read per pid covers all of them.
        // An app element that would not build reads as not hidden.
        for (pid, ids) in app_check {
            let app_hidden = unsafe {
                ax_element::create_app_element(pid)
                    .is_some_and(|app| ax_element::bool_attribute(app.ptr(), kAXHiddenAttribute))
            };
            for window_id in ids {
                result.insert(window_id, app_hidden);
            }
        }
        result
    }

    fn window_is_fullscreen(&self, window_id: WindowId) -> bool {
        let Some((element, _pid)) = self.cache_get_element(window_id) else {
            return false;
        };
        unsafe { ax_element::bool_attribute(element.ptr(), AX_FULLSCREEN_ATTRIBUTE) }
    }

    /// Release every OS-side resource cached for `window_id`. The trait
    /// doc describes the prod contract; the fake records the call so state
    /// tests can assert every untrack path evicts exactly once.
    fn forget_window(&self, window_id: WindowId) {
        self.evict_element(window_id);
    }

    /// Termination funnel for cached-but-untracked members of `pid`. The
    /// per-window untrack loop in `on_app_terminated` covers store-tracked
    /// windows; this releases what the observer cached beyond that set.
    fn forget_pid(&self, pid: i32) -> usize {
        let evicted = self.ctx.clear_pid(pid);
        if evicted > 0 {
            log::debug!(
                "forget_pid: released {} cached element(s) for pid {}",
                evicted,
                pid
            );
        }
        evicted
    }

    /// Tick-sweep cache-vs-store diff: cached elements the store no longer
    /// tracks (excluded apps' post-bootstrap windows, missed terminations,
    /// windows that closed between resync and untrack) are evicted here —
    /// the funnel that gives every cached ref a release path.
    fn prune_untracked_elements(&self, keep: &std::collections::HashSet<WindowId>) -> usize {
        let evicted = self.ctx.prune_untracked(keep);
        if evicted > 0 {
            log::debug!(
                "prune_untracked_elements: released {} element(s) not tracked by the store",
                evicted
            );
        }
        evicted
    }

    fn app_bundle_id(&self, pid: i32) -> Option<String> {
        ns_workspace::bundle_id_for_pid(pid)
    }

    fn app_name(&self, pid: i32) -> Option<String> {
        ns_workspace::localized_name_for_pid(pid)
    }
}

impl AdapterObserverRegistry for MacOsAdapter {
    fn attach_observer(&mut self, pid: i32) {
        self.observer_registry.attach(pid, &self.ctx);
    }

    fn detach_observer(&mut self, pid: i32) {
        self.observer_registry.detach(pid);
    }

    fn observed_pids(&self) -> Vec<i32> {
        self.observer_registry.observed_pids()
    }

    fn refresh_aged_observers(&mut self, older_than: std::time::Duration) {
        self.observer_registry.refresh_aged(&self.ctx, older_than);
    }
}
