use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use pengwm_core::layout::Rect;
use pengwm_core::tree::WindowId;

use super::drag::DragTickAction;
use super::StateManager;

/// Replace observers attached longer than this. AX accumulates
/// per-registration records for an observer's lifetime (no
/// per-notification removal), so a long-lived app's observer grows with
/// window churn until it is replaced — previously only on wake resync or
/// app relaunch. Age-bounded replacement keeps the bound regardless of
/// whether the machine ever sleeps.
const OBSERVER_MAX_AGE: Duration = Duration::from_secs(15 * 60);

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

        // Placement persistence: a dirty session drains to disk at most
        // once per interval. The projection reads no OS state, so this
        // is safe mid-blackout and costs nothing when clean.
        self.save_session_if_dirty(now);

        // Fallback hidden reconcile via predicate — no downcast. The
        // fullscreen exit probe rides the same interval: one tick of
        // latency to catch a unfullscreen, guaranteed, because the parked
        // set is tiny and probed unconditionally.
        if self.store.should_reconcile(now) {
            self.reconcile_hidden_windows();
            self.reconcile_fullscreen_windows();
            self.reconcile_dead_pids();
            self.os.refresh_aged_observers(OBSERVER_MAX_AGE);
        }

        // Safety net for windows whose WindowCreated notification never
        // arrived (Firefox tear-offs / incognito often don't fire it, or
        // fire while transiently non-manageable so the observer drops it).
        // The poll classifies instead of dropping now: popups reach
        // `on_window_created` and land in their workspace's popup set.
        const WINDOW_SWEEP_INTERVAL: Duration = Duration::from_secs(2);
        if now.duration_since(self.last_window_sweep) >= WINDOW_SWEEP_INTERVAL {
            self.last_window_sweep = now;
            // Topology heals first: a missed/out-of-order display event used
            // to leave stale geometry that the misplaced sweep below would
            // then re-assert every 2s (writes into no-man's land, forever).
            // The poll is one cheap CG query; a no-op answer costs nothing.
            self.run_topology_sync();
            self.reconcile_new_windows();
            // Tracked-but-misplaced: login-time Firefox often drifts or fails
            // its first write (not-yet-resizable) and gets stuck — manual
            // re-tile is skipped via `applied_rects`, and the new-window
            // sweep above ignores tracked ids. Re-assert those here.
            self.reconcile_misplaced_windows(now);
            // Cache-vs-store diff: elements cached for windows the store
            // doesn't track have no eviction path of their own — released
            // here. Runs after the sweep so windows it just registered
            // keep their fresh elements.
            self.prune_untracked_elements();
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

    /// Recover from missed `AppTerminated` notifications: any pid we still
    /// observe or still track windows for, but that NSWorkspace no longer
    /// reports running, goes through the normal `on_app_terminated` funnel.
    ///
    /// NSWorkspace liveness is the one authoritative non-AX evidence for
    /// death: a dead app's own AX poll is *always* empty — the "zero
    /// listing is no judgment" blackout signature — so the listing sweep
    /// can never untrack it, and only this diff can. Runs `on_app_terminated`
    /// rather than a bespoke cleanup so the missed path and the notified
    /// path retire an app exactly the same way.
    ///
    /// Suppressed while a wake resync is pending, mirroring the listing
    /// sweep: the resync re-attaches and re-polls everything at commit, and
    /// the next sweep cleans up after it.
    fn reconcile_dead_pids(&mut self) {
        if !self.may_judge("dead-pid sweep") {
            return;
        }
        let running: HashSet<i32> = self.os.running_pids_any_policy().into_iter().collect();
        // Suspects: every pid with an attached observer (a stranded
        // observer is the leak) plus every pid whose windows we still
        // track (a stranded tree entry is the symptom).
        let mut suspect: HashSet<i32> = self.os.observed_pids().into_iter().collect();
        suspect.extend(self.store.all_pids().keys().copied());
        let dead: Vec<i32> = suspect
            .into_iter()
            .filter(|pid| !running.contains(pid))
            .collect();
        for pid in dead {
            log::info!(
                "dead-pid sweep: pid {} no longer running (termination missed) — running cleanup",
                pid
            );
            self.on_app_terminated(pid);
        }
    }

    /// Evict cached elements for windows the store doesn't track — the one
    /// rule that gives every cached ref a release path, no matter which
    /// notification was missed. The `keep` set is the store's tracked
    /// windows: parked and popup members stay tracked while they wait, so
    /// prunes never touch live state.
    fn prune_untracked_elements(&mut self) {
        let keep: HashSet<WindowId> = self.store.all_window_pids().keys().copied().collect();
        let evicted = self.os.prune_untracked_elements(&keep);
        if evicted > 0 {
            log::debug!(
                "prune: released {} cached element(s) for windows the store doesn't track",
                evicted
            );
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
        // One batched read for the whole sweep — but only for the windows
        // whose verdict this reconcile can act on: current tree/popup
        // members (hide candidates) and Hide-parked windows (show
        // candidates). Fullscreen-parked windows and Untracked overlays
        // are deliberately absent: `pending_for_reconcile` can only
        // discard their answers (fullscreen reveal is owned by the
        // probe, overlays have no reveal funnel), so polling them paid
        // up to two AX RPCs each per second for an unusable verdict.
        // The read set narrows with the verdicts; the adapter's
        // app-hidden resolution follows (one app element per pid that
        // still has a candidate).
        let members: HashSet<WindowId> = self
            .workspaces
            .iter()
            .flat_map(|ws| ws.all_windows().into_iter().chain(ws.popup_ids()))
            .collect();
        let mut read_set: Vec<WindowId> = members.iter().copied().collect();
        for wid in self.store.hide_parked() {
            if !members.contains(&wid) {
                read_set.push(wid);
            }
        }
        let hidden = self.os.windows_are_hidden(&read_set);
        let (to_hide, to_show) = self.store.pending_for_reconcile(&self.workspaces, &hidden);
        // Hide first, then show — order doesn't matter but hide frees capacity.
        for wid in to_hide {
            // Only hide if still a member (pending set already checked, but
            // window may have been destroyed between pending calc and now).
            if self.find_workspace_of_any(wid).is_some() {
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

    /// Per-tick fullscreen exit probe: every fullscreen-parked window that
    /// no longer reports `AXFullScreen` reveals through the normal funnel
    /// (remembered workspace, capacity overflow beyond). Probing the parked
    /// set — never the tree — is what makes unconditional probing cheap.
    ///
    /// Suppressed while a wake resync is pending: a blacked-out AX reports
    /// *every* attribute as unreadable (→ `false`), which would mass-reveal
    /// fullscreened windows into a dead layout pass. The gate is the
    /// blackout module's `may_judge` — the same funnel the listing sweep
    /// and the dead-pid sweep ask.
    fn reconcile_fullscreen_windows(&mut self) {
        if !self.may_judge("fullscreen reconcile") {
            return;
        }
        for window_id in self.store.fullscreen_parked() {
            if !self.os.window_is_fullscreen(window_id) {
                self.on_fullscreen_exit(window_id);
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
    /// re-applies what came back invalidated. The read set comes from
    /// `sweep_plan` — only at-target windows past the write grace get
    /// their actual read (the drift detector); unsettled windows are
    /// already scheduled for a write, so their workspace goes to
    /// re-apply without a read. Popups are skipped (free-drag
    /// after placement — nothing re-centers them) via the exclusion
    /// predicate, and so is anything the user is dragging or the OS
    /// reports hidden.
    fn reconcile_misplaced_windows(&mut self, now: Instant) {
        let drag_window = self.drag.drag_window();
        // Snapshot visible indices and the popup membership first — both are
        // read inside the sweep closure while `layout_cache` is borrowed.
        let visible: Vec<usize> = self.displays.active().values().copied().collect();
        let popups: std::collections::HashSet<WindowId> = self
            .workspaces
            .iter()
            .flat_map(|ws| ws.popup_ids())
            .collect();
        let mut affected: Vec<usize> = Vec::new();
        for idx in visible {
            if idx >= self.workspaces.len() {
                continue;
            }
            let targets = self.workspaces[idx].layout(self.gap_inner, self.gap_outer);
            let is_skipped = |wid: WindowId| {
                Some(wid) == drag_window || self.store.is_hidden(wid) || popups.contains(&wid)
            };
            // The read set is the plan's: at-target windows only, grace
            // fresh ones included. Everything else either owns its
            // geometry or is already scheduled for a write.
            let (needs_read, unsettled) = self.layout_cache.sweep_plan(&targets, now, is_skipped);
            let actuals: HashMap<WindowId, Option<Rect>> = needs_read
                .iter()
                .map(|&wid| (wid, self.os.window_rect(wid)))
                .collect();
            let invalidated = self
                .layout_cache
                .sweep_displaced(&targets, &actuals, now, is_skipped);
            if !invalidated.is_empty() || unsettled {
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
