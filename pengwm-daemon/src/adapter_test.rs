#![cfg(test)]

use std::collections::{HashMap, HashSet};

use crate::adapter::{DisplayInfo, OsAdapter};
use crate::event_loop::DaemonEvent;
use pengwm_core::layout::Rect;
use pengwm_core::tree::WindowId;

pub struct TestAdapter {
    pub running_apps: Vec<i32>,
    pub frontmost: Option<i32>,
    pub windows: HashMap<i32, Vec<WindowId>>,
    pub window_pids: HashMap<WindowId, i32>,
    pub window_rects: HashMap<WindowId, Rect>,
    pub displays: Vec<DisplayInfo>,
    pub focused_windows: HashMap<i32, WindowId>,
    pub observers: HashSet<i32>,
    pub bundle_ids: HashMap<i32, String>,
}

impl TestAdapter {
    pub fn new() -> Self {
        Self {
            running_apps: Vec::new(),
            frontmost: None,
            windows: HashMap::new(),
            window_pids: HashMap::new(),
            window_rects: HashMap::new(),
            displays: Vec::new(),
            focused_windows: HashMap::new(),
            observers: HashSet::new(),
            bundle_ids: HashMap::new(),
        }
    }
}

impl OsAdapter for TestAdapter {
    fn running_app_pids(&self) -> Vec<i32> {
        self.running_apps.clone()
    }

    fn frontmost_pid(&self) -> Option<i32> {
        self.frontmost
    }

    fn poll_windows_for_pid(&mut self, pid: i32) -> Vec<WindowId> {
        self.windows.get(&pid).cloned().unwrap_or_default()
    }

    fn focused_window_for_pid(&self, pid: i32) -> Option<WindowId> {
        self.focused_windows.get(&pid).copied()
    }

    fn active_displays(&self) -> Vec<DisplayInfo> {
        self.displays.clone()
    }

    fn primary_display_id(&self) -> u32 {
        self.displays.first().map(|d| d.id).unwrap_or(0)
    }

    fn set_window_rect(&mut self, window_id: WindowId, rect: Rect) -> anyhow::Result<()> {
        self.window_rects.insert(window_id, rect);
        Ok(())
    }

    fn close_window(&mut self, window_id: WindowId) {
        self.window_rects.remove(&window_id);
        if let Some(pid) = self.window_pids.remove(&window_id) {
            if let Some(windows) = self.windows.get_mut(&pid) {
                windows.retain(|w| *w != window_id);
            }
        }
    }

    fn hide_windows(&mut self, window_ids: &[WindowId]) {
        let offscreen = Rect {
            x: -9999.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        };
        for &wid in window_ids {
            self.window_rects.insert(wid, offscreen);
        }
    }

    fn attach_observer(&mut self, pid: i32) {
        self.observers.insert(pid);
    }

    fn detach_observer(&mut self, pid: i32) {
        self.observers.remove(&pid);
    }

    fn app_bundle_id(&self, pid: i32) -> Option<String> {
        self.bundle_ids.get(&pid).cloned()
    }

    fn with_callback(_callback: Box<dyn Fn(DaemonEvent) + Send>) -> Self
    where
        Self: Sized,
    {
        TestAdapter::new()
    }

    fn update_workspace_indicator(
        &mut self,
        _workspaces: &[(&str, bool)],
        _display_width: f64,
        _display_height: f64,
    ) {
    }
}
