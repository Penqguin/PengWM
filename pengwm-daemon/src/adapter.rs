use std::collections::HashMap;

use pengwm_core::layout::{HidePlacement, Rect, WriteOutcome};
use pengwm_core::tree::WindowId;

pub trait ObserverRegistry {
    fn attach_observer(&mut self, pid: i32);
    fn detach_observer(&mut self, pid: i32);
}

pub trait OsAdapter: ObserverRegistry {
    fn running_app_pids(&self) -> Vec<i32>;
    fn frontmost_pid(&self) -> Option<i32>;
    fn poll_windows_for_pid(&self, pid: i32) -> Vec<WindowId>;
    fn focused_window_for_pid(&self, pid: i32) -> Option<WindowId>;
    fn active_displays(&self) -> Vec<DisplayInfo>;
    fn primary_display_id(&self) -> u32;
    fn set_window_rect(&self, window_id: WindowId, rect: Rect) -> WriteOutcome;
    /// Read back the current OS rect. Lets verify-and-retry policy and tests
    /// observe whether a write landed without reaching through FFI.
    fn window_rect(&self, window_id: WindowId) -> Option<Rect>;
    fn focus_window(&self, window_id: WindowId);
    fn close_window(&self, window_id: WindowId);
    /// Hide windows at precomputed placements. `placements` maps each
    /// `WindowId` to a `HidePlacement` variant so the adapter can match on
    /// intent instead of guessing from raw coordinates (no magic threshold).
    /// `StateManager` computes placements so the adapter stays display-agnostic
    /// and `pengwm-core` stays pure.
    fn hide_windows(&self, placements: &HashMap<WindowId, HidePlacement>);
    /// True when the window is minimized or hidden (per-window `AXHidden`,
    /// `AXMinimized`, or its app is hidden). Used by the periodic reconcile so
    /// hidden windows stop being tiled even when AX notifications are missed.
    fn window_is_hidden(&self, window_id: WindowId) -> bool;
    fn app_bundle_id(&self, pid: i32) -> Option<String>;
    /// Human-readable display name for the app owning `pid` (e.g. "Safari"),
    /// distinct from its bundle id ("com.apple.Safari"). Drives the menubar's
    /// per-window app labels.
    fn app_name(&self, pid: i32) -> Option<String>;
}

#[derive(Clone)]
pub struct DisplayInfo {
    pub id: u32,
    pub origin: (i32, i32),
    pub size: (u32, u32),
}
