use crate::event_loop::DaemonEvent;
use pengwm_core::layout::Rect;
use pengwm_core::tree::WindowId;
use std::any::Any;

pub trait OsAdapter {
    fn running_app_pids(&self) -> Vec<i32>;
    fn frontmost_pid(&self) -> Option<i32>;
    fn poll_windows_for_pid(&mut self, pid: i32) -> Vec<WindowId>;
    fn focused_window_for_pid(&self, pid: i32) -> Option<WindowId>;
    fn active_displays(&self) -> Vec<DisplayInfo>;
    fn primary_display_id(&self) -> u32;
    fn set_window_rect(&mut self, window_id: WindowId, rect: Rect) -> anyhow::Result<()>;
    fn focus_window(&mut self, window_id: WindowId);
    fn close_window(&mut self, window_id: WindowId);
    fn hide_windows(&mut self, window_ids: &[WindowId]);
    /// True when the window is minimized or hidden (per-window `AXHidden`,
    /// `AXMinimized`, or its app is hidden). Used by the periodic reconcile so
    /// hidden windows stop being tiled even when AX notifications are missed.
    fn window_is_hidden(&self, window_id: WindowId) -> bool;
    fn attach_observer(&mut self, pid: i32);
    fn detach_observer(&mut self, pid: i32);
    fn app_bundle_id(&self, pid: i32) -> Option<String>;
    /// Human-readable display name for the app owning `pid` (e.g. "Safari"),
    /// distinct from its bundle id ("com.apple.Safari"). Drives the menubar's
    /// per-window app labels.
    fn app_name(&self, pid: i32) -> Option<String>;
    fn with_callback(callback: Box<dyn Fn(DaemonEvent) + Send>) -> Self
    where
        Self: Sized;
    /// Test seam: downcast to the concrete adapter (used by daemon state tests).
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

#[derive(Clone)]
pub struct DisplayInfo {
    pub id: u32,
    pub origin: (i32, i32),
    pub size: (u32, u32),
}
