//! Executes membership transitions and their placement effects.
//!
//! Workspace owns membership; WindowStore owns verdicts and Park. Callers
//! choose intent and destination, never assemble the bookkeeping sequence.
use pengwm_core::tree::WindowId;
use pengwm_core::workspace::{position_of_member, PendingKind, PendingWindow};

use super::store::{ParkReason, ParkedEntry};

use super::StateManager;

/// Carry invalidates an individual write target before placement. Topology
/// recovery leaves that to the existing whole-topology cache reset, and its
/// membership bookkeeping remains permitted while the destination is gated.
enum PendingTransfer {
    Carry,
    TopologyRecovery,
}

impl StateManager {
    /// Move `wid` from workspace `from` to `to`, re-laying-out visible ends.
    /// Shared by workspace moves and cross-display moves. The admit tail
    /// (`StateManager::place_admitted`) owns the destination placement:
    /// a window thrown at a hidden workspace takes the same hide
    /// placement a routed one does, or it would float over the source
    /// screen until the next switch. Full targets host a bound overlay.
    pub(super) fn transfer_window(&mut self, from: usize, to: usize, wid: WindowId) {
        let popup = self.detach_member(from, wid);
        if self.displays.is_visible(from, &self.workspaces) {
            self.apply_layout(from);
        }
        if popup {
            self.admit_popup(wid, to);
        } else {
            self.admit_window(wid, to, PendingKind::Explicit);
        }
        self.session_dirty = true;
    }

    /// Remove any membership shape and report whether it was a popup.
    /// Callers capture placement/Park first and decide when to re-layout.
    fn detach_member(&mut self, idx: usize, id: WindowId) -> bool {
        let popup = self.workspaces[idx].is_popup(id);
        if popup {
            self.workspaces[idx].remove_popup(id);
        } else {
            self.workspaces[idx].remove_window(id);
        }
        popup
    }

    fn transfer_pending_member(
        &mut self,
        from: usize,
        to: usize,
        window: PendingWindow,
        cause: PendingTransfer,
    ) {
        self.workspaces[from].remove_pending(window.window_id);
        self.workspaces[to].add_pending(window);
        self.deferred_hides.remove(&window.window_id);
        if matches!(cause, PendingTransfer::Carry) {
            self.layout_cache.invalidate(window.window_id);
        }
        self.note_where(window.window_id);
    }

    pub(super) fn relocate_detached_overflow(&mut self, previous: &[u32]) -> Option<usize> {
        let live = self.os.active_displays();
        let target = self.displays.visible_on(self.os.primary_display_id())?;
        let mut moved = false;
        for (idx, monitor) in previous.iter().enumerate() {
            if live.iter().any(|display| display.id == *monitor) || idx == target {
                continue;
            }
            // Topology ownership recovery remains bookkeeping when the target
            // is gated. The ordinary placement tail defers all OS writes.
            let windows: Vec<_> = self.workspaces[idx]
                .pending
                .iter()
                .filter(|p| p.kind == PendingKind::Traveling)
                .copied()
                .collect();
            moved |= !windows.is_empty();
            for window in windows {
                self.transfer_pending_member(
                    idx,
                    target,
                    window,
                    PendingTransfer::TopologyRecovery,
                );
            }
            moved |= self.store.relocate_parked_overflow(idx, target);
        }
        self.session_dirty |= moved;
        moved.then_some(target)
    }

    pub(super) fn admit_window(&mut self, id: WindowId, target: usize, kind: PendingKind) {
        if target >= self.workspaces.len() || self.find_workspace_of_any(id).is_some() {
            return;
        }
        let order = self.store.order(id);
        self.workspaces[target].add_pending(PendingWindow {
            window_id: id,
            kind,
            order,
        });
        self.fill_workspace_slots(target);
        self.note_where(id);
        self.session_dirty = true;
        self.place_admitted(id, target);
    }

    pub(super) fn fill_workspace_slots(&mut self, idx: usize) {
        if !self.workspace_may_act(idx) || !self.may_judge("overflow admission") {
            return;
        }
        if self.workspaces[idx].window_count() >= self.displays.max_tiles()
            || self.workspaces[idx].pending.is_empty()
        {
            return;
        }
        self.park_hidden_pending(idx);
        let admitted = self.workspaces[idx].fill_slots(self.displays.max_tiles());
        if !admitted.is_empty() {
            self.session_dirty = true;
        }
        for id in admitted {
            self.note_where(id);
            self.layout_cache.invalidate(id);
        }
    }

    pub(super) fn carry_overflow(&mut self, from: usize, to: usize) {
        if from == to
            || !self.workspace_may_act(from)
            || !self.workspace_may_act(to)
            || !self.may_judge("overflow carry")
        {
            return;
        }
        self.park_hidden_pending(from);
        let traveling: Vec<_> = self.workspaces[from]
            .pending
            .iter()
            .filter(|p| p.kind == PendingKind::Traveling)
            .copied()
            .collect();
        for window in traveling {
            self.transfer_pending_member(from, to, window, PendingTransfer::Carry);
        }
        self.fill_workspace_slots(to);
        self.session_dirty = true;
    }

    /// Carry a pending member to a resolved display workspace, preserving intent.
    pub(super) fn transfer_pending_window(
        &mut self,
        idx: usize,
        target: usize,
        id: WindowId,
    ) -> bool {
        let Some(window) = self.workspaces[idx].pending_window(id) else {
            return false;
        };
        self.transfer_pending_member(idx, target, window, PendingTransfer::Carry);
        self.fill_workspace_slots(target);
        self.session_dirty = true;
        for end in [idx, target] {
            if self.displays.is_visible(end, &self.workspaces) {
                self.apply_layout(end);
            }
        }
        true
    }

    fn park_hidden_pending(&mut self, idx: usize) {
        let ids: Vec<_> = self.workspaces[idx]
            .pending
            .iter()
            .map(|p| p.window_id)
            .collect();
        if ids.is_empty() {
            return;
        }
        // App-hide emits one event per window. Park hidden siblings before
        // filling the first freed slot, preserving their traveling intent.
        let hidden = self.os.windows_are_hidden(&ids);
        for id in ids {
            if hidden.get(&id) != Some(&true) {
                continue;
            }
            self.park_current_member(id, ParkReason::Hide);
            self.session_dirty = true;
        }
    }

    /// Capture before removal: both the seatbelt and Park inspect membership.
    /// Placement is owned by the enclosing operation so hidden siblings all
    /// park before any freed slot is offered to the remaining candidates.
    fn park_current_member(&mut self, id: WindowId, reason: ParkReason) -> Option<usize> {
        self.note_where(id);
        let idx = self.find_workspace_of_any(id)?;
        if self.workspaces[idx].is_popup(id) {
            debug_assert_eq!(reason, ParkReason::Hide);
            self.store.park_popup(id, idx);
        } else {
            self.store.park(id, &self.workspaces, reason)?;
        }
        self.detach_member(idx, id);
        Some(idx)
    }

    /// Preserve discovery's existing popup placement: only visible workspaces
    /// are laid out here. Return and transfer use the hidden-placement tail.
    pub(super) fn admit_discovered_popup(&mut self, id: WindowId, target: usize) {
        if self.workspaces[target].add_popup(id)
            && self.displays.is_visible(target, &self.workspaces)
        {
            self.apply_layout(target);
        }
    }

    fn admit_popup(&mut self, id: WindowId, target: usize) -> bool {
        let added = self.workspaces[target].add_popup(id);
        self.place_admitted(id, target);
        added
    }

    /// Returns whether the event caller should publish a membership change.
    /// Upgrading an existing fullscreen Park retains the old no-publish path.
    pub(super) fn park_hidden_member(&mut self, window_id: WindowId) -> bool {
        self.session_dirty = true;
        if self.store.is_fullscreen_parked(window_id) {
            // A hide landed while the window sat fullscreen-parked: upgrade
            // the parking reason and keep the remembered idx. The fullscreen
            // exit probe will now leave it alone — the hidden flow owns the
            // window from here, exactly as if it had never fullscreened.
            log::debug!(
                "WindowHidden: fullscreen-parked window {} upgrades to hidden park",
                window_id
            );
            self.store.upgrade_park_to_hide(window_id);
            return false;
        }
        let Some(idx) = self.find_workspace_of_any(window_id) else {
            log::debug!("WindowHidden: window {} not tracked, ignoring", window_id);
            return false;
        };
        debug_assert!(idx < self.workspaces.len());
        self.park_current_member(window_id, ParkReason::Hide);
        if self.displays.is_visible(idx, &self.workspaces) {
            self.apply_layout(idx);
        }
        true
    }

    pub(super) fn return_hidden_member(&mut self, window_id: WindowId) -> bool {
        if self.store.is_fullscreen_parked(window_id) {
            // The probe owns this window's reveal; a stray shown event must
            // not shortcut it.
            log::debug!(
                "WindowShown: fullscreen-parked window {}, ignoring",
                window_id
            );
            return false;
        }
        if self.find_workspace_of_any(window_id).is_some() {
            return false;
        }
        let pid = match self.store.pid_for(window_id) {
            Some(p) => p,
            None => {
                log::debug!("WindowShown: unknown window {}, ignoring", window_id);
                return false;
            }
        };
        let remembered = self.store.reveal(window_id);
        if let Some(entry) = remembered.filter(|e| e.popup && e.idx < self.workspaces.len()) {
            self.admit_popup(window_id, entry.idx);
            return true;
        }
        self.return_tree_member(window_id, pid, remembered);
        true
    }

    fn return_tree_member(
        &mut self,
        window_id: WindowId,
        pid: i32,
        remembered: Option<ParkedEntry>,
    ) {
        let pending = remembered.and_then(|e| e.pending);
        let preferred = remembered
            .map(|e| e.idx)
            .filter(|&idx| idx < self.workspaces.len())
            .unwrap_or_else(|| {
                let identity = self.app_identity(pid);
                self.where_should(window_id, pid)
                    .or_else(|| self.routed_workspace_for(&identity))
                    .unwrap_or_else(|| self.active_workspace_idx())
            });
        self.admit_window(
            window_id,
            preferred,
            pending.map(|p| p.kind).unwrap_or(PendingKind::Returning),
        );
    }

    pub(super) fn park_fullscreen_member(&mut self, window_id: WindowId) -> bool {
        if !self.fullscreen_parking_enabled() {
            return false;
        }
        if !self.store.contains(window_id) || self.store.is_parked(window_id) {
            return false;
        }
        // Popups never tile anyway; parking would be no-op overhead.
        if self.is_popup_anywhere(window_id) {
            return false;
        }
        // The fullscreen probe is the gate: a merely pinned (zoomed / busy
        // app) window answers false here and keeps its tree slot.
        if !self.os.window_is_fullscreen(window_id) {
            return false;
        }
        let Some(idx) = self.park_current_member(window_id, ParkReason::Fullscreen) else {
            log::debug!(
                "FullscreenEnter: window {} not in any tree, ignoring",
                window_id
            );
            return false;
        };
        log::info!(
            "FullscreenEnter: parking window {} (workspace {}), siblings reflow",
            window_id,
            idx
        );
        if self.displays.is_visible(idx, &self.workspaces) {
            self.apply_layout(idx);
        }
        true
    }

    pub(super) fn return_fullscreen_member(&mut self, window_id: WindowId) -> bool {
        let Some(entry) = self.store.reveal(window_id) else {
            return false;
        };
        let Some(pid) = self.store.pid_for(window_id) else {
            log::debug!("FullscreenExit: unknown window {}, ignoring", window_id);
            return false;
        };
        log::info!(
            "FullscreenExit: revealing window {} into workspace {:?}",
            window_id,
            entry.idx
        );
        debug_assert!(!entry.popup, "popups are exempt from fullscreen parking");
        self.return_tree_member(window_id, pid, Some(entry));
        true
    }

    pub(super) fn remove_member(&mut self, window_id: WindowId, confirmed_destroy: bool) {
        self.session_dirty = true;
        if let Some(idx) = self.find_workspace_of_any(window_id) {
            let is_visible = self.displays.is_visible(idx, &self.workspaces);
            self.detach_member(idx, window_id);
            // Preserve the checkpoint: freed slots are promoted/placed before
            // unregistering and retiring the removed window's cached element.
            if is_visible {
                self.apply_layout(idx);
            }
        }
        if let Some(pid) = self.store.unregister(window_id) {
            self.note_placement_closed(window_id, confirmed_destroy);
            self.prune_stale_placements(pid);
        }
        self.layout_cache.forget(window_id);
        self.deferred_hides.remove(&window_id);
        // One funnel for OS-side eviction: destroyed notification, missed
        // notification sweep, gone-grace, and app termination all reach
        // here, so the cached element and its registrations never outlive
        // the window on any path.
        self.os.forget_window(window_id);
    }

    pub(super) fn return_all_members(&mut self) -> bool {
        let revealed = self.store.reveal_all();
        if revealed.is_empty() {
            return false;
        }
        let mut retiled = false;
        for (wid, entry) in revealed {
            // Skip if already re-tiled via earlier iteration.
            if self.find_workspace_of_any(wid).is_some() {
                continue;
            }
            // Popups re-attach to their remembered workspace: no routing,
            // no capacity. The admit tail places them — hidden target
            // takes the hide placement like every other member.
            if entry.popup {
                if entry.idx < self.workspaces.len() {
                    retiled |= self.admit_popup(wid, entry.idx);
                }
                continue;
            }
            let pid = match self.store.pid_for(wid) {
                Some(p) => p,
                None => continue,
            };
            self.return_tree_member(wid, pid, Some(entry));
            retiled = true;
        }
        // Re-layout visible workspaces to ensure tiling clean.
        let visible: Vec<usize> = self.displays.active().values().copied().collect();
        for idx in visible {
            if idx < self.workspaces.len() {
                self.apply_layout(idx);
            }
        }
        retiled
    }

    /// Admit a verified restart survivor without disturbing restart priority.
    /// The caller verifies identity and processes former tiles before waiters.
    pub(super) fn restore_verified_member(
        &mut self,
        record: &super::session::LivePlacement,
        idx: usize,
    ) {
        let id = record.identity.window_id;
        let window = PendingWindow {
            window_id: id,
            kind: record.pending.unwrap_or(PendingKind::Returning),
            order: record.order,
        };
        let hidden = self
            .os
            .windows_are_hidden(&[id])
            .get(&id)
            .copied()
            .unwrap_or(false);
        let fullscreen = self.os.window_is_fullscreen(id) && self.fullscreen_parking_enabled();
        if hidden || fullscreen {
            self.store.restore_park(
                id,
                ParkedEntry {
                    idx,
                    popup: false,
                    pending: record.pending.map(|_| window),
                    reason: if hidden {
                        ParkReason::Hide
                    } else {
                        ParkReason::Fullscreen
                    },
                },
            );
        } else {
            self.workspaces[idx].add_pending(window);
            if record.pending.is_none() {
                self.fill_workspace_slots(idx);
            }
        }
    }

    pub(super) fn place_admitted(&mut self, window_id: WindowId, workspace_idx: usize) {
        if self.displays.is_visible(workspace_idx, &self.workspaces) {
            self.apply_layout(workspace_idx);
        } else {
            self.hide_window_in_workspace(window_id, workspace_idx);
        }
    }

    /// Workspace index holding `window_id` as a tree member, pending
    /// overlay, *or* popup. The membership-aware find: destroy/hide/show
    /// cleanup needs all three.
    pub(super) fn find_workspace_of_any(&self, window_id: WindowId) -> Option<usize> {
        position_of_member(&self.workspaces, window_id)
    }

    /// True when any workspace tracks `window_id` as a popup. Popups are
    /// excluded from tree-only funnels by this check: drag hit-testing,
    /// the displaced note, the misplaced sweep, capacity.
    pub(super) fn is_popup_anywhere(&self, window_id: WindowId) -> bool {
        self.workspaces.iter().any(|ws| ws.is_popup(window_id))
    }
}
