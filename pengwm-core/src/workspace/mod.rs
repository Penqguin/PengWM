use serde::{Serialize, Deserialize};
use crate::tree::{Arena, Direction, NodeData, NodeId, SplitDirection, WindowId};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    pub name: String,
    pub root: Option<NodeId>,
    pub arena: Arena,
    pub monitor_id: u32,
    pub monitor_origin: (i32, i32),
    pub monitor_size: (u32, u32),
    pub focused_node: Option<NodeId>,
    pub monocle: bool,
    pub pending_split: Option<SplitDirection>,
}

impl Workspace {
    pub fn new(
        name: String,
        monitor_id: u32,
        origin: (i32, i32),
        size: (u32, u32),
    ) -> Self {
        Self {
            name,
            root: None,
            arena: Arena::new(),
            monitor_id,
            monitor_origin: origin,
            monitor_size: size,
            focused_node: None,
            monocle: false,
            pending_split: None,
        }
    }

    // -----------------------------------------------------------------------
    // Window management
    // -----------------------------------------------------------------------

    pub fn add_window(
        &mut self,
        window_id: WindowId,
        direction: Option<SplitDirection>,
    ) -> NodeId {
        if self.root.is_none() {
            let id = self
                .arena
                .alloc(NodeData::Window { window_id, is_focused: true });
            self.root = Some(id);
            self.focused_node = Some(id);
            return id;
        }

        let dir = direction
            .or_else(|| self.pending_split.take())
            .unwrap_or_else(|| self.next_direction());
        let focused = self.focused_node.expect("focused_node set when root exists");

        let flatten = self
            .arena
            .get(focused)
            .and_then(|n| n.parent)
            .and_then(|pid| self.arena.get(pid))
            .is_some_and(|p| {
                matches!(&p.data, NodeData::Split { direction: d, .. } if *d == dir)
            });

        if flatten {
            let parent_id = self.arena.get(focused).and_then(|n| n.parent).unwrap();
            let new_id = self
                .arena
                .alloc(NodeData::Window { window_id, is_focused: true });
            self.arena.get_mut(parent_id).unwrap().children.push(new_id);
            self.arena.get_mut(new_id).unwrap().parent = Some(parent_id);
            self.rebalance_ratios(parent_id);
            self.set_focused_node(new_id);
            return new_id;
        }

        let old_parent = self.arena.get(focused).and_then(|n| n.parent);
        let new_id = self
            .arena
            .alloc(NodeData::Window { window_id, is_focused: true });
        let split_id = self.arena.alloc(NodeData::Split {
            direction: dir,
            ratios: vec![0.5, 0.5],
        });

        self.arena.get_mut(split_id).unwrap().children = vec![focused, new_id];
        self.arena.get_mut(focused).unwrap().parent = Some(split_id);
        self.arena.get_mut(new_id).unwrap().parent = Some(split_id);

        match old_parent {
            Some(pid) => {
                let parent = self.arena.get_mut(pid).unwrap();
                if let Some(pos) = parent.children.iter().position(|&c| c == focused) {
                    parent.children[pos] = split_id;
                }
                self.arena.get_mut(split_id).unwrap().parent = Some(pid);
            }
            None => {
                self.root = Some(split_id);
            }
        }

        self.set_focused_node(new_id);
        new_id
    }

    pub fn remove_window(&mut self, window_id: WindowId) {
        let node_id = match self.find_window(window_id) {
            Some(id) => id,
            None => return,
        };

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
        self.arena.remove(node_id);
        self.collapse_upward(parent_id);

        if self.root.is_none() {
            self.focused_node = None;
        } else if was_focused {
            self.focus_nearest_leaf();
        }
    }

    pub fn focus_window(&mut self, window_id: WindowId) {
        let node_id = match self.find_window(window_id) {
            Some(id) => id,
            None => return,
        };
        self.set_focused_node(node_id);
    }

    pub fn focus_neighbor(&mut self, direction: Direction) {
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

    pub fn swap_window(&mut self, direction: Direction) {
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

    // -----------------------------------------------------------------------
    // Tree queries
    // -----------------------------------------------------------------------

    pub fn find_window(&self, window_id: WindowId) -> Option<NodeId> {
        for (id, node) in &self.arena.nodes {
            if let NodeData::Window {
                window_id: wid, ..
            } = &node.data
            {
                if *wid == window_id {
                    return Some(*id);
                }
            }
        }
        None
    }

    pub fn all_windows(&self) -> Vec<WindowId> {
        let mut result = Vec::new();
        for node in self.arena.nodes.values() {
            if let NodeData::Window {
                window_id: wid, ..
            } = &node.data
            {
                result.push(*wid);
            }
        }
        result
    }

    pub fn window_count(&self) -> usize {
        self.all_windows().len()
    }

    pub fn toggle_monocle(&mut self) {
        self.monocle = !self.monocle;
    }

    /// If `split_id` is a Split whose direction matches its parent's, absorb its
    /// children into the parent and remove the now-redundant split.
    pub fn flatten_split_if_redundant(&mut self, split_id: NodeId) {
        let parent_id = match self.arena.get(split_id).and_then(|n| n.parent) {
            Some(pid) => pid,
            None => return,
        };

        let same_dir = match (&self.arena.get(split_id).unwrap().data, &self.arena.get(parent_id).unwrap().data) {
            (NodeData::Split { direction: d1, .. }, NodeData::Split { direction: d2, .. }) => d1 == d2,
            _ => return,
        };

        if !same_dir {
            return;
        }

        let children: Vec<NodeId> = self.arena.get(split_id).unwrap().children.clone();
        for &child in &children {
            self.arena.get_mut(child).unwrap().parent = Some(parent_id);
        }
        let parent = self.arena.get_mut(parent_id).unwrap();
        if let Some(pos) = parent.children.iter().position(|&c| c == split_id) {
            parent.children.splice(pos..=pos, children);
        }
        self.arena.get_mut(split_id).unwrap().children.clear();
        self.arena.remove(split_id);
        self.rebalance_ratios(parent_id);
    }

    // -----------------------------------------------------------------------
    // Monitor
    // -----------------------------------------------------------------------

    pub fn update_monitor_geometry(&mut self, origin: (i32, i32), size: (u32, u32)) {
        self.monitor_origin = origin;
        self.monitor_size = size;
    }

    pub fn is_on_monitor(&self, display_id: u32) -> bool {
        self.monitor_id == display_id
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    fn next_direction(&self) -> SplitDirection {
        let focused = match self.focused_node {
            Some(id) => id,
            None => {
                if self.monitor_size.0 > self.monitor_size.1 {
                    return SplitDirection::Vertical;
                } else {
                    return SplitDirection::Horizontal;
                }
            }
        };
        match self.arena.get(focused).and_then(|n| n.parent) {
            Some(pid) => match self.arena.get(pid).map(|p| &p.data) {
                Some(NodeData::Split {
                    direction: d, ..
                }) => match d {
                    SplitDirection::Horizontal => SplitDirection::Vertical,
                    SplitDirection::Vertical => SplitDirection::Horizontal,
                },
                _ => {
                    if self.monitor_size.0 > self.monitor_size.1 {
                        SplitDirection::Vertical
                    } else {
                        SplitDirection::Horizontal
                    }
                }
            },
            None => {
                if self.monitor_size.0 > self.monitor_size.1 {
                    SplitDirection::Vertical
                } else {
                    SplitDirection::Horizontal
                }
            }
        }
    }

    fn set_focused_node(&mut self, node_id: NodeId) {
        if let Some(old_id) = self.focused_node {
            if let Some(old) = self.arena.get_mut(old_id) {
                if let NodeData::Window {
                    ref mut is_focused,
                    ..
                } = old.data
                {
                    *is_focused = false;
                }
            }
        }
        self.focused_node = Some(node_id);
        if let Some(node) = self.arena.get_mut(node_id) {
            if let NodeData::Window {
                ref mut is_focused,
                ..
            } = node.data
            {
                *is_focused = true;
            }
        }
    }

    fn focus_nearest_leaf(&mut self) {
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

    fn rebalance_ratios(&mut self, split_id: NodeId) {
        let n = self
            .arena
            .get(split_id)
            .map(|n| n.children.len())
            .unwrap_or(0);
        if n > 0 {
            let equal = 1.0 / n as f32;
            if let NodeData::Split {
                ref mut ratios, ..
            } = &mut self.arena.get_mut(split_id).unwrap().data
            {
                *ratios = vec![equal; n];
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
                self.rebalance_ratios(id);
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
            self.arena.get_mut(only_child).unwrap().parent = parent_id;

            match parent_id {
                Some(gp_id) => {
                    let gp = self.arena.get_mut(gp_id).unwrap();
                    if let Some(pos) = gp.children.iter().position(|&c| c == id) {
                        gp.children[pos] = only_child;
                    }
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

    fn find_neighbor(&self, from_node: NodeId, direction: Direction) -> Option<NodeId> {
        let target_axis = direction.axis();
        let is_forward = direction.is_forward();

        let mut current = from_node;
        let (split_id, branch_id) = loop {
            let node = self.arena.get(current)?;
            match node.parent {
                Some(pid) => {
                    let parent = self.arena.get(pid)?;
                    if let NodeData::Split {
                        direction: d, ..
                    } = &parent.data
                    {
                        if *d == target_axis {
                            break (pid, current);
                        }
                    }
                    current = pid;
                }
                None => return None,
            }
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

#[cfg(test)]
mod tests;
