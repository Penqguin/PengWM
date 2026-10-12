use crate::tree::{Arena, NodeData, NodeId, SplitDirection, WindowId};

use super::Workspace;

impl Workspace {
    pub fn add_window(&mut self, window_id: WindowId, direction: Option<SplitDirection>) -> NodeId {
        if self.root.is_none() {
            let id = self.arena.alloc(NodeData::Window {
                window_id,
                is_focused: true,
            });
            self.root = Some(id);
            self.focused_node = Some(id);
            return id;
        }

        // Default master-stack: 1 window on the left, remaining stacked on the
        // right. Only use the generic split logic when an explicit direction
        // was requested (via keybind or pending_split).
        let explicit = direction.is_some() || self.pending_split.is_some();
        if !explicit {
            if let Some(id) = self.try_add_master_stack(window_id) {
                return id;
            }
        }

        let dir = direction
            .or_else(|| self.pending_split.take())
            .unwrap_or_else(|| self.next_direction());
        let focused = self
            .focused_node
            .expect("focused_node set when root exists");

        let flatten = self
            .arena
            .get(focused)
            .and_then(|n| n.parent)
            .and_then(|pid| self.arena.get(pid))
            .is_some_and(|p| matches!(&p.data, NodeData::Split { direction: d, .. } if *d == dir));

        if flatten {
            let parent_id = self.arena.get(focused).and_then(|n| n.parent).unwrap();
            let new_id = self.arena.alloc(NodeData::Window {
                window_id,
                is_focused: true,
            });
            self.arena.attach(parent_id, new_id);
            self.set_focused_node(new_id);
            return new_id;
        }

        let old_parent = self.arena.get(focused).and_then(|n| n.parent);
        let new_id = self.arena.alloc(NodeData::Window {
            window_id,
            is_focused: true,
        });
        let split_id = self.arena.alloc_split(dir, old_parent, &[focused, new_id]);
        match old_parent {
            Some(pid) => {
                self.arena.replace_child(pid, focused, split_id);
            }
            None => {
                self.root = Some(split_id);
            }
        }

        self.set_focused_node(new_id);
        new_id
    }

    /// Master-stack insertion: left master (single window), right stack
    /// (Horizontal split). Returns None if the current tree is not in
    /// master-stack shape and we should fall back to the generic splitter.
    fn try_add_master_stack(&mut self, window_id: WindowId) -> Option<NodeId> {
        let root_id = self.root?;
        // Single window → create Vertical [master, new]
        if self.arena.len() == 1 {
            if !matches!(self.arena.get(root_id)?.data, NodeData::Window { .. }) {
                return None;
            }
            let new_id = self.arena.alloc(NodeData::Window {
                window_id,
                is_focused: true,
            });
            let split_id =
                self.arena
                    .alloc_split(SplitDirection::Vertical, None, &[root_id, new_id]);
            self.root = Some(split_id);
            self.set_focused_node(new_id);
            return Some(new_id);
        }

        // Check master-stack shape: root Vertical with exactly 2 children,
        // left is Window, right is Window or Horizontal.
        let root_node = self.arena.get(root_id)?;
        let (dir, children) = match &root_node.data {
            NodeData::Split { direction, .. } => (*direction, root_node.children.clone()),
            _ => return None,
        };
        if dir != SplitDirection::Vertical || children.len() != 2 {
            return None;
        }
        let left_id = children[0];
        let right_id = children[1];
        let left_is_window = matches!(self.arena.get(left_id)?.data, NodeData::Window { .. });
        if !left_is_window {
            return None;
        }
        // Clone right data before mutable alloc to avoid borrow conflict
        let right_data = self.arena.get(right_id)?.data.clone();
        let new_id = self.arena.alloc(NodeData::Window {
            window_id,
            is_focused: true,
        });
        match &right_data {
            NodeData::Window { .. } => {
                // Right is single window → convert to Horizontal stack [right, new]
                let stack_id = self.arena.alloc_split(
                    SplitDirection::Horizontal,
                    Some(root_id),
                    &[right_id, new_id],
                );
                self.arena.replace_child(root_id, right_id, stack_id);
                self.set_focused_node(new_id);
                Some(new_id)
            }
            NodeData::Split { direction, .. } if *direction == SplitDirection::Horizontal => {
                // Right is already a Horizontal stack → append
                self.arena.attach(right_id, new_id);
                self.set_focused_node(new_id);
                Some(new_id)
            }
            _ => None,
        }
    }

    /// Attach `window_id` as a workspace-bound popup: tracked but never
    /// tiled. Idempotent; refuses when the window is already tiled (a
    /// window is either in the tree or a popup, never both). Returns
    /// whether membership changed.
    pub fn add_popup(&mut self, window_id: WindowId) -> bool {
        if self.find_window(window_id).is_some() || self.is_popup(window_id) {
            return false;
        }
        self.popups.push(window_id);
        true
    }

    /// Remove a popup (close / hide / untrack). Idempotent; returns
    /// whether it was a member.
    pub fn remove_popup(&mut self, window_id: WindowId) -> bool {
        let before = self.popups.len();
        self.popups.retain(|&wid| wid != window_id);
        self.popups.len() != before
    }

    pub fn remove_window(&mut self, window_id: WindowId) {
        let node_id = match self.find_window(window_id) {
            Some(id) => id,
            None => return,
        };

        self.clear_magnify_if(window_id);
        let was_focused = self.focused_node == Some(node_id);

        if self.arena.len() == 1 {
            self.arena.remove(node_id);
            self.root = None;
            self.focused_node = None;
            return;
        }

        let parent_id = self
            .arena
            .get(node_id)
            .and_then(|n| n.parent)
            .expect("non-root node has a parent");
        let removed_pos = self
            .arena
            .get(parent_id)
            .and_then(|p| p.children.iter().position(|&c| c == node_id));
        self.arena.remove(node_id);
        // Preserve manual sizes: drop the removed child's share and rescale
        // the survivors proportionally instead of re-equalizing. Only a
        // preset re-equalizes.
        if let Some(pos) = removed_pos {
            self.drop_ratio_at(parent_id, pos);
        }
        self.collapse_upward(parent_id);

        if self.root.is_none() {
            self.focused_node = None;
        } else if was_focused {
            self.focus_nearest_leaf();
        }
    }

    /// A child at `pos` was just removed from `split_id`: drop its share and
    /// rescale the survivors proportionally (no re-equalize on remove).
    fn drop_ratio_at(&mut self, split_id: NodeId, pos: usize) {
        let node = match self.arena.get_mut(split_id) {
            Some(n) => n,
            None => return,
        };
        if let NodeData::Split { ref mut ratios, .. } = &mut node.data {
            if pos < ratios.len() {
                ratios.remove(pos);
            }
            if ratios.is_empty() {
                return;
            }
            let sum: f64 = ratios.iter().sum();
            if sum > 0.0 {
                for r in ratios.iter_mut() {
                    *r /= sum;
                }
            } else {
                *ratios = Arena::equal_shares(ratios.len());
            }
        }
    }

    /// Replace the ratio at `pos` with `count` equal slices of its share.
    /// Used when a split's children are absorbed into its parent: the
    /// absorbed slot's share is divided, every other share is untouched.
    pub(super) fn split_ratio_slot(&mut self, split_id: NodeId, pos: usize, count: usize) {
        if count == 0 {
            return;
        }
        let node = match self.arena.get_mut(split_id) {
            Some(n) => n,
            None => return,
        };
        if let NodeData::Split { ref mut ratios, .. } = &mut node.data {
            if pos >= ratios.len() {
                return;
            }
            let share = ratios.remove(pos);
            let each = share / count as f64;
            for i in 0..count {
                ratios.insert(pos + i, each);
            }
        }
    }

    fn collapse_upward(&mut self, start_id: NodeId) {
        let mut current = Some(start_id);
        while let Some(id) = current {
            let (child_count, parent_id) = match self.arena.get(id) {
                Some(node) => (node.children.len(), node.parent),
                None => break,
            };

            if child_count >= 2 {
                // Ratios were fixed up at the removal site (`drop_ratio_at`);
                // survivors keep their shares — no re-equalize here.
                break;
            }

            if child_count == 0 {
                let next = parent_id;
                self.arena.remove(id);
                current = next;
                if next.is_none() {
                    self.root = None;
                }
                continue;
            }

            let only_child = self.arena.get(id).unwrap().children[0];

            // If the only remaining child is itself a split, absorb its
            // children into `id` rather than promoting it wholesale. Promoting
            // a differently-oriented split flips the layout (e.g. a vertical
            // left/right split becomes horizontal top/bottom); absorbing keeps
            // `id`'s orientation, e.g. one window on the left and the rest on
            // the right.
            if matches!(
                &self.arena.get(only_child).unwrap().data,
                NodeData::Split { .. }
            ) {
                let grandchildren: Vec<NodeId> =
                    self.arena.get(only_child).unwrap().children.clone();
                // Absorb: `id` takes the grandchildren, shares re-equalized
                // by the container's `equalize`.
                self.arena.set_children(id, &grandchildren);
                self.arena.equalize(id);
                self.arena.get_mut(only_child).unwrap().children.clear();
                self.arena.remove(only_child);
                continue;
            }

            self.arena.get_mut(only_child).unwrap().parent = parent_id;

            match parent_id {
                Some(gp_id) => {
                    self.arena.replace_child(gp_id, id, only_child);
                }
                None => {
                    self.root = Some(only_child);
                }
            }

            self.arena.get_mut(id).unwrap().children.clear();
            self.arena.remove(id);
            current = parent_id;
        }
    }
}
