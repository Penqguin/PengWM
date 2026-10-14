use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::{Duration, Instant};

use crate::adapter::{DisplayInfo, ObserverRegistry, OsAdapter, WindowClass, WindowListing};
use pengwm_core::layout::{HidePlacement, Rect, WriteOutcome};
use pengwm_core::tree::WindowId;

/// Fault injection vocabulary for tests. Mirrors the four failure modes of
/// `WriteOutcome` without the rect payloads: the fake stamps
/// `target` (the requested rect) and `actual` (the stored OS rect) live at
/// serve time, so tests name the mode, not the geometry.
#[derive(Debug, Clone)]
pub enum Fault {
    Gone,
    Transient(String),
    Drift,
    Pinned,
}

pub struct TestAdapter {
    pub running_apps: RefCell<Vec<i32>>,
    pub frontmost: Option<i32>,
    pub windows: RefCell<HashMap<i32, Vec<WindowId>>>,
    pub window_pids: RefCell<HashMap<WindowId, i32>>,
    pub window_rects: RefCell<HashMap<WindowId, Rect>>,
    pub displays: Vec<DisplayInfo>,
    /// `set_displays` override — topology tests swap the live snapshot
    /// mid-life without touching the construction-time field.
    pub display_override: RefCell<Option<Vec<DisplayInfo>>>,
    pub focused_windows: RefCell<HashMap<i32, WindowId>>,
    pub last_focused: Cell<Option<WindowId>>,
    pub observers: RefCell<HashSet<i32>>,
    pub bundle_ids: RefCell<HashMap<i32, String>>,
    pub app_names: RefCell<HashMap<i32, String>>,
    pub hidden_windows: RefCell<HashSet<WindowId>>,
    pub hidden_apps: RefCell<HashSet<i32>>,
    /// The element-cache stand-in: window id → pid for every element the
    /// "observer callback or poll" cached. Mirrors `ObserverContext`'s
    /// cache so prune/forget funnels are observable: eviction removes from
    /// here (and records it) while listings stay untouched, exactly as
    /// prod releases the ref but the app keeps its window.
    pub cached_elements: RefCell<HashMap<WindowId, i32>>,
    /// Injected native-fullscreen set (missed-transition simulation for the
    /// fullscreen parking probe).
    pub fullscreen_windows: RefCell<HashSet<WindowId>>,
    /// Injected write faults per window. Served as the matching
    /// `WriteOutcome` without updating the OS rect — the fake and the seam
    /// share one failure language.
    pub faults: RefCell<HashMap<WindowId, Fault>>,
    /// Number of `set_window_rect` calls served. Lets tests observe whether
    /// `apply_layout` skipped redundant writes.
    pub set_rect_calls: Cell<usize>,
    /// Simulates the post-wake AX blackout: polls come back empty and
    /// rects are unreadable, exactly as a live app behaves in the seconds
    /// after `NSWorkspaceDidWake`, while the windows are still very much
    /// alive.
    pub ax_blackout: Cell<bool>,
    /// Injected per-window `WindowClass`. Uninjected windows classify as
    /// `None`, which routing reads as a standard tiled window — matching
    /// how the old binary gate treated every evented window.
    pub window_classes: RefCell<HashMap<WindowId, WindowClass>>,
    /// `raise_window` calls in order, so tests can assert the switch-back
    /// "popup comes back on top" invariant.
    pub raised: RefCell<Vec<WindowId>>,
    /// `forget_window` calls in order, so tests can assert every untrack
    /// path evicts the OS-side resources of the window it untracked.
    pub forgotten: RefCell<Vec<WindowId>>,
    /// `forget_pid` calls (pid, element count) in order, so tests can
    /// assert app termination releases cached-but-untracked elements too.
    pub forgotten_pids: RefCell<Vec<(i32, usize)>>,
    /// Window ids evicted by `prune_untracked_elements`, in order.
    pub pruned: RefCell<Vec<WindowId>>,
    /// Observer attach timestamps per pid (refresh-aging simulation).
    pub observer_age: RefCell<HashMap<i32, Instant>>,
    /// Number of `refresh_aged_observers` triggers (pid replacements).
    pub refreshes: Cell<usize>,
}

impl Default for TestAdapter {
    fn default() -> Self {
        Self {
            running_apps: RefCell::new(Vec::new()),
            frontmost: None,
            windows: RefCell::new(HashMap::new()),
            window_pids: RefCell::new(HashMap::new()),
            window_rects: RefCell::new(HashMap::new()),
            displays: Vec::new(),
            display_override: RefCell::new(None),
            focused_windows: RefCell::new(HashMap::new()),
            last_focused: Cell::new(None),
            observers: RefCell::new(HashSet::new()),
            bundle_ids: RefCell::new(HashMap::new()),
            app_names: RefCell::new(HashMap::new()),
            hidden_windows: RefCell::new(HashSet::new()),
            hidden_apps: RefCell::new(HashSet::new()),
            cached_elements: RefCell::new(HashMap::new()),
            fullscreen_windows: RefCell::new(HashSet::new()),
            faults: RefCell::new(HashMap::new()),
            set_rect_calls: Cell::new(0),
            ax_blackout: Cell::new(false),
            window_classes: RefCell::new(HashMap::new()),
            raised: RefCell::new(Vec::new()),
            forgotten: RefCell::new(Vec::new()),
            forgotten_pids: RefCell::new(Vec::new()),
            pruned: RefCell::new(Vec::new()),
            observer_age: RefCell::new(HashMap::new()),
            refreshes: Cell::new(0),
        }
    }
}

impl TestAdapter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Observer attach/detach with shared-friendly `&self` (interior
    /// mutability). Both the `ObserverRegistry` impl and the shared
    /// adapter delegate here.
    pub fn attach(&self, pid: i32) {
        self.observers.borrow_mut().insert(pid);
        // Replace-on-attach resets the age, as prod re-attach does.
        self.observer_age
            .borrow_mut()
            .insert(pid, std::time::Instant::now());
    }

    pub fn detach(&self, pid: i32) {
        self.observers.borrow_mut().remove(&pid);
        self.observer_age.borrow_mut().remove(&pid);
    }

    /// Age every attached observer by `secs` (refresh-aging simulation:
    /// time passes without an attach).
    pub fn age_observers_for_test(&self, secs: u64) {
        let shift = Duration::from_secs(secs);
        let mut ages = self.observer_age.borrow_mut();
        for timestamp in ages.values_mut() {
            *timestamp = timestamp
                .checked_sub(shift)
                .unwrap_or(Instant::now() - shift);
        }
    }

    /// Replace observers attached longer than `older_than`: detach +
    /// attach each, recording the replacement count. The mechanics prod's
    /// `ObserverRegistry::refresh_aged` performs under its AX FFI.
    pub fn refresh_aged_impl(&self, older_than: Duration) {
        let now = Instant::now();
        let aged: Vec<i32> = self
            .observer_age
            .borrow()
            .iter()
            .filter(|(_, at)| now.duration_since(**at) >= older_than)
            .map(|(&pid, _)| pid)
            .collect();
        if aged.is_empty() {
            return;
        }
        self.refreshes.set(self.refreshes.get() + aged.len());
        for pid in aged {
            self.detach(pid);
            self.attach(pid);
        }
    }

    /// Number of aged-observer replacements served.
    pub fn refreshes(&self) -> usize {
        self.refreshes.get()
    }

    /// Pids pruned-for (forget_pid calls), with their eviction counts.
    pub fn forgotten_pids(&self) -> Vec<(i32, usize)> {
        self.forgotten_pids.borrow().clone()
    }

    /// Window ids evicted by the untracked cache prune, in order.
    pub fn pruned(&self) -> Vec<WindowId> {
        self.pruned.borrow().clone()
    }

    /// Simulate an app quitting without its `NSWorkspaceDidTerminate`
    /// notification reaching the daemon: the pid vanishes from the
    /// running list and its windows vanish from every listing and cache —
    /// but `WindowStore` keeps tracking them until the dead-pid sweep
    /// runs. This is exactly the state that stranded observers, leaked
    /// elements, and tree entries before the sweep existed.
    pub fn simulate_missed_termination(&self, pid: i32) {
        self.running_apps.borrow_mut().retain(|&p| p != pid);
        if let Some(windows) = self.windows.borrow_mut().get_mut(&pid) {
            windows.clear();
        }
        // Collect the pid's window ids first: the pid maps drop them below,
        // and the rects are keyed by window id.
        let pids_windows: Vec<WindowId> = self
            .window_pids
            .borrow()
            .iter()
            .filter(|(_, &p)| p == pid)
            .map(|(&wid, _)| wid)
            .collect();
        self.cached_elements
            .borrow_mut()
            .retain(|_, &mut p| p != pid);
        self.window_pids.borrow_mut().retain(|_, &mut p| p != pid);
        let mut rects = self.window_rects.borrow_mut();
        for wid in pids_windows {
            rects.remove(&wid);
        }
    }

    /// Inject a write fault: the next `set_window_rect` calls for
    /// `window_id` serve it without touching the OS rect.
    pub fn set_fault(&self, window_id: WindowId, fault: Fault) {
        self.faults.borrow_mut().insert(window_id, fault);
    }

    /// Clear the fault: the window writes cleanly again (same id) after a
    /// transient blackout. Lets tests exercise grace-then-heal without
    /// retiling as new.
    pub fn clear_fault(&self, window_id: WindowId) {
        self.faults.borrow_mut().remove(&window_id);
    }

    /// Externally displace a window's OS rect (user drag / app move
    /// simulation). Lets tests exercise the misplaced reconcile.
    pub fn displace(&self, window_id: WindowId, dx: f64, dy: f64) {
        let mut rects = self.window_rects.borrow_mut();
        if let Some(r) = rects.get_mut(&window_id) {
            r.x += dx;
            r.y += dy;
        }
    }

    /// Seed a window's OS rect directly (e.g. a popup the app drew at its
    /// own position before the WM saw it).
    pub fn inject_rect(&self, window_id: WindowId, rect: Rect) {
        self.window_rects.borrow_mut().insert(window_id, rect);
    }

    /// Mark a window minimized/hidden in the fake (missed-notification
    /// simulation for the hidden reconcile).
    pub fn inject_hidden_window(&self, window_id: WindowId) {
        self.hidden_windows.borrow_mut().insert(window_id);
    }

    /// Un-mark a hidden window (deminiaturize simulation).
    pub fn clear_hidden_window(&self, window_id: WindowId) {
        self.hidden_windows.borrow_mut().remove(&window_id);
    }

    /// Mark a window native-fullscreen in the fake (the probe's only
    /// transition channel — no dedicated AX notification exists in prod
    /// either, so the fake has no event to inject).
    pub fn inject_fullscreen_window(&self, window_id: WindowId) {
        self.fullscreen_windows.borrow_mut().insert(window_id);
    }

    /// Clear the fullscreen mark: simulates exiting fullscreen.
    pub fn clear_fullscreen_window(&self, window_id: WindowId) {
        self.fullscreen_windows.borrow_mut().remove(&window_id);
    }

    /// Black out AX: polls return no windows and rects are unreadable,
    /// while the windows stay alive in the fake. Mirrors the seconds after
    /// a wake notification.
    pub fn set_ax_blackout(&self, blacked_out: bool) {
        self.ax_blackout.set(blacked_out);
    }

    /// Number of `set_window_rect` calls served. Lets layout tests observe
    /// whether redundant writes were skipped.
    pub fn writes(&self) -> usize {
        self.set_rect_calls.get()
    }

    /// Current OS rect as the fake sees it.
    pub fn rect(&self, window_id: WindowId) -> Option<Rect> {
        self.window_rect(window_id)
    }

    pub fn inject_window(&self, pid: i32, window_id: WindowId) {
        self.windows
            .borrow_mut()
            .entry(pid)
            .or_default()
            .push(window_id);
        self.window_pids.borrow_mut().insert(window_id, pid);
        // The observer callback / poll would have cached this element.
        self.cached_elements.borrow_mut().insert(window_id, pid);
    }

    /// Remove a window from the fake's listing without any destroyed
    /// notification — simulates a close whose notification never arrives
    /// (the missed-notification path the sweep untrack exists for).
    pub fn unlist_window(&self, pid: i32, window_id: WindowId) {
        if let Some(windows) = self.windows.borrow_mut().get_mut(&pid) {
            windows.retain(|&w| w != window_id);
        }
        self.window_pids.borrow_mut().remove(&window_id);
        self.window_rects.borrow_mut().remove(&window_id);
    }

    pub fn inject_app_name(&self, pid: i32, name: String) {
        self.app_names.borrow_mut().insert(pid, name);
    }

    pub fn inject_bundle_id(&self, pid: i32, bundle: String) {
        self.bundle_ids.borrow_mut().insert(pid, bundle);
    }

    /// Inject a window's `WindowClass` (the fake's classification cell).
    pub fn inject_window_kind(&self, window_id: WindowId, class: WindowClass) {
        self.window_classes.borrow_mut().insert(window_id, class);
    }

    /// `raise_window` calls in order, so tests can assert on-top behavior.
    pub fn raised(&self) -> Vec<WindowId> {
        self.raised.borrow().clone()
    }

    /// `forget_window` calls in order, for the eviction-on-untrack tests.
    pub fn forgotten(&self) -> Vec<WindowId> {
        self.forgotten.borrow().clone()
    }
}

/// Shared handle to a `TestAdapter` boxed into a `StateManager`. Tests hold
/// the handle and speak the test vocabulary (`set_fault`, `displace`,
/// `writes`, `rect`, `inject_*`); the boxed `SharedTestAdapter` speaks the
/// prod `OsAdapter` interface. One `Rc` underneath, so faults land in the
/// same cells the layout path reads.
#[derive(Clone)]
pub struct TestHandle(Rc<TestAdapter>);

impl TestHandle {
    pub fn new(adapter: TestAdapter) -> Self {
        Self(Rc::new(adapter))
    }

    /// The prod-facing adapter sharing this handle's state. Box it into
    /// `StateManager::new`.
    pub fn shared(&self) -> SharedTestAdapter {
        SharedTestAdapter(self.0.clone())
    }

    pub fn set_fault(&self, window_id: WindowId, fault: Fault) {
        self.0.set_fault(window_id, fault)
    }

    pub fn clear_fault(&self, window_id: WindowId) {
        self.0.clear_fault(window_id)
    }

    pub fn displace(&self, window_id: WindowId, dx: f64, dy: f64) {
        self.0.displace(window_id, dx, dy)
    }

    pub fn inject_rect(&self, window_id: WindowId, rect: Rect) {
        self.0.inject_rect(window_id, rect)
    }

    pub fn inject_hidden_window(&self, window_id: WindowId) {
        self.0.inject_hidden_window(window_id)
    }

    pub fn clear_hidden_window(&self, window_id: WindowId) {
        self.0.clear_hidden_window(window_id)
    }

    pub fn inject_fullscreen_window(&self, window_id: WindowId) {
        self.0.inject_fullscreen_window(window_id)
    }

    pub fn clear_fullscreen_window(&self, window_id: WindowId) {
        self.0.clear_fullscreen_window(window_id)
    }

    /// Replace the shared adapter's live display snapshot. Topology tests
    /// drive this to simulate "the display list changed without any
    /// `MonitorRemoved`/`MonitorAdded`/`MonitorResized` event reaching us" —
    /// exactly what a dropped hotplug event or a before-pass race produces.
    pub fn set_displays(&self, displays: Vec<DisplayInfo>) {
        *self.0.display_override.borrow_mut() = Some(displays);
    }

    pub fn writes(&self) -> usize {
        self.0.writes()
    }

    pub fn set_ax_blackout(&self, blacked_out: bool) {
        self.0.set_ax_blackout(blacked_out)
    }

    pub fn rect(&self, window_id: WindowId) -> Option<Rect> {
        self.0.rect(window_id)
    }

    pub fn inject_window(&self, pid: i32, window_id: WindowId) {
        self.0.inject_window(pid, window_id)
    }

    pub fn unlist_window(&self, pid: i32, window_id: WindowId) {
        self.0.unlist_window(pid, window_id)
    }

    pub fn inject_app_name(&self, pid: i32, name: String) {
        self.0.inject_app_name(pid, name)
    }

    pub fn inject_bundle_id(&self, pid: i32, bundle: String) {
        self.0.inject_bundle_id(pid, bundle)
    }

    pub fn inject_window_kind(&self, window_id: WindowId, class: WindowClass) {
        self.0.inject_window_kind(window_id, class)
    }

    pub fn raised(&self) -> Vec<WindowId> {
        self.0.raised()
    }

    pub fn forgotten(&self) -> Vec<WindowId> {
        self.0.forgotten()
    }

    /// Pids pruned-for (forget_pid calls), with eviction counts.
    pub fn forgotten_pids(&self) -> Vec<(i32, usize)> {
        self.0.forgotten_pids()
    }

    /// Window ids evicted by the untracked cache prune, in order.
    pub fn pruned(&self) -> Vec<WindowId> {
        self.0.pruned()
    }

    /// Number of aged-observer replacements served.
    pub fn refreshes(&self) -> usize {
        self.0.refreshes()
    }

    /// Age every attached observer by `secs` (refresh-aging simulation).
    pub fn age_observers_for_test(&self, secs: u64) {
        self.0.age_observers_for_test(secs)
    }

    /// Simulate an app quitting without its termination notification.
    pub fn simulate_missed_termination(&self, pid: i32) {
        self.0.simulate_missed_termination(pid)
    }

    /// Attached observer pids (the refresh/dead-pid sweep subjects).
    pub fn observers(&self) -> Vec<i32> {
        self.0.observers.borrow().iter().copied().collect()
    }

    /// Reset the eviction logs so a later episode starts empty.
    pub fn clear_forgotten_for_test(&self) {
        self.0.forgotten.borrow_mut().clear();
        self.0.forgotten_pids.borrow_mut().clear();
        self.0.pruned.borrow_mut().clear();
    }
}

/// Prod-facing view of a shared `TestAdapter`: implements only the prod
/// `OsAdapter` interface by delegating to the shared cells.
pub struct SharedTestAdapter(Rc<TestAdapter>);

impl ObserverRegistry for SharedTestAdapter {
    fn attach_observer(&mut self, pid: i32) {
        self.0.attach(pid);
    }

    fn detach_observer(&mut self, pid: i32) {
        self.0.detach(pid);
    }

    fn observed_pids(&self) -> Vec<i32> {
        self.0.observers.borrow().iter().copied().collect()
    }

    fn refresh_aged_observers(&mut self, older_than: Duration) {
        self.0.refresh_aged_impl(older_than)
    }
}

impl OsAdapter for SharedTestAdapter {
    fn running_app_pids(&self) -> Vec<i32> {
        self.0.running_app_pids()
    }

    fn frontmost_pid(&self) -> Option<i32> {
        self.0.frontmost_pid()
    }

    fn poll_windows_for_pid(&self, pid: i32) -> WindowListing {
        self.0.poll_windows_for_pid(pid)
    }

    fn focused_window_for_pid(&self, pid: i32) -> Option<WindowId> {
        self.0.focused_window_for_pid(pid)
    }

    fn active_displays(&self) -> Vec<DisplayInfo> {
        self.0.active_displays()
    }

    fn primary_display_id(&self) -> u32 {
        self.0.primary_display_id()
    }

    fn set_window_rect(&self, window_id: WindowId, rect: Rect) -> WriteOutcome {
        self.0.set_window_rect(window_id, rect)
    }

    fn window_rect(&self, window_id: WindowId) -> Option<Rect> {
        self.0.window_rect(window_id)
    }

    fn window_kind(&self, window_id: WindowId) -> Option<WindowClass> {
        self.0.window_kind(window_id)
    }

    fn raise_window(&self, window_id: WindowId) {
        self.0.raise_window(window_id)
    }

    fn focus_window(&self, window_id: WindowId) {
        self.0.focus_window(window_id)
    }

    fn close_window(&self, window_id: WindowId) {
        self.0.close_window(window_id)
    }

    fn hide_windows(&self, placements: &HashMap<WindowId, HidePlacement>) {
        self.0.hide_windows(placements)
    }

    fn window_is_hidden(&self, window_id: WindowId) -> bool {
        self.0.window_is_hidden(window_id)
    }

    fn window_is_fullscreen(&self, window_id: WindowId) -> bool {
        self.0.window_is_fullscreen(window_id)
    }

    fn forget_window(&self, window_id: WindowId) {
        self.0.forget_window(window_id)
    }

    fn forget_pid(&self, pid: i32) -> usize {
        self.0.forget_pid(pid)
    }

    fn prune_untracked_elements(&self, keep: &HashSet<WindowId>) -> usize {
        self.0.prune_untracked_elements(keep)
    }

    fn running_pids_any_policy(&self) -> Vec<i32> {
        self.0.running_pids_any_policy()
    }

    fn app_bundle_id(&self, pid: i32) -> Option<String> {
        self.0.app_bundle_id(pid)
    }

    fn app_name(&self, pid: i32) -> Option<String> {
        self.0.app_name(pid)
    }
}

impl ObserverRegistry for TestAdapter {
    fn attach_observer(&mut self, pid: i32) {
        self.attach(pid);
    }

    fn detach_observer(&mut self, pid: i32) {
        self.detach(pid);
    }

    fn observed_pids(&self) -> Vec<i32> {
        self.observers.borrow().iter().copied().collect()
    }

    fn refresh_aged_observers(&mut self, older_than: Duration) {
        self.refresh_aged_impl(older_than)
    }
}

impl OsAdapter for TestAdapter {
    fn running_app_pids(&self) -> Vec<i32> {
        self.running_apps.borrow().clone()
    }

    fn frontmost_pid(&self) -> Option<i32> {
        self.frontmost
    }

    fn poll_windows_for_pid(&self, pid: i32) -> WindowListing {
        if self.ax_blackout.get() {
            // AX answering nothing: the blackout signature, not death
            // evidence.
            return WindowListing::NoAnswer;
        }
        let windows = self.windows.borrow().get(&pid).cloned().unwrap_or_default();
        // Prod's poll re-caches every listed element (fresh AXUIElementRef
        // per window); the fake mirrors that so prune semantics match.
        {
            let mut cached = self.cached_elements.borrow_mut();
            for &wid in &windows {
                cached.insert(wid, pid);
            }
        }
        // Empty but answered: the closed-only-window signature (Preview /
        // 1Password after their one window closed).
        WindowListing::Answered(windows)
    }

    fn focused_window_for_pid(&self, pid: i32) -> Option<WindowId> {
        self.focused_windows.borrow().get(&pid).copied()
    }

    fn active_displays(&self) -> Vec<DisplayInfo> {
        // Topology tests swap the snapshot mid-life (`TestHandle::set_displays`)
        // — a lost hotplug event is simulated by what the *list* says.
        if let Some(over) = self.display_override.borrow().as_ref() {
            return over.clone();
        }
        self.displays.clone()
    }

    fn primary_display_id(&self) -> u32 {
        self.active_displays().first().map(|d| d.id).unwrap_or(0)
    }

    fn set_window_rect(&self, window_id: WindowId, rect: Rect) -> WriteOutcome {
        self.set_rect_calls.set(self.set_rect_calls.get() + 1);
        if let Some(fault) = self.faults.borrow().get(&window_id).cloned() {
            // Faults serve the matching outcome without touching the OS
            // rect. Drift/Pinned stamp target/actual live: the target is
            // what layout just asked for, the actual is what the OS holds.
            return match fault {
                Fault::Gone => WriteOutcome::Gone,
                Fault::Transient(msg) => WriteOutcome::Transient(msg),
                Fault::Drift => WriteOutcome::Drift {
                    target: rect,
                    actual: self.window_rect(window_id).unwrap_or(rect),
                },
                Fault::Pinned => WriteOutcome::Pinned {
                    target: rect,
                    actual: self.window_rect(window_id).unwrap_or(rect),
                },
            };
        }
        self.window_rects.borrow_mut().insert(window_id, rect);
        WriteOutcome::Ok
    }

    fn close_window(&self, window_id: WindowId) {
        self.window_rects.borrow_mut().remove(&window_id);
        if let Some(pid) = self.window_pids.borrow_mut().remove(&window_id) {
            if let Some(windows) = self.windows.borrow_mut().get_mut(&pid) {
                windows.retain(|w| *w != window_id);
            }
        }
    }

    fn window_rect(&self, window_id: WindowId) -> Option<Rect> {
        if self.ax_blackout.get() {
            return None;
        }
        self.window_rects.borrow().get(&window_id).copied()
    }

    fn window_kind(&self, window_id: WindowId) -> Option<WindowClass> {
        self.window_classes.borrow().get(&window_id).copied()
    }

    fn raise_window(&self, window_id: WindowId) {
        self.raised.borrow_mut().push(window_id);
    }

    fn focus_window(&self, window_id: WindowId) {
        self.last_focused.set(Some(window_id));
        if let Some(pid) = self.window_pids.borrow().get(&window_id).copied() {
            self.focused_windows.borrow_mut().insert(pid, window_id);
        }
    }

    fn hide_windows(&self, placements: &HashMap<WindowId, HidePlacement>) {
        // Position-only like prod: move offscreen, keep size (no reflow).
        for (&wid, placement) in placements {
            let at = placement.rect();
            let mut rects = self.window_rects.borrow_mut();
            match rects.get(&wid).copied() {
                Some(cur) => {
                    rects.insert(
                        wid,
                        Rect {
                            x: at.x,
                            y: at.y,
                            width: cur.width,
                            height: cur.height,
                        },
                    );
                }
                None => {
                    rects.insert(wid, placement.rect());
                }
            }
        }
    }

    fn window_is_hidden(&self, window_id: WindowId) -> bool {
        self.hidden_windows.borrow().contains(&window_id)
            || self
                .window_pids
                .borrow()
                .get(&window_id)
                .is_some_and(|pid| self.hidden_apps.borrow().contains(pid))
    }

    fn window_is_fullscreen(&self, window_id: WindowId) -> bool {
        self.fullscreen_windows.borrow().contains(&window_id)
    }

    fn forget_window(&self, window_id: WindowId) {
        self.forgotten.borrow_mut().push(window_id);
        self.cached_elements.borrow_mut().remove(&window_id);
    }

    fn forget_pid(&self, pid: i32) -> usize {
        let mut evicted = 0usize;
        self.cached_elements.borrow_mut().retain(|_, &mut p| {
            if p == pid {
                evicted += 1;
                false
            } else {
                true
            }
        });
        self.forgotten_pids.borrow_mut().push((pid, evicted));
        evicted
    }

    fn prune_untracked_elements(&self, keep: &HashSet<WindowId>) -> usize {
        let mut evicted = Vec::new();
        self.cached_elements.borrow_mut().retain(|wid, _| {
            if keep.contains(wid) {
                true
            } else {
                evicted.push(*wid);
                false
            }
        });
        let count = evicted.len();
        self.pruned.borrow_mut().extend(evicted);
        count
    }
    fn running_pids_any_policy(&self) -> Vec<i32> {
        // Liveness = the running list plus every pid with listed windows:
        // a real app with windows is running regardless of activation
        // policy, which is exactly what the prod list answers.
        let mut pids: HashSet<i32> = self.running_apps.borrow().iter().copied().collect();
        pids.extend(self.window_pids.borrow().values().copied());
        pids.into_iter().collect()
    }

    fn app_bundle_id(&self, pid: i32) -> Option<String> {
        self.bundle_ids.borrow().get(&pid).cloned()
    }

    fn app_name(&self, pid: i32) -> Option<String> {
        self.app_names
            .borrow()
            .get(&pid)
            .cloned()
            .or_else(|| self.bundle_ids.borrow().get(&pid).cloned())
    }
}
