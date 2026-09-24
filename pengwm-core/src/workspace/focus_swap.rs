use crate::tree::{Direction, NodeData, NodeId, WindowId};

use super::Workspace;

impl Workspace {
    pub fn focus_window(&mut self, window_id: WindowId) {
        let node_id = match self.find_window(window_id) {
            Some(id) => id,
            None => return,
        };
        self.set_focused_node(node_id);
    }

    pub fn focus_neighbor(&mut self, direction: Direction) {
        if self.monocle {
            let leaves = self.cycle_leaves();
            if leaves.is_empty() {
                return;
            }
            let pos = leaves
                .iter()
                .position(|&id| Some(id) == self.focused_node)
                .unwrap_or(0);
            let target_pos = if direction.is_forward() {
                (pos + 1) % leaves.len()
            } else if pos == 0 {
                leaves.len() - 1
            } else {
                pos - 1
            };
            self.set_focused_node(leaves[target_pos]);
            return;
        }
        let from = match self.focused_node {
            Some(id) => id,
            None => return,
        };
        let target = match self.find_neighbor(from, direction) {
            Some(id) => id,
            None => return,
        };
        self.set_focused_node(target);
    }

    pub fn swap_windows_by_id(&mut self, dragged_id: WindowId, target_id: WindowId) -> bool {
        let dragged_node = match self.find_window(dragged_id) {
            Some(id) => id,
            None => return false,
        };
        let target_node = match self.find_window(target_id) {
            Some(id) => id,
            None => return false,
        };
        if dragged_node == target_node {
            return false;
        }
        let dragged_data = self.arena.get(dragged_node).unwrap().data.clone();
        let target_data = self.arena.get(target_node).unwrap().data.clone();
        self.arena.get_mut(dragged_node).unwrap().data = target_data;
        self.arena.get_mut(target_node).unwrap().data = dragged_data;
        self.focused_node = Some(target_node);
        true
    }

    pub fn swap_window(&mut self, direction: Direction) {
        if self.monocle {
            let leaves = self.cycle_leaves();
            if leaves.len() < 2 {
                return;
            }
            let pos = match leaves.iter().position(|&id| Some(id) == self.focused_node) {
                Some(p) => p,
                None => return,
            };
            let target_pos = if direction.is_forward() {
                (pos + 1) % leaves.len()
            } else if pos == 0 {
                leaves.len() - 1
            } else {
                pos - 1
            };
            let focused = leaves[pos];
            let target = leaves[target_pos];
            // In monocle all windows are stacked — swapping tree positions
            // is invisible if we keep the original WindowId visible. Swap
            // only the WindowId so the *neighbor* becomes the visible
            // fullscreen window, and keep is_focused at the original position.
            let focused_wid = match &self.arena.get(focused).unwrap().data {
                NodeData::Window { window_id, .. } => *window_id,
                _ => return,
            };
            let target_wid = match &self.arena.get(target).unwrap().data {
                NodeData::Window { window_id, .. } => *window_id,
                _ => return,
            };
            if let NodeData::Window { window_id, .. } =
                &mut self.arena.get_mut(focused).unwrap().data
            {
                *window_id = target_wid;
            }
            if let NodeData::Window { window_id, .. } =
                &mut self.arena.get_mut(target).unwrap().data
            {
                *window_id = focused_wid;
            }
            // Flags already correct — focused stays focused but now shows the
            // swapped-in window, so the swap is visible in monocle.
            return;
        }
        let focused = match self.focused_node {
            Some(id) => id,
            None => return,
        };
        let target = match self.find_neighbor(focused, direction) {
            Some(id) => id,
            None => return,
        };
        let old_focused_data = self.arena.get(focused).unwrap().data.clone();
        let old_target_data = self.arena.get(target).unwrap().data.clone();
        self.arena.get_mut(focused).unwrap().data = old_target_data;
        self.arena.get_mut(target).unwrap().data = old_focused_data;
        self.focused_node = Some(target);
    }

    pub fn find_window(&self, window_id: WindowId) -> Option<NodeId> {
        self.arena.find_window(window_id)
    }

    pub fn all_windows(&self) -> Vec<WindowId> {
        self.arena.all_windows()
    }

    pub fn window_count(&self) -> usize {
        self.all_windows().len()
    }

    pub fn focused_window_id(&self) -> Option<WindowId> {
        self.focused_node.and_then(|nid| {
            if let NodeData::Window { window_id, .. } = &self.arena.get(nid)?.data {
                Some(*window_id)
            } else {
                None
            }
        })
    }

    pub fn focus_first(&mut self) -> Option<WindowId> {
        let root = self.root?;
        let leaf = self.leftmost_leaf(root);
        self.set_focused_node(leaf);
        self.focused_window_id()
    }

    pub(super) fn set_focused_node(&mut self, node_id: NodeId) {
        if let Some(old_id) = self.focused_node {
            if let Some(old) = self.arena.get_mut(old_id) {
                if let NodeData::Window {
                    ref mut is_focused, ..
                } = old.data
                {
                    *is_focused = false;
                }
            }
        }
        self.focused_node = Some(node_id);
        if let Some(node) = self.arena.get_mut(node_id) {
            if let NodeData::Window {
                ref mut is_focused, ..
            } = node.data
            {
                *is_focused = true;
            }
        }
    }

    pub(super) fn focus_nearest_leaf(&mut self) {
        match self.root {
            Some(root_id) => {
                let leaf = self.leftmost_leaf(root_id);
                self.set_focused_node(leaf);
            }
            None => {
                self.focused_node = None;
            }
        }
    }

    fn cycle_leaves(&self) -> Vec<NodeId> {
        let Some(root) = self.root else {
            return Vec::new();
        };
        let mut out = Vec::new();
        self.collect_leaves(root, &mut out);
        out
    }

    fn collect_leaves(&self, node_id: NodeId, out: &mut Vec<NodeId>) {
        let Some(node) = self.arena.get(node_id) else {
            return;
        };
        match &node.data {
            NodeData::Window { .. } => out.push(node_id),
            NodeData::Split { .. } => {
                for &child in &node.children.clone() {
                    self.collect_leaves(child, out);
                }
            }
        }
    }

    fn find_neighbor(&self, from_node: NodeId, direction: Direction) -> Option<NodeId> {
        let target_axis = direction.axis();
        let is_forward = direction.is_forward();

        let mut current = from_node;
        let (split_id, branch_id) = loop {
            let node = self.arena.get(current)?;
            let pid = node.parent?;
            let parent = self.arena.get(pid)?;
            if let NodeData::Split { direction: d, .. } = &parent.data {
                if *d == target_axis {
                    break (pid, current);
                }
            }
            current = pid;
        };

        let split = self.arena.get(split_id)?;
        let pos = split.children.iter().position(|&c| c == branch_id)?;
        let n_children = split.children.len();

        let target_pos = if is_forward {
            (pos + 1) % n_children
        } else if pos == 0 {
            n_children - 1
        } else {
            pos - 1
        };

        let target_branch = split.children[target_pos];
        Some(if is_forward {
            self.leftmost_leaf(target_branch)
        } else {
            self.rightmost_leaf(target_branch)
        })
    }

    fn leftmost_leaf(&self, node_id: NodeId) -> NodeId {
        let node = self.arena.get(node_id).unwrap();
        match &node.data {
            NodeData::Window { .. } => node_id,
            NodeData::Split { .. } => {
                if node.children.is_empty() {
                    node_id
                } else {
                    self.leftmost_leaf(node.children[0])
                }
            }
        }
    }

    fn rightmost_leaf(&self, node_id: NodeId) -> NodeId {
        let node = self.arena.get(node_id).unwrap();
        match &node.data {
            NodeData::Window { .. } => node_id,
            NodeData::Split { .. } => {
                if node.children.is_empty() {
                    node_id
                } else {
                    self.rightmost_leaf(node.children[node.children.len() - 1])
                }
            }
        }
    }
}
