use std::collections::HashMap;
use std::time::{Duration, Instant};

use pengwm_core::layout::Rect;
use pengwm_core::tree::WindowId;

use super::drag::DragTickAction;
use super::StateManager;

/// Owns the tick path: the hidden-state reconcile (predicate-injected, no
/// downcast), the missed-window sweep, and the drag-to-swap tick. Shared
/// helpers stay `pub(super)` for the test harness.
impl StateManager {
    pub fn on_tick(&mut self) {
        let now = Instant::now();

        // A wake resync waits here for AX to come back. Driven first: the
        // sweeps below all query AX, and running them against a blacked-out
        // subsystem is what starts gone-grace timers on live windows.
        self.drive_wake_resync(now);

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
    }

    /// Safety net for windows whose WindowCreated notification never
    /// arrived (Firefox tear-offs / incognito often don't fire it, or
    /// fire while transiently non-manageable so the observer drops it).
    /// Delegates to the shared discovery loop; see `sync_untracked_windows`.
    fn reconcile_new_windows(&mut self) {
        let pids: Vec<i32> = self.os.running_app_pids();
        self.sync_untracked_windows(pids);
    }

    /// Re-assert tracked windows whose OS rect disagrees with the tiled
    /// target (login-time Firefox drift / failed first write). Without this,
    /// a drifted write poisons `applied_rects` (or, post-fix, a transient
    /// failure leaves the window tracked-but-untiled) and neither manual
    /// re-tile (skip-if-unchanged) nor `reconcile_new_windows`
    /// (`!contains` only) ever retries it — only a full app quit + reopen
    /// (fresh WindowId) heals it.
    ///
    /// One sweep funnel: visible workspaces only (invisible ones aren't
    /// asserted until shown). Displacement judgment lives in
    /// `LayoutWriteCache::sweep_displaced`; this only feeds reads and
    /// re-applies what came back invalidated.
    fn reconcile_misplaced_windows(&mut self, now: Instant) {
        let drag_window = self.drag.drag_window();
        // Snapshot visible indices first to avoid borrow conflicts.
        let visible: Vec<usize> = self.displays.active().values().copied().collect();
        let mut affected: Vec<usize> = Vec::new();
        for idx in visible {
            if idx >= self.workspaces.len() {
                continue;
            }
            let targets = self.workspaces[idx].layout(self.gap_inner, self.gap_outer);
            let actuals: HashMap<WindowId, Option<Rect>> = targets
                .keys()
                .map(|&wid| (wid, self.os.window_rect(wid)))
                .collect();
            let invalidated = self
                .layout_cache
                .sweep_displaced(&targets, &actuals, now, |wid| {
                    Some(wid) == drag_window || self.store.is_hidden(wid)
                });
            if !invalidated.is_empty() {
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
