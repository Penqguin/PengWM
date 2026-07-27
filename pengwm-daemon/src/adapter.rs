use pengwm_core::layout::Rect;
use pengwm_core::tree::WindowId;
use crate::event_loop::DaemonEvent;

pub trait OsAdapter {
    fn running_app_pids(&self) -> Vec<i32>;
    fn frontmost_pid(&self) -> Option<i32>;
    fn poll_windows_for_pid(&mut self, pid: i32) -> Vec<WindowId>;
    fn focused_window_for_pid(&self, pid: i32) -> Option<WindowId>;
    fn active_displays(&self) -> Vec<DisplayInfo>;
    fn primary_display_id(&self) -> u32;
    fn set_window_rect(&mut self, window_id: WindowId, rect: Rect) -> anyhow::Result<()>;
    fn close_window(&mut self, window_id: WindowId);
    fn hide_windows(&mut self, window_ids: &[WindowId]);
    fn attach_observer(&mut self, pid: i32);
    fn detach_observer(&mut self, pid: i32);
    fn with_callback(callback: Box<dyn Fn(DaemonEvent) + Send>) -> Self
    where
        Self: Sized;
}

#[derive(Clone)]
pub struct DisplayInfo {
    pub id: u32,
    pub origin: (i32, i32),
    pub size: (u32, u32),
}
