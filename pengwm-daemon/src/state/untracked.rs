use crate::adapter::{DisplayInfo, WindowClass};
use pengwm_core::layout::Rect;
use pengwm_core::tree::WindowId;

use super::StateManager;

/// Owns the untracked-window raise funnel (ADR-0002). Untracked windows
/// are store-tracked, tree-less overlays: macOS keeps a level ≠ 0 window
/// above the normal layer on its own, but a level-0 `float_apps` window
/// (System Settings) stacks against the newly shown tree after every
/// workspace switch. One `AXRaise` per untracked window on the monitor
/// whose visible workspace changed is the same mechanism popups use on
/// switch-back reveal — macOS stacking after un-hide is unspecified.
impl StateManager {
    /// Raise untracked windows after a visibility change. `monitor` scopes
    /// the pass to the monitor whose visible workspace changed (the
    /// containing-rect resolution is the helper popups share); `None`
    /// raises on every monitor (wake resync: stacking is globally unset).
    pub(super) fn raise_untracked_windows(&mut self, monitor: Option<u32>) {
        let untracked: Vec<WindowId> = self
            .store
            .all_window_pids()
            .keys()
            .copied()
            .filter(|&wid| self.os.window_kind(wid) == Some(WindowClass::Untracked))
            .collect();
        if untracked.is_empty() {
            return;
        }
        let displays = self.os.active_displays();
        for wid in untracked {
            if let Some(monitor) = monitor {
                let Some(rect) = self.os.window_rect(wid) else {
                    continue;
                };
                if display_containing_point(&displays, rect) != Some(monitor) {
                    continue;
                }
            }
            log::debug!("raising untracked window {} (visibility changed)", wid);
            self.os.raise_window(wid);
        }
    }

    /// True when the window is classified untracked (cached class from
    /// discovery + routing).
    pub(super) fn is_untracked(&self, window_id: WindowId) -> bool {
        self.os.window_kind(window_id) == Some(WindowClass::Untracked)
    }
}

/// Flat id of the display whose bounds contain `rect`'s center. The shared
/// containment math: arguments in, one answer — no monitor arithmetic at
/// any call site.
pub(super) fn display_containing_point(displays: &[DisplayInfo], rect: Rect) -> Option<u32> {
    let cx = rect.x + rect.width / 2.0;
    let cy = rect.y + rect.height / 2.0;
    for display in displays {
        let (ox, oy) = display.origin;
        let (w, h) = display.size;
        if Rect::new(ox as f64, oy as f64, w as f64, h as f64).contains_point(cx, cy) {
            return Some(display.id);
        }
    }
    None
}
