use std::time::{Duration, Instant};

use pengwm_core::tree::WindowId;

use super::drag::DragTickAction;
use super::StateManager;

/// Owns the tick path: the hidden-state reconcile (predicate-injected, no
/// downcast), the missed-window sweep, and the drag-to-swap tick. Shared
/// helpers stay `pub(super)` for the test harness.
impl StateManager {
    pub fn on_tick(&mut self) {
        let now = Instant::now();

        // Fallback hidden reconcile via predicate — no downcast.
        if self.store.should_reconcile(now) {
            self.reconcile_hidden_windows();
        }

        // Safety net for windows whose WindowCreated notification never
        // arrived (Firefox tear-offs / incognito often don't fire it, or
        // fire while transiently non-manageable so the observer drops it).
        // `poll_windows_for_pid` already filters to manageable windows, so
        // PiP / dialog windows stay excluded here just as on the event path.
        const WINDOW_SWEEP_INTERVAL: Duration = Duration::from_secs(2);
        if now.duration_since(self.last_window_sweep) >= WINDOW_SWEEP_INTERVAL {
            self.last_window_sweep = now;
            self.reconcile_new_windows();
            // Tracked-but-misplaced: login-time Firefox often drifts or fails
            // its first write (not-yet-resizable) and gets stuck — manual
            // re-tile is skipped via `applied_rects`, and the new-window
            // sweep above ignores tracked ids. Re-assert those here.
            self.reconcile_misplaced_windows(now);
        }

        match self.drag.on_tick(
            now,
            &self.workspaces,
            &self.last_layout_rects,
            self.active_workspace_idx(),
        ) {
            DragTickAction::Swap {
                workspace_idx,
                drag,
                target,
            } => {
                let ws = &mut self.workspaces[workspace_idx];
                if ws.swap_windows_by_id(drag, target) {
                    self.apply_layout(workspace_idx);
                }
            }
            DragTickAction::SnapBack { workspace_idx } => {
                self.apply_layout(workspace_idx);
            }
            DragTickAction::None => {}
        }
    }

    #[allow(dead_code)]
    fn clear_drag_state(&mut self) {
        self.drag.clear();
    }

    /// Re-check every tracked window's actual hidden/minimized state and bring
    /// the tree in line: untile windows the OS reports as hidden, and retile
    /// ones that became visible again. Uses predicate injection so tests don't
    /// need `as_any_mut` downcast.
    fn reconcile_hidden_windows(&mut self) {
        // Snapshot to avoid borrow conflicts with &mut self in the loop.
        let window_ids: Vec<WindowId> = self.store.all_window_pids().keys().copied().collect();
        let (to_hide, to_show) = self
            .store
            .pending_for_reconcile(&self.workspaces, |wid| self.os.window_is_hidden(wid));
        // Hide first, then show — order doesn't matter but hide frees capacity.
        for wid in to_hide {
            // Only hide if still tiled (pending set already checked, but window
            // may have been destroyed between pending calc and now).
            if self.find_workspace_for_window(wid).is_some() {
                self.on_window_hidden(wid);
            }
        }
        for wid in to_show {
            // Only show if hidden-tracked; pending already checked.
            if self.store.is_hidden(wid) {
                self.on_window_shown(wid);
            }
        }
        // Keep unused variable for clarity if pending logic changes.
        let _ = window_ids;
    }

    /// Poll every running app for windows the store has never seen and tile
    /// them via the normal `on_window_created` path (routing, capacity,
    /// monocle, layout all shared — no duplicate logic). Only brand-new
    /// window ids are touched: tracked-but-untiled windows are either hidden
    /// (owned by `HiddenTracker`) or intentionally overflowed at capacity.
    fn reconcile_new_windows(&mut self) {
        let pids: Vec<i32> = self
            .os
            .running_app_pids()
            .into_iter()
            .filter(|pid| !self.excluded_pids.contains(pid))
            .collect();
        for pid in pids {
            for window_id in self.os.poll_windows_for_pid(pid) {
                if !self.store.contains(window_id) {
                    log::info!(
                        "reconcile_new_windows: discovered untracked window {} pid {}",
                        window_id,
                        pid
                    );
                    self.on_window_created(window_id, pid);
                }
            }
        }
    }

    /// Re-assert tracked windows whose OS rect disagrees with the tiled
    /// target (login-time Firefox drift / failed first write). Without this,
    /// a drifted write poisons `applied_rects` (or, post-fix, a transient
    /// failure leaves the window tracked-but-untiled) and neither manual
    /// re-tile (skip-if-unchanged) nor `reconcile_new_windows`
    /// (`!contains` only) ever retries it — only a full app quit + reopen
    /// (fresh WindowId) heals it.
    ///
    /// Guards mirror `note_displaced`: skips hidden windows, the active drag
    /// window, moves within grace/epsilon (our own animation settling), and
    /// invisible workspaces. Invalidates `applied_rects` for genuinely
    /// displaced windows then re-applies those workspaces.
    fn reconcile_misplaced_windows(&mut self, now: Instant) {
        const MISPLACED_GRACE: Duration = Duration::from_millis(500);
        const MISPLACED_EPSILON: f64 = 8.0;
        let drag_window = self.drag.drag_window();
        let mut affected: Vec<usize> = Vec::new();
        // Snapshot visible indices first to avoid borrow conflicts.
        let visible: Vec<usize> = self.displays.active().values().copied().collect();
        for idx in visible {
            if idx >= self.workspaces.len() {
                continue;
            }
            let targets = self.workspaces[idx].layout(self.gap_inner, self.gap_outer);
            let mut misplaced = false;
            for (&wid, target) in &targets {
                if Some(wid) == drag_window {
                    continue;
                }
                if self.store.is_hidden(wid) {
                    continue;
                }
                let actual = match self.os.window_rect(wid) {
                    Some(r) => r,
                    None => continue,
                };
                let displaced = (actual.x - target.x).abs() > MISPLACED_EPSILON
                    || (actual.y - target.y).abs() > MISPLACED_EPSILON
                    || (actual.width - target.width).abs() > MISPLACED_EPSILON
                    || (actual.height - target.height).abs() > MISPLACED_EPSILON;
                if !displaced {
                    continue;
                }
                // Recent writes are animation settling, not external drift.
                if let Some((_, written_at)) = self.applied_rects.get(&wid) {
                    if now.duration_since(*written_at) < MISPLACED_GRACE {
                        continue;
                    }
                }
                log::info!(
                    "reconcile_misplaced: window {} displaced target ({:.0},{:.0} {}x{}) actual ({:.0},{:.0} {}x{}), invalidating",
                    wid,
                    target.x,
                    target.y,
                    target.width,
                    target.height,
                    actual.x,
                    actual.y,
                    actual.width,
                    actual.height
                );
                self.applied_rects.remove(&wid);
                misplaced = true;
            }
            if misplaced {
                affected.push(idx);
            }
        }
        for idx in affected {
            self.apply_layout(idx);
        }
    }

    #[cfg(test)]
    pub(super) fn force_window_sweep_for_test(&mut self) {
        self.last_window_sweep = Instant::now() - Duration::from_secs(5);
    }
}
