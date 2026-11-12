use serde::{Deserialize, Serialize};

use super::Workspace;
use crate::tree::WindowId;

/// Declaration order is admission priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum PendingKind {
    Returning,
    Explicit,
    Traveling,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingWindow {
    pub window_id: WindowId,
    pub kind: PendingKind,
    pub order: u64,
}

impl Workspace {
    pub fn owns(&self, id: WindowId) -> bool {
        self.find_window(id).is_some() || self.is_popup(id) || self.pending_window(id).is_some()
    }

    pub fn pending_window(&self, id: WindowId) -> Option<PendingWindow> {
        self.pending.iter().find(|p| p.window_id == id).copied()
    }

    pub fn add_pending(&mut self, window: PendingWindow) {
        if !self.owns(window.window_id) {
            self.pending.push(window);
            self.pending.sort_by_key(|p| p.order);
        }
    }

    pub fn remove_pending(&mut self, id: WindowId) {
        self.pending.retain(|p| p.window_id != id);
        self.clear_overlay_focus(id);
    }

    pub fn clear_overlay_focus(&mut self, id: WindowId) {
        if self.focused_overlay == Some(id) {
            self.focused_overlay = None;
        }
    }

    /// The render set: every window drawn as a centered overlay above
    /// the tiling — pending (overflow) windows plus popups. Not a
    /// membership answer: use `owns` for that.
    pub fn overlay_ids(&self) -> Vec<WindowId> {
        self.pending
            .iter()
            .map(|p| p.window_id)
            .chain(self.popups.iter().copied())
            .collect()
    }

    /// Fill slots atomically in priority/opening order. Ownership stays here.
    pub fn fill_slots(&mut self, capacity: usize) -> Vec<WindowId> {
        let mut candidates = self.pending.clone();
        candidates.sort_by_key(|p| (p.kind, p.order));
        let available = capacity.saturating_sub(self.window_count());
        let focused = self.focused_window_id();
        let mut admitted = Vec::new();
        for window in candidates.into_iter().take(available) {
            self.remove_pending(window.window_id);
            self.add_window(window.window_id, None);
            admitted.push(window.window_id);
        }
        if let Some(id) = focused.filter(|id| self.owns(*id)) {
            self.focus_window(id);
        }
        admitted
    }

    pub fn cycle_window(&mut self, backward: bool) -> Option<WindowId> {
        let ids = self.all_owned();
        if ids.is_empty() {
            return None;
        }
        let current = self
            .focused_window_id()
            .and_then(|id| ids.iter().position(|i| *i == id));
        let next = match (current, backward) {
            (Some(i), true) => (i + ids.len() - 1) % ids.len(),
            (Some(i), false) => (i + 1) % ids.len(),
            (None, true) => ids.len() - 1,
            (None, false) => 0,
        };
        self.focus_window(ids[next]);
        Some(ids[next])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_allocation_prioritizes_home_then_opening_order() {
        let mut ws = Workspace::new("test".into(), 1, (0, 0), (1920, 1080));
        for (window_id, kind) in [
            (1, PendingKind::Traveling),
            (2, PendingKind::Explicit),
            (3, PendingKind::Returning),
            (4, PendingKind::Explicit),
        ] {
            ws.add_pending(PendingWindow {
                window_id,
                kind,
                order: window_id,
            });
        }
        ws.focus_window(1);
        assert_eq!(ws.fill_slots(3), [3, 2, 4]);
        assert_eq!(ws.focused_window_id(), Some(1));
        assert_eq!(ws.pending.len(), 1);
        assert_eq!(ws.all_owned().len(), 4);
        assert!(ws.fill_slots(3).is_empty());
    }

    #[test]
    fn removing_overlay_without_tree_clears_ownership_and_focus() {
        let mut ws = Workspace::new("test".into(), 1, (0, 0), (1920, 1080));
        let window = PendingWindow {
            window_id: 1,
            kind: PendingKind::Traveling,
            order: 1,
        };
        ws.add_pending(window);
        ws.add_pending(window);
        ws.focus_window(1);
        assert_eq!(ws.all_owned(), [1]);
        ws.remove_window(1);
        assert!(ws.all_owned().is_empty());
        assert_eq!(ws.focused_window_id(), None);
    }
}
