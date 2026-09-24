use std::collections::HashMap;

use pengwm_core::layout::{HidePlacement, Rect};
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
    fn set_window_rect(&self, window_id: WindowId, rect: Rect) -> anyhow::Result<()>;
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

    #[cfg(test)]
    fn inject_window(&self, pid: i32, window_id: WindowId);
    #[cfg(test)]
    fn inject_app_name(&self, pid: i32, name: String);
    #[cfg(test)]
    fn inject_bundle_id(&self, pid: i32, bundle: String);
    #[cfg(test)]
    fn window_rect_for_test(&self, window_id: WindowId) -> Option<Rect>;
    /// Number of `set_window_rect` calls served. Lets layout tests observe
    /// whether redundant writes were skipped.
    #[cfg(test)]
    fn set_rect_calls_for_test(&self) -> usize;
    /// Mark a window as permanently gone: `set_window_rect` fails with the
    /// same "element not found" error prod returns after refresh+discover
    /// miss. Lets tests exercise dead-window cleanup.
    #[cfg(test)]
    fn fail_rect_for_test(&self, window_id: WindowId);
    /// Mark a window as transiently contested: `set_window_rect` fails with
    /// `kAXErrorFailure` (live resize) while the OS still lists it. Lets
    /// tests exercise the throttled-retry path (stays tracked, retries).
    #[cfg(test)]
    fn fail_transient_for_test(&self, window_id: WindowId);
    #[cfg(test)]
    fn clear_transient_for_test(&self, window_id: WindowId);
    /// Mark a window as drift-stuck: `set_window_rect` returns the transient
    /// drift error without updating the OS rect. Lets tests exercise the
    /// login-time Firefox path (tracked-but-misplaced retries, no
    /// `applied_rects` poisoning).
    #[cfg(test)]
    fn fail_drift_for_test(&self, window_id: WindowId);
    #[cfg(test)]
    fn clear_drift_for_test(&self, window_id: WindowId);
    /// Externally displace a window's OS rect (user drag / app move
    /// simulation). Lets tests exercise the misplaced reconcile without
    /// reaching through the adapter.
    #[cfg(test)]
    fn displace_window_for_test(&self, window_id: WindowId, dx: f64, dy: f64);
}

#[derive(Clone)]
pub struct DisplayInfo {
    pub id: u32,
    pub origin: (i32, i32),
    pub size: (u32, u32),
}
