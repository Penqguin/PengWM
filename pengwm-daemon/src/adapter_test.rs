use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};

use crate::adapter::{DisplayInfo, ObserverRegistry, OsAdapter};
use pengwm_core::layout::Rect;
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

    fn focus_window(&self, window_id: WindowId) {
        self.last_focused.set(Some(window_id));
        if let Some(pid) = self.window_pids.borrow().get(&window_id).copied() {
            self.focused_windows.borrow_mut().insert(pid, window_id);
        }
    }

    fn hide_windows(&self, window_ids: &[WindowId]) {
        // 0×0 far off-screen — even 1×1 leaves a 1px sliver due to
        // macOS position clamping. 0×0 is fully invisible.
        let offscreen = Rect {
            x: -100_000.0,
            y: -100_000.0,
            width: 0.0,
            height: 0.0,
        };
        for &wid in window_ids {
            self.window_rects.borrow_mut().insert(wid, offscreen);
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
        self.windows.borrow_mut().entry(pid).or_default().push(window_id);
        self.window_pids.borrow_mut().insert(window_id, pid);
    }

    fn inject_app_name(&self, pid: i32, name: String) {
        self.app_names.borrow_mut().insert(pid, name);
    }

    fn inject_bundle_id(&self, pid: i32, bundle: String) {
        self.bundle_ids.borrow_mut().insert(pid, bundle);
    }
}
