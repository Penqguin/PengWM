use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr;
use std::time::{Duration, Instant};

use accessibility_sys::*;
use core_foundation::base::{CFEqual, CFTypeRef, TCFType};
use core_foundation::runloop::{kCFRunLoopDefaultMode, CFRunLoopAddSource, CFRunLoopGetCurrent};
use core_foundation::string::{CFString, CFStringRef};

use crate::adapter::WindowClass;
use crate::event_loop::DaemonEvent;
use crate::macos::ax_element;
use crate::macos::cf_ref::{CFPtr, CFRef};
use crate::macos::cg_window_level;

use pengwm_core::tree::WindowId;

/// The element cache's value type, factored out for readability: one
/// `CFRef`-owned element per window, its pid, and its discovery-time
/// `WindowClass`.
pub(crate) mod element_cache {
    use super::*;
    pub(crate) type CachedElement = (CFRef<AXUIElementRef>, i32, WindowClass);
    pub type ElementCache = HashMap<WindowId, CachedElement>;
}

/// Shared mutable state accessible from both the AX observer callback and
/// MacOsAdapter. The cache maps WindowId to an owned retained AXUIElementRef
/// (`CFRef` — drop releases, and overwriting an existing key releases the
/// old ref by construction), its pid, and a typed `WindowClass` —
/// classified at the discovery gate, read back by `window_kind`. The
/// event_callback forwards AX events to the event loop.
pub struct ObserverContext {
    cache: std::cell::UnsafeCell<element_cache::ElementCache>,
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
    pub unsafe fn cache_mut(&self) -> &mut element_cache::ElementCache {
        &mut *self.cache.get()
    }

    pub fn cache_get(&self) -> &element_cache::ElementCache {
        unsafe { &*self.cache.get() }
    }

    pub fn call_event(&self, event: DaemonEvent) {
        (self.event_callback)(event);
    }

    /// Insert a window into the cache with its discovery-time class. A
    /// previously cached element for the same id is released by the map
    /// overwrite — the balance the old hand-rolled insert dance provided.
    pub fn cache_insert(
        &self,
        window_id: WindowId,
        element: CFRef<AXUIElementRef>,
        pid: i32,
        class: WindowClass,
    ) {
        unsafe { self.cache_mut() }.insert(window_id, (element, pid, class));
    }

    /// Find the window ID for a given element pointer by linear scan.
    /// Uses CFEqual for reliable comparison (handles toll-free bridging).
    pub fn find_window_id_by_element(&self, element: AXUIElementRef) -> Option<WindowId> {
        for (&wid, (cached_elem, _, _)) in self.cache_get().iter() {
            let equal: bool =
                unsafe { CFEqual(cached_elem.ptr().as_cf_type(), element as CFTypeRef) != 0 };
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
            .filter_map(|(&wid, &(_, p, _))| if p == pid { Some(wid) } else { None })
            .collect()
    }

    /// Release and drop every cached element belonging to `pid`, returning
    /// the count. The app-termination funnel for cached-but-untracked
    /// members: `on_app_terminated` evicts store-tracked windows per id
    /// (`forget_window`); this releases the rest — an excluded app's
    /// windows created after bootstrap (routing returned before
    /// `register`), so their elements would otherwise outlive the app.
    /// No eviction path of their own exists for those entries.
    pub fn clear_pid(&self, pid: i32) -> usize {
        let mut evicted = 0usize;
        unsafe { self.cache_mut() }.retain(|_, (_, p, _)| {
            if *p == pid {
                // `retain` drops the removed value — the CFRef release.
                evicted += 1;
                false
            } else {
                true
            }
        });
        evicted
    }

    /// Release and drop every cached element whose window id is not in
    /// `keep`, returning the count. The tick sweep's cache-vs-store diff:
    /// entries the store doesn't track have no eviction path (see
    /// `clear_pid`), so the sweep prunes them here.
    pub fn prune_untracked(&self, keep: &std::collections::HashSet<WindowId>) -> usize {
        let mut evicted = 0usize;
        unsafe { self.cache_mut() }.retain(|wid, (_, _, _)| {
            if keep.contains(wid) {
                true
            } else {
                evicted += 1;
                false
            }
        });
        evicted
    }
}

impl Drop for ObserverContext {
    fn drop(&mut self) {
        // Release every cached ref. Correct on shutdown, and required if
        // an adapter is ever built while an old one still lives — without
        // this, the outgoing cache's refs were stranded wholesale.
        unsafe { &mut *self.cache.get() }.clear();
    }
}

unsafe impl Send for ObserverContext {}
unsafe impl Sync for ObserverContext {}

pub struct RegisteredObserver {
    observer: CFRef<AXObserverRef>,
    /// Held (never read) for its lifetime: our +1 keeps the app element
    /// the registrations were made against alive for as long as the
    /// observer lives. Released by the CFRef drop.
    _app: CFRef<AXUIElementRef>,
    /// When `attach` built this observer. The age is the replacement
    /// trigger: AX accumulates per-registration records for the observer's
    /// lifetime and there is no per-notification removal, so a long-lived
    /// observer grows its records with window churn. See `refresh_aged`.
    attached_at: Instant,
    /// Per-window, per-notification registration bookkeeping. AX
    /// accumulates per-registration records for the lifetime of the
    /// observer and there is no per-notification removal in AX, so the
    /// daemon re-registers nothing: a window is registered once per
    /// notification against the observer it lives under. (The key is the
    /// (window id, notification) pair — a window id shared across
    /// notification types once made the moved registration a silent
    /// no-op behind the destroyed one, losing drag detection for
    /// sweep-discovered windows.) Records die with the observer —
    /// `attach` replaces the whole observer on re-attach intent (the wake
    /// resync), which is what gives its first-following poll a clean
    /// slate without any per-tick churn.
    registered: std::cell::RefCell<std::collections::HashSet<(WindowId, &'static str)>>,
    // The `CFRef` fields release themselves — no manual Drop, so the old
    // balance-prone `impl Drop { CFRelease; CFRelease }` is gone.
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
        // Build the replacement first, swap second: a failed build (null app
        // element, AXObserverCreate failure, notification registration
        // failure) keeps the existing observer running instead of dropping
        // observation of a live pid. The replace-on-attach intent (wake
        // resync, aged-observer refresh) must never trade a working observer
        // for nothing — so the old observer is only torn down once the new
        // one is fully built and registered to the run loop.
        let app = unsafe { ax_element::create_app_element(pid) };
        let Some(app) = app else {
            log::warn!("AXUIElementCreateApplication returned null for pid {}", pid);
            return;
        };

        let mut observer: AXObserverRef = ptr::null_mut();
        let err = unsafe { AXObserverCreate(pid, observer_callback, &mut observer) };

        if err != kAXErrorSuccess || observer.is_null() {
            log::warn!(
                "AXObserverCreate failed for pid {}: {}",
                pid,
                error_string(err)
            );
            // `app` drops — the failed build releases what it owns.
            return;
        }
        let observer = unsafe { CFRef::from_create(observer) }.expect("non-null checked above");

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
                AXObserverAddNotification(
                    observer.ptr(),
                    app.ptr(),
                    name.as_concrete_TypeRef(),
                    ctx_ptr,
                )
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
            // `observer` + `app` both drop — nothing released by hand.
            return;
        }

        let run_loop_source = unsafe { AXObserverGetRunLoopSource(observer.ptr()) };
        if run_loop_source.is_null() {
            log::warn!("AXObserverGetRunLoopSource returned null for pid {}", pid);
            return;
        }

        unsafe {
            let run_loop = CFRunLoopGetCurrent();
            CFRunLoopAddSource(run_loop, run_loop_source, kCFRunLoopDefaultMode);
        }

        // Success: swap. The old observer — with every per-registration
        // record AX built under it — dies here, giving the next poll a
        // clean registration slate.
        self.detach_locked(pid);
        self.observers.insert(
            pid,
            RegisteredObserver {
                observer,
                _app: app,
                attached_at: Instant::now(),
                registered: std::cell::RefCell::new(std::collections::HashSet::new()),
            },
        );
    }

    /// Run-loop teardown for one pid's observer. Shared by `detach` and the
    /// replace-on-attach intent path. The removed `RegisteredObserver`
    /// drops at scope end — observer and app element released — after the
    /// runloop source removal below has detached the loop's retain.
    fn detach_locked(&mut self, pid: i32) {
        if let Some(registered) = self.observers.remove(&pid) {
            let run_loop = unsafe { CFRunLoopGetCurrent() };
            let run_loop_source = unsafe { AXObserverGetRunLoopSource(registered.observer.ptr()) };
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
            drop(registered);
        }
    }

    /// Register a kAXWindowMovedNotification on a specific window element,
    /// once per (observer, window id, notification): repeats are skipped —
    /// both because the poll path runs every 2s per tracked window and
    /// because AX accumulates per-registration records with no removal call.
    /// # Safety
    /// `window` must be a valid `AXUIElementRef` and `refcon` a valid pointer.
    pub unsafe fn register_window_moved(
        &self,
        pid: i32,
        window_id: WindowId,
        window: AXUIElementRef,
        refcon: *mut c_void,
    ) {
        self.register_window_impl(
            pid,
            window_id,
            window,
            refcon,
            kAXWindowMovedNotification,
            false,
        );
    }

    /// The refresh path: the old element died, so its registration records
    /// are gone with it and the fresh element must carry its own — even
    /// though the window id is already recorded.
    /// # Safety
    /// `window` must be a valid `AXUIElementRef` and `refcon` a valid pointer.
    pub unsafe fn re_register_window_moved(
        &self,
        pid: i32,
        window_id: WindowId,
        window: AXUIElementRef,
        refcon: *mut c_void,
    ) {
        self.register_window_impl(
            pid,
            window_id,
            window,
            refcon,
            kAXWindowMovedNotification,
            true,
        );
    }

    /// Register a kAXUIElementDestroyedNotification on a specific window
    /// element (once per (observer, window id); see `register_window_moved`).
    /// This fires when the window is closed, allowing the daemon to re-layout.
    /// # Safety
    /// `window` must be a valid `AXUIElementRef` and `refcon` a valid pointer.
    pub unsafe fn register_window_destroyed(
        &self,
        pid: i32,
        window_id: WindowId,
        window: AXUIElementRef,
        refcon: *mut c_void,
    ) {
        self.register_window_impl(
            pid,
            window_id,
            window,
            refcon,
            kAXUIElementDestroyedNotification,
            false,
        );
    }

    /// # Safety
    /// `window` must be a valid `AXUIElementRef` and `refcon` a valid pointer.
    pub unsafe fn re_register_window_destroyed(
        &self,
        pid: i32,
        window_id: WindowId,
        window: AXUIElementRef,
        refcon: *mut c_void,
    ) {
        self.register_window_impl(
            pid,
            window_id,
            window,
            refcon,
            kAXUIElementDestroyedNotification,
            true,
        );
    }

    unsafe fn register_window_impl(
        &self,
        pid: i32,
        window_id: WindowId,
        window: AXUIElementRef,
        refcon: *mut c_void,
        notification: &'static str,
        force: bool,
    ) {
        let Some(registered) = self.observers.get(&pid) else {
            return;
        };
        {
            let mut seen = registered.registered.borrow_mut();
            if !force && seen.contains(&(window_id, notification)) {
                return;
            }
            seen.insert((window_id, notification));
        }
        let name = CFString::new(notification);
        let err = AXObserverAddNotification(
            registered.observer.ptr(),
            window,
            name.as_concrete_TypeRef(),
            refcon,
        );
        if err != kAXErrorSuccess && err != kAXErrorNotificationAlreadyRegistered {
            log::warn!(
                "register {} failed for pid {} window {}: {}",
                notification,
                pid,
                window_id,
                error_string(err)
            );
        }
    }

    pub fn detach(&mut self, pid: i32) {
        self.detach_locked(pid);
    }

    /// Every pid with an attached observer — the dead-pid sweep's suspect
    /// set. An observer whose app quit without delivering the termination
    /// notification is a stranded `AXObserverRef` + app element + retained
    /// runloop source, and its callback can never fire again, so only this
    /// diff can retire it.
    pub fn observed_pids(&self) -> Vec<i32> {
        self.observers.keys().copied().collect()
    }

    /// Replace observers attached longer than `older_than`, re-attaching
    /// each through `attach` (which builds the replacement before tearing
    /// the old one down — a failed rebuild keeps the old observer running).
    /// The replacement resets both the AX-side per-registration records and
    /// the per-observer `registered` set, so long window churn between
    /// sleep events no longer grows the observer unboundedly. The pid's
    /// next poll re-registers each listed window once under the new
    /// observer.
    pub fn refresh_aged(&mut self, ctx: &ObserverContext, older_than: Duration) {
        let aged: Vec<i32> = self
            .observers
            .iter()
            .filter(|(_, registered)| registered.attached_at.elapsed() >= older_than)
            .map(|(&pid, _)| pid)
            .collect();
        for pid in aged {
            log::info!(
                "Refreshing observer for pid {} (attached > {:?}, AX record bound)",
                pid,
                older_than
            );
            self.attach(pid, ctx);
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
                    // The reliable window id is unknowable (stale ref /
                    // CFEqual mismatch), but the pid tells us which app
                    // reported the death. Probe that pid's listing and
                    // untrack whatever tracked window of it is now
                    // unlisted — a dropped warning here used to strand
                    // the dead window in the tree until the 10s write-
                    // path grace (or forever, if no layout ever ran).
                    log::warn!("WindowDestroyed: unknown element, probing pid {}", pid);
                    ctx.call_event(DaemonEvent::UnknownWindowDestroyed(pid));
                    return;
                }
            },
        };
        log::debug!("WindowDestroyed: {}", window_id);
        // The removed CFRef releases itself.
        unsafe { ctx.cache_mut() }.remove(&window_id);
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
            // Classify, don't drop: Sheets and unknowns return at the gate
            // exactly as the old binary check did; everything else is
            // cached + observed + evented, and the class rides the cache.
            let class = WindowClass::apply_level_gate(
                ax_element::classify(element),
                cg_window_level::window_level(window_id),
            );
            if !class.is_manageable() {
                log::debug!(
                    "WindowCreated SKIPPED (class {:?}): {} pid={}",
                    class,
                    window_id,
                    pid
                );
                return;
            }
            log::debug!("WindowCreated: {} pid={} class={:?}", window_id, pid, class);
            // Get-rule borrow from the notification: retain it into
            // ownership for the cache. The fresh element releases the
            // previous cache entry when insert overwrites it.
            if let Some(owned) = CFRef::from_get(element) {
                ax_element::apply_messaging_timeout(owned.ptr());
                ctx.cache_insert(window_id, owned, pid, class);
            }

            // Register per-window notifications directly, NOT through the
            // registry's dedupe set: this C callback holds only the
            // ObserverContext, not the registry, and the element it fired
            // on is the freshest possible evidence of the live element —
            // adding unconditionally here is what keeps a window whose id
            // the poll has already recorded (a late-created, a re-created,
            // or a refreshed element) covered. AX tolerates the duplicate
            // the next poll makes (AlreadyRegistered); the age-bounded
            // observer refresh bounds the accumulated records.
            //
            // Destroyed, so we detect when this window closes (the
            // app-level registration does NOT fire for child windows):
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

            // Moved, so we detect drags for swap-on-hold and snap-back:
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

            // Miniaturize/restore, so a miniaturized window stops occupying
            // tiled space and is retiled on deminiaturize:
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
