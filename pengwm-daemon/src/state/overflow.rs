use pengwm_core::tree::{Direction, WindowId};
use pengwm_core::workspace::{PendingKind, PendingWindow};

use super::StateManager;

impl StateManager {
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
                self.workspaces[idx].remove_pending(window.window_id);
                self.workspaces[target].add_pending(window);
                self.deferred_hides.remove(&window.window_id);
                self.note_where(window.window_id);
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
            self.workspaces[from].remove_pending(window.window_id);
            self.workspaces[to].add_pending(window);
            self.deferred_hides.remove(&window.window_id);
            self.layout_cache.invalidate(window.window_id);
            self.note_where(window.window_id);
        }
        self.fill_workspace_slots(to);
        self.session_dirty = true;
    }

    /// Directional move for a focused pending overlay: carry it to the
    /// workspace of the display in `direction` and run the destination's
    /// slot fill (the same tail `carry_overflow` uses — room there tiles
    /// it, a full destination keeps it waiting as a bound overlay). No
    /// display in that direction leaves it in place, mirroring the tree
    /// swap's no-neighbor no-op. The window keeps its placement kind.
    pub(super) fn move_pending_overlay(&mut self, idx: usize, direction: Direction, id: WindowId) {
        let displays = self.os.active_displays();
        let Some(target) =
            self.displays
                .direction_target(&self.workspaces, idx, direction, &displays)
        else {
            return;
        };
        let Some(window) = self.workspaces[idx].pending_window(id) else {
            return;
        };
        self.workspaces[idx].remove_pending(id);
        self.workspaces[target].add_pending(window);
        self.deferred_hides.remove(&id);
        self.layout_cache.invalidate(id);
        self.note_where(id);
        self.fill_workspace_slots(target);
        self.session_dirty = true;
        for end in [idx, target] {
            if self.displays.is_visible(end, &self.workspaces) {
                self.apply_layout(end);
            }
        }
        // The moved window keeps focus across the move, like the tree
        // swap keeps the (re)focused tile and `move_window_between`
        // focuses its target.
        self.workspaces[target].focus_window(id);
        if self.displays.is_visible(target, &self.workspaces) && self.workspace_may_act(target) {
            self.os.focus_window(id);
        }
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
            self.note_where(id);
            self.store
                .park(id, &self.workspaces, super::store::ParkReason::Hide);
            self.workspaces[idx].remove_pending(id);
            self.session_dirty = true;
        }
    }
}
