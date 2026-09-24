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
/// commands.rs` and `tests.rs` keep calling the same interface.
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

        let mut dead = Vec::new();
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
                    self.layout_fail_logged.remove(&window_id);
                }
                Err(e) => {
                    let msg = e.to_string();
                    // Permanent-gone signals: the adapter already refreshed +
                    // re-discovered and still failed. Verify the OS no longer
                    // lists the window before dropping it (transient AX
                    // hiccups stay tracked and retry next layout).
                    if msg.contains("kAXErrorInvalidUIElement")
                        || msg.contains("element not found in cache")
                    {
                        let still_exists = self
                            .store
                            .pid_for(window_id)
                            .map(|pid| self.os.poll_windows_for_pid(pid).contains(&window_id))
                            .unwrap_or(false);
                        if !still_exists {
                            log::warn!(
                                "apply_layout: window {} gone ({}), untracking",
                                window_id,
                                msg
                            );
                            dead.push(window_id);
                            continue;
                        }
                    }
                    // Expected-transient AX contention, not a daemon bug.
                    // Live resizes reject size writes with kAXErrorFailure /
                    // kAXErrorCannotComplete until the drag settles, and a
                    // mid-flight element refresh surfaces InvalidUIElement
                    // while the window still exists. Warn once per window
                    // per throttle window, then debug — the write retries on
                    // the next layout anyway, so ERROR on every retry is spam.
                    if is_transient_ax_error(&msg) {
                        self.log_transient_layout_failure(window_id, &msg);
                    } else {
                        log::error!(
                            "apply_layout: set_window_rect failed for window {}: {}",
                            window_id,
                            e
                        );
                    }
                }
            }
        }
        // Untrack via the normal destroyed path (removes from tree + store,
        // re-layouts the visible workspace to fill the gap).
        for window_id in dead {
            self.on_window_destroyed(window_id);
        }
    }

    /// Throttled log for expected-transient layout failures: warn on the
    /// first failure per window per throttle window, debug on repeats.
    /// Repeats mean the next layout is still retrying the same contested
    /// write (e.g. an in-progress live resize), not new information.
    fn log_transient_layout_failure(&mut self, window_id: WindowId, msg: &str) {
        const FAIL_LOG_THROTTLE: Duration = Duration::from_secs(5);
        let now = Instant::now();
        let repeat = self
            .layout_fail_logged
            .get(&window_id)
            .is_some_and(|last| now.duration_since(*last) < FAIL_LOG_THROTTLE);
        if repeat {
            log::debug!(
                "apply_layout: set_window_rect retry failed for window {}: {}",
                window_id,
                msg
            );
        } else {
            log::warn!(
                "apply_layout: set_window_rect failed for window {}: {} (retrying)",
                window_id,
                msg
            );
            self.layout_fail_logged.insert(window_id, now);
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

/// AX failures that are expected while a window is being live-resized or is
/// mid-flight through an element refresh — the write retries on the next
/// layout, so these are throttled warnings, not errors. Anything else (e.g.
/// permission loss) stays an error. Gone-window signals are classified by the
/// caller via the OS listing check before reaching here.
fn is_transient_ax_error(msg: &str) -> bool {
    // Size writes rejected while the user holds the resize handle, or while
    // the app clamps the size (min/max, fullscreen): the next layout retries.
    msg.contains("kAXErrorFailure")
        || msg.contains("kAXErrorCannotComplete")
        // Stale element for a window the OS still lists: the refresh race,
        // not a close. (Truly gone windows are untracked by the caller.)
        || msg.contains("kAXErrorInvalidUIElement")
        || msg.contains("element not found in cache")
        // Persistent position/size drift (Firefox frame-vs-content shift,
        // login-time not-yet-resizable windows): the write never landed so
        // `applied_rects` must not record success. Retry on next layout.
        || msg.contains("drift did not converge")
        || msg.contains("drift attempt")
}

#[cfg(test)]
mod tests {
    use super::is_transient_ax_error;

    #[test]
    fn classifies_live_resize_contention_as_transient() {
        assert!(is_transient_ax_error(
            "AXUIElementSetAttributeValue size error: kAXErrorFailure"
        ));
        assert!(is_transient_ax_error(
            "AXUIElementSetAttributeValue size error: kAXErrorCannotComplete"
        ));
        assert!(is_transient_ax_error(
            "AXUIElementSetAttributeValue position error: kAXErrorInvalidUIElement"
        ));
        assert!(is_transient_ax_error(
            "element not found in cache for window 17912"
        ));
        assert!(is_transient_ax_error(
            "set_window_rect drift did not converge target Rect { x: 0.0, y: 0.0, width: 100.0, height: 100.0 } actual Rect { x: 10.0, y: 10.0, width: 100.0, height: 100.0 }"
        ));
    }

    #[test]
    fn unexpected_errors_stay_errors() {
        assert!(!is_transient_ax_error("AXIsProcessTrusted() == false"));
        assert!(!is_transient_ax_error("some unknown io failure"));
    }
}
