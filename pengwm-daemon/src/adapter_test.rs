use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};

use crate::adapter::{DisplayInfo, ObserverRegistry, OsAdapter};
use pengwm_core::layout::{HidePlacement, Rect};
use pengwm_core::tree::WindowId;

pub struct TestAdapter {
    pub running_apps: Vec<i32>,
    pub frontmost: Option<i32>,
    pub windows: RefCell<HashMap<i32, Vec<WindowId>>>,
    pub window_pids: RefCell<HashMap<WindowId, i32>>,
    pub window_rects: RefCell<HashMap<WindowId, Rect>>,
    pub displays: Vec<DisplayInfo>,
    pub focused_windows: RefCell<HashMap<i32, WindowId>>,
    pub last_focused: Cell<Option<WindowId>>,
    pub observers: RefCell<HashSet<i32>>,
    pub bundle_ids: RefCell<HashMap<i32, String>>,
    pub app_names: RefCell<HashMap<i32, String>>,
    pub hidden_windows: RefCell<HashSet<WindowId>>,
    pub hidden_apps: RefCell<HashSet<i32>>,
    pub gone_windows: RefCell<HashSet<WindowId>>,
    /// Windows whose `set_window_rect` fails transiently with
    /// `kAXErrorFailure` (live resize contention) while the OS still lists
    /// them. Unlike `gone_windows`, these must stay tracked and retry.
    pub transient_fail_windows: RefCell<HashSet<WindowId>>,
    /// Windows stuck in Firefox-style position/size drift: the AX write is
    /// accepted by the call but never converges, so `set_window_rect`
    /// returns a transient drift error without updating the OS rect. Models
    /// login-time not-yet-resizable windows. Must stay tracked and retry —
    /// never poison `applied_rects`.
    pub drift_windows: RefCell<HashSet<WindowId>>,
    /// Number of `set_window_rect` calls served. Lets tests observe whether
    /// `apply_layout` skipped redundant writes.
    pub set_rect_calls: Cell<usize>,
}

impl Default for TestAdapter {
    fn default() -> Self {
        Self {
            running_apps: Vec::new(),
            frontmost: None,
            windows: RefCell::new(HashMap::new()),
            window_pids: RefCell::new(HashMap::new()),
            window_rects: RefCell::new(HashMap::new()),
            displays: Vec::new(),
            focused_windows: RefCell::new(HashMap::new()),
            last_focused: Cell::new(None),
            observers: RefCell::new(HashSet::new()),
            bundle_ids: RefCell::new(HashMap::new()),
            app_names: RefCell::new(HashMap::new()),
            hidden_windows: RefCell::new(HashSet::new()),
            hidden_apps: RefCell::new(HashSet::new()),
            gone_windows: RefCell::new(HashSet::new()),
            transient_fail_windows: RefCell::new(HashSet::new()),
            drift_windows: RefCell::new(HashSet::new()),
            set_rect_calls: Cell::new(0),
        }
    }
}

impl TestAdapter {
    pub fn new() -> Self {
        Self::default()
    }
}

impl ObserverRegistry for TestAdapter {
    fn attach_observer(&mut self, pid: i32) {
        self.observers.borrow_mut().insert(pid);
    }

    fn detach_observer(&mut self, pid: i32) {
        self.observers.borrow_mut().remove(&pid);
    }
}

impl OsAdapter for TestAdapter {
    fn running_app_pids(&self) -> Vec<i32> {
        self.running_apps.clone()
    }

    fn frontmost_pid(&self) -> Option<i32> {
        self.frontmost
    }

    fn poll_windows_for_pid(&self, pid: i32) -> Vec<WindowId> {
        self.windows.borrow().get(&pid).cloned().unwrap_or_default()
    }

    fn focused_window_for_pid(&self, pid: i32) -> Option<WindowId> {
        self.focused_windows.borrow().get(&pid).copied()
    }

    fn active_displays(&self) -> Vec<DisplayInfo> {
        self.displays.clone()
    }

    fn primary_display_id(&self) -> u32 {
        self.displays.first().map(|d| d.id).unwrap_or(0)
    }

    fn set_window_rect(&self, window_id: WindowId, rect: Rect) -> anyhow::Result<()> {
        self.set_rect_calls.set(self.set_rect_calls.get() + 1);
        if self.gone_windows.borrow().contains(&window_id) {
            anyhow::bail!("element not found in cache for window {}", window_id);
        }
        if self.transient_fail_windows.borrow().contains(&window_id) {
            anyhow::bail!("AXUIElementSetAttributeValue size error: kAXErrorFailure");
        }
        if self.drift_windows.borrow().contains(&window_id) {
            anyhow::bail!(
                "set_window_rect drift did not converge target {:?} actual {:?}",
                rect,
                self.window_rects.borrow().get(&window_id).copied()
            );
        }
        self.window_rects.borrow_mut().insert(window_id, rect);
        Ok(())
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
        self.window_rects.borrow().get(&window_id).copied()
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

    fn inject_window(&self, pid: i32, window_id: WindowId) {
        self.windows
            .borrow_mut()
            .entry(pid)
            .or_default()
            .push(window_id);
        self.window_pids.borrow_mut().insert(window_id, pid);
    }

    fn inject_app_name(&self, pid: i32, name: String) {
        self.app_names.borrow_mut().insert(pid, name);
    }

    fn inject_bundle_id(&self, pid: i32, bundle: String) {
        self.bundle_ids.borrow_mut().insert(pid, bundle);
    }

    fn window_rect_for_test(&self, window_id: WindowId) -> Option<Rect> {
        self.window_rect(window_id)
    }

    fn set_rect_calls_for_test(&self) -> usize {
        self.set_rect_calls.get()
    }

    fn fail_rect_for_test(&self, window_id: WindowId) {
        self.gone_windows.borrow_mut().insert(window_id);
    }

    fn fail_transient_for_test(&self, window_id: WindowId) {
        self.transient_fail_windows.borrow_mut().insert(window_id);
    }

    fn clear_transient_for_test(&self, window_id: WindowId) {
        self.transient_fail_windows.borrow_mut().remove(&window_id);
    }

    fn fail_drift_for_test(&self, window_id: WindowId) {
        self.drift_windows.borrow_mut().insert(window_id);
    }

    fn clear_drift_for_test(&self, window_id: WindowId) {
        self.drift_windows.borrow_mut().remove(&window_id);
    }

    fn displace_window_for_test(&self, window_id: WindowId, dx: f64, dy: f64) {
        let mut rects = self.window_rects.borrow_mut();
        if let Some(r) = rects.get_mut(&window_id) {
            r.x += dx;
            r.y += dy;
        }
    }
}
