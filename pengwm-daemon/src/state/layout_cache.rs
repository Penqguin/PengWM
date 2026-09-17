use std::time::{Duration, Instant};

use pengwm_core::layout::Rect;
use pengwm_core::tree::WindowId;

use super::StateManager;

/// Owns the layout-write cache policy: skip-if-unchanged (the Firefox
/// reflow storm), the post-write grace/epsilon that keeps our own animation
/// settling from tripping snap-back, and the hidden-rect seeding that keeps
/// switch-back from comparing equal to a stale tiled entry. `StateManager`
/// retains the maps, the workspace tree and the `OsAdapter` — this module
/// only hides the policy. Everything is `pub(super)` so `mod.rs`,
/// `commands.rs` and `tests.rs` keep calling the same interface.
impl StateManager {
    pub(super) fn apply_layout(&mut self, workspace_idx: usize) {
        let rects = self.workspaces[workspace_idx].layout(self.gap_inner, self.gap_outer);
        self.last_layout_rects = rects.clone();

        log::debug!(
            "apply_layout ws={} gaps_in={} out={}:",
            workspace_idx,
            self.gap_inner,
            self.gap_outer
        );
        for (&window_id, rect) in &rects {
            log::debug!(
                "  win={} -> ({:.0},{:.0}) {}x{}",
                window_id,
                rect.x,
                rect.y,
                rect.width,
                rect.height
            );
        }

        for (&window_id, rect) in &rects {
            // Skip windows already at their target — redundant AX writes are
            // what makes Firefox crawl (reflow per write).
            if self.applied_rects.get(&window_id).map(|(r, _)| r) == Some(rect) {
                continue;
            }
            match self.os.set_window_rect(window_id, *rect) {
                Ok(()) => {
                    self.applied_rects
                        .insert(window_id, (*rect, Instant::now()));
                }
                Err(e) => {
                    log::error!(
                        "apply_layout: set_window_rect failed for window {}: {}",
                        window_id,
                        e
                    );
                }
            }
        }
    }

    /// If the window is genuinely displaced from where we put it, forget
    /// the applied entry so the next layout re-asserts (snap-back, app
    /// moves). Two guards keep our own writes from tripping this:
    /// moves within the post-write grace window are our animation
    /// settling, and moves within a few px are jitter, not a drag.
    pub(super) fn note_displaced(&mut self, window_id: WindowId, x: f64, y: f64) {
        const MOVE_GRACE: Duration = Duration::from_millis(500);
        const MOVE_EPSILON: f64 = 8.0;
        if let Some((target, written_at)) = self.applied_rects.get(&window_id) {
            let now = Instant::now();
            let displaced =
                (x - target.x).abs() > MOVE_EPSILON || (y - target.y).abs() > MOVE_EPSILON;
            if displaced && now.duration_since(*written_at) > MOVE_GRACE {
                log::debug!(
                    "on_window_moved: window {} displaced to ({:.0},{:.0}), invalidating applied rect",
                    window_id,
                    x,
                    y
                );
                self.applied_rects.remove(&window_id);
            }
        }
    }

    /// Record where hidden windows were put: a hidden rect never equals a
    /// future tile target, so this can't cause a wrongful skip — worst case
    /// one extra write. Without it, a window hidden after being tiled would
    /// compare equal to its stale tiled entry and never come back on
    /// switch-back.
    pub(super) fn seed_hidden_rect(
        &mut self,
        rect: Rect,
        window_ids: impl IntoIterator<Item = WindowId>,
    ) {
        let written_at = Instant::now();
        for wid in window_ids {
            self.applied_rects.insert(wid, (rect, written_at));
        }
    }

    #[cfg(test)]
    pub(super) fn age_applied_for_test(&mut self, window_id: WindowId, age: Duration) {
        if let Some((rect, _)) = self.applied_rects.get(&window_id).copied() {
            self.applied_rects
                .insert(window_id, (rect, Instant::now() - age));
        }
    }
}
