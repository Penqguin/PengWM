use crate::tree::{NodeData, NodeId, SplitDirection};

use super::Workspace;

impl Workspace {
    /// Set the split direction for the next window added (when a Window is
    /// focused) or re-orient the focused Split container (when one is focused).
    /// The invariant — "a split direction only applies to a Split container,
    /// otherwise it's pending for the next window" — lives here with the tree.
    pub fn apply_split_direction(&mut self, direction: SplitDirection) {
        if self.focused_is_window() {
            self.pending_split = Some(direction);
        } else {
            self.set_split_direction(direction);
        }
    }

    /// Direction of the focused split container (the focused node itself, or
    /// its nearest split ancestor). `None` when there is no focused split.
    pub fn focused_split_direction(&self) -> Option<SplitDirection> {
        let mut current = self.focused_node?;
        loop {
            let node = self.arena.get(current)?;
            match &node.data {
                NodeData::Split { direction, .. } => return Some(*direction),
                NodeData::Window { .. } => current = node.parent?,
            }
        }
    }

    /// True if the focused node is a Window leaf (not a Split container).
    fn focused_is_window(&self) -> bool {
        self.focused_node.is_some_and(|nid| {
            self.arena
                .get(nid)
                .is_some_and(|n| matches!(n.data, NodeData::Window { .. }))
        })
    }

    /// Change the direction of the focused Split and flatten if redundant.
    fn set_split_direction(&mut self, direction: SplitDirection) {
        if let Some(node_id) = self.focused_node {
            if let NodeData::Split {
                direction: ref mut dir,
                ..
            } = &mut self.arena.get_mut(node_id).unwrap().data
            {
                *dir = direction;
                self.flatten_split_if_redundant(node_id);
            }
        }
    }

    /// If `split_id` is a Split whose direction matches its parent's, absorb its
    /// children into the parent and remove the now-redundant split.
    fn flatten_split_if_redundant(&mut self, split_id: NodeId) {
        let parent_id = match self.arena.get(split_id).and_then(|n| n.parent) {
            Some(pid) => pid,
            None => return,
        };

        let same_dir = match (
            &self.arena.get(split_id).unwrap().data,
            &self.arena.get(parent_id).unwrap().data,
        ) {
            (NodeData::Split { direction: d1, .. }, NodeData::Split { direction: d2, .. }) => {
                d1 == d2
            }
            _ => return,
        };

        if !same_dir {
            return;
        }

        let children: Vec<NodeId> = self.arena.get(split_id).unwrap().children.clone();
        let absorbed = children.len();
        let slot_pos = self
            .arena
            .get(parent_id)
            .and_then(|p| p.children.iter().position(|&c| c == split_id));
        for &child in &children {
            self.arena.get_mut(child).unwrap().parent = Some(parent_id);
        }
        let parent = self.arena.get_mut(parent_id).unwrap();
        if let Some(pos) = parent.children.iter().position(|&c| c == split_id) {
            parent.children.splice(pos..=pos, children);
        }
        self.arena.get_mut(split_id).unwrap().children.clear();
        self.arena.remove(split_id);
        // The absorbed slot's share is split equally among the absorbed
        // children; every other share is untouched (no re-equalize).
        if let Some(pos) = slot_pos {
            self.split_ratio_slot(parent_id, pos, absorbed);
        }
    }

    pub(super) fn next_direction(&self) -> SplitDirection {
        let default = if self.is_widescreen() {
            SplitDirection::Vertical
        } else {
            SplitDirection::Horizontal
        };
        let focused = match self.focused_node {
            Some(id) => id,
            None => return default,
        };
        match self
            .arena
            .get(focused)
            .and_then(|n| n.parent)
            .and_then(|pid| self.arena.get(pid))
            .map(|p| &p.data)
        {
            Some(NodeData::Split { direction, .. }) => match direction {
                SplitDirection::Horizontal => SplitDirection::Vertical,
                SplitDirection::Vertical => SplitDirection::Horizontal,
            },
            _ => default,
        }
    }

    fn is_widescreen(&self) -> bool {
        self.monitor_size.0 > self.monitor_size.1
    }
}
