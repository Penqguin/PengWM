use serde::{Deserialize, Serialize};

pub type NodeId = u64;

pub type WindowId = u64;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

impl Direction {
    pub fn axis(&self) -> SplitDirection {
        match self {
            Direction::Left | Direction::Right => SplitDirection::Vertical,
            Direction::Up | Direction::Down => SplitDirection::Horizontal,
        }
    }

    pub fn is_forward(&self) -> bool {
        matches!(self, Direction::Right | Direction::Down)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum SplitDirection {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NodeData {
    Window {
        window_id: WindowId,
        is_focused: bool,
    },
    Split {
        direction: SplitDirection,
        ratios: Vec<f64>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub id: NodeId,
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    pub data: NodeData,
}

impl Node {
    /// The split's shares, when this node is a split.
    pub fn ratios(&self) -> Option<&[f64]> {
        match &self.data {
            NodeData::Split { ratios, .. } => Some(ratios),
            NodeData::Window { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Arena {
    pub(crate) nodes: std::collections::HashMap<NodeId, Node>,
    next_id: NodeId,
}

impl Default for Arena {
    fn default() -> Self {
        Self::new()
    }
}

impl Arena {
    /// Equal shares for `n` children — the ratio length invariant lives
    /// beside the node type it guards.
    pub(crate) fn equal_shares(n: usize) -> Vec<f64> {
        vec![1.0 / n as f64; n]
    }

    pub fn new() -> Self {
        Self {
            nodes: std::collections::HashMap::new(),
            next_id: 1,
        }
    }

    pub fn alloc(&mut self, data: NodeData) -> NodeId {
        let id = self.next_id;
        self.next_id += 1;
        let node = Node {
            id,
            parent: None,
            children: Vec::new(),
            data,
        };
        self.nodes.insert(id, node);
        id
    }

    pub fn get(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(&id)
    }

    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.get_mut(&id)
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn descendants(&self, id: NodeId) -> Vec<NodeId> {
        let mut result = Vec::new();
        let mut stack = vec![id];
        while let Some(current) = stack.pop() {
            if let Some(node) = self.nodes.get(&current) {
                for &child in &node.children {
                    result.push(child);
                    stack.push(child);
                }
            }
        }
        result
    }

    pub fn is_leaf(&self, id: NodeId) -> bool {
        self.nodes
            .get(&id)
            .is_some_and(|n| matches!(n.data, NodeData::Window { .. }))
    }

    pub fn remove(&mut self, id: NodeId) -> Option<Node> {
        let descendants = self.descendants(id);
        if let Some(node) = self.nodes.get(&id) {
            if let Some(parent_id) = node.parent {
                if let Some(parent) = self.nodes.get_mut(&parent_id) {
                    parent.children.retain(|&c| c != id);
                }
            }
        }
        let root_node = self.nodes.remove(&id);
        for desc_id in descendants {
            self.nodes.remove(&desc_id);
        }
        root_node
    }

    pub fn reparent(&mut self, child: NodeId, new_parent: NodeId) {
        if let Some(node) = self.nodes.get(&child) {
            if let Some(old_parent_id) = node.parent {
                if let Some(old_parent) = self.nodes.get_mut(&old_parent_id) {
                    old_parent.children.retain(|&c| c != child);
                }
            }
        }
        if let Some(child_node) = self.nodes.get_mut(&child) {
            child_node.parent = Some(new_parent);
        }
        if let Some(parent_node) = self.nodes.get_mut(&new_parent) {
            parent_node.children.push(child);
        }
    }

    // -- child-list mutation ---------------------------------------------------
    // The interface for building trees. Children lists and split ratios are
    // wired together here, by construction — call sites splicing `children`
    // by hand and remembering `insert_balanced`/`rebalance_ratios` afterwards
    // were the reason the ratio invariant drifted, whatever the drift rescue
    // branch pretended.

    /// Allocate a split with equal shares for `children`, wiring every
    /// child's parent and the split's own parent. The one constructor for
    /// split nodes — `ratios.len() == children.len()` holds by construction.
    pub fn alloc_split(
        &mut self,
        direction: SplitDirection,
        parent: Option<NodeId>,
        children: &[NodeId],
    ) -> NodeId {
        let id = self.alloc(NodeData::Split {
            direction,
            ratios: Self::equal_shares(children.len()),
        });
        for &child in children {
            self.nodes.get_mut(&child).unwrap().parent = Some(id);
        }
        self.set_children(id, children);
        if let Some(pid) = parent {
            self.nodes.get_mut(&id).unwrap().parent = Some(pid);
        }
        id
    }

    /// Point `parent`'s children at `children` (each child's parent field
    /// follows). Pure wiring — split shares are *not* touched; callers
    /// pick their policy (`equalize` for re-equalize, slot splices for
    /// preserve).
    pub fn set_children(&mut self, parent: NodeId, children: &[NodeId]) {
        for &child in children {
            self.nodes.get_mut(&child).unwrap().parent = Some(parent);
        }
        self.nodes
            .get_mut(&parent)
            .expect("set_children: parent exists")
            .children = children.to_vec();
    }

    /// Wire `children` as children of `parent` and splice them into the
    /// list at `pos` (replacing whatever child held `pos` — the caller has
    /// already taken it). Share policy stays with the caller: an absorbed
    /// slot's share dividing into the newcomers is the splice caller's
    /// business, not this container's.
    pub fn insert_children(&mut self, parent: NodeId, pos: usize, children: &[NodeId]) {
        for &child in children {
            self.nodes.get_mut(&child).unwrap().parent = Some(parent);
        }
        let node = self
            .nodes
            .get_mut(&parent)
            .expect("insert_children: parent exists");
        node.children.splice(pos..=pos, children.to_vec());
    }

    /// Re-equalize a split's shares across its children. The policy half
    /// of `set_children` — the share-preserving operations (removals, slot
    /// splices) never call it. Drift is repaired here too: the vector is
    /// rebuilt, not patched, so a drifted length costs the design, and no
    /// caller needs a rescue branch.
    pub fn equalize(&mut self, split_id: NodeId) {
        let n = self
            .nodes
            .get(&split_id)
            .map(|n| n.children.len())
            .unwrap_or(0);
        if n == 0 {
            return;
        }
        if let Some(node) = self.nodes.get_mut(&split_id) {
            if let NodeData::Split { ref mut ratios, .. } = &mut node.data {
                *ratios = Self::equal_shares(n);
            }
        }
    }

    /// Append `child` under `parent` and, for a split parent, make room:
    /// the survivors' shares scale down by one slot and the newcomer takes
    /// an equal slice (preserves relative sizes — no re-equalize). A drifted
    /// ratio vector is resorted to equal shares here, at the container.
    pub fn attach(&mut self, parent: NodeId, child: NodeId) {
        self.nodes.get_mut(&child).unwrap().parent = Some(parent);
        let (is_split, n) = match self.nodes.get(&parent) {
            Some(node) => (
                matches!(node.data, NodeData::Split { .. }),
                node.children.len() + 1,
            ),
            None => (false, 0),
        };
        self.nodes.get_mut(&parent).unwrap().children.push(child);
        if is_split {
            if let Some(node) = self.nodes.get_mut(&parent) {
                if let NodeData::Split { ref mut ratios, .. } = &mut node.data {
                    if ratios.len() + 1 == n {
                        let scale = (n - 1) as f64 / n as f64;
                        for r in ratios.iter_mut() {
                            *r *= scale;
                        }
                        ratios.push(1.0 / n as f64);
                    } else {
                        // Lengths drifted out of sync — restore at the
                        // container, once, where the invariant belongs.
                        *ratios = Self::equal_shares(n);
                    }
                }
            }
        }
    }

    /// Swap `old` for `new` in `parent`'s children and re-parent `new`.
    /// Ratio counts are unchanged.
    pub fn replace_child(&mut self, parent: NodeId, old: NodeId, new: NodeId) -> bool {
        let Some(slot) = self
            .nodes
            .get(&parent)
            .and_then(|node| node.children.iter().position(|&c| c == old))
        else {
            return false;
        };
        self.nodes.get_mut(&parent).unwrap().children[slot] = new;
        self.nodes.get_mut(&new).unwrap().parent = Some(parent);
        true
    }

    /// Find the node that holds `window_id`, if any. Encapsulates the scan so
    /// `Workspace` doesn't reach into `nodes` directly.
    pub fn find_window(&self, window_id: WindowId) -> Option<NodeId> {
        for (id, node) in &self.nodes {
            if let NodeData::Window { window_id: wid, .. } = &node.data {
                if *wid == window_id {
                    return Some(*id);
                }
            }
        }
        None
    }

    /// All window ids currently in the arena.
    pub fn all_windows(&self) -> Vec<WindowId> {
        self.nodes
            .values()
            .filter_map(|n| {
                if let NodeData::Window { window_id, .. } = &n.data {
                    Some(*window_id)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Iterate over window nodes (window_id, node_id) pairs.
    pub fn windows_iter(&self) -> impl Iterator<Item = (WindowId, NodeId)> + '_ {
        self.nodes.iter().filter_map(|(id, node)| {
            if let NodeData::Window { window_id, .. } = &node.data {
                Some((*window_id, *id))
            } else {
                None
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arena_alloc_and_get() {
        let mut arena = Arena::new();
        let id = arena.alloc(NodeData::Window {
            window_id: 42,
            is_focused: true,
        });
        assert_eq!(arena.len(), 1);
        let node = arena.get(id).unwrap();
        assert_eq!(node.id, id);
        assert!(node.parent.is_none());
        assert!(node.children.is_empty());
    }

    #[test]
    fn arena_remove_leaf_updates_parent() {
        let mut arena = Arena::new();
        let parent = arena.alloc(NodeData::Split {
            direction: SplitDirection::Vertical,
            ratios: vec![0.5, 0.5],
        });
        let child_a = arena.alloc(NodeData::Window {
            window_id: 1,
            is_focused: false,
        });
        let child_b = arena.alloc(NodeData::Window {
            window_id: 2,
            is_focused: false,
        });
        arena.reparent(child_a, parent);
        arena.reparent(child_b, parent);

        arena.remove(child_a);
        assert_eq!(arena.len(), 2);
        let p = arena.get(parent).unwrap();
        assert_eq!(p.children, vec![child_b]);
    }

    #[test]
    fn arena_remove_subtree_removes_all() {
        let mut arena = Arena::new();
        let a = arena.alloc(NodeData::Split {
            direction: SplitDirection::Vertical,
            ratios: vec![0.5, 0.5],
        });
        let b = arena.alloc(NodeData::Window {
            window_id: 1,
            is_focused: false,
        });
        let c = arena.alloc(NodeData::Window {
            window_id: 2,
            is_focused: false,
        });
        arena.reparent(b, a);
        arena.reparent(c, a);

        arena.remove(a);
        assert_eq!(arena.len(), 0);
    }

    #[test]
    fn arena_reparent_moves_child() {
        let mut arena = Arena::new();
        let p1 = arena.alloc(NodeData::Split {
            direction: SplitDirection::Vertical,
            ratios: vec![0.5, 0.5],
        });
        let p2 = arena.alloc(NodeData::Split {
            direction: SplitDirection::Horizontal,
            ratios: vec![0.5, 0.5],
        });
        let child = arena.alloc(NodeData::Window {
            window_id: 1,
            is_focused: false,
        });
        arena.reparent(child, p1);
        assert_eq!(arena.get(p1).unwrap().children, vec![child]);
        assert_eq!(arena.get(child).unwrap().parent, Some(p1));

        arena.reparent(child, p2);
        assert!(arena.get(p1).unwrap().children.is_empty());
        assert_eq!(arena.get(p2).unwrap().children, vec![child]);
        assert_eq!(arena.get(child).unwrap().parent, Some(p2));
    }

    #[test]
    fn arena_descendants_nested() {
        let mut arena = Arena::new();
        let root = arena.alloc(NodeData::Split {
            direction: SplitDirection::Vertical,
            ratios: vec![0.5, 0.5],
        });
        let inner = arena.alloc(NodeData::Split {
            direction: SplitDirection::Horizontal,
            ratios: vec![0.5, 0.5],
        });
        let leaf_a = arena.alloc(NodeData::Window {
            window_id: 1,
            is_focused: false,
        });
        let leaf_b = arena.alloc(NodeData::Window {
            window_id: 2,
            is_focused: false,
        });
        arena.reparent(inner, root);
        arena.reparent(leaf_a, inner);
        arena.reparent(leaf_b, inner);

        let desc = arena.descendants(root);
        assert_eq!(desc.len(), 3);
        assert!(desc.contains(&inner));
        assert!(desc.contains(&leaf_a));
        assert!(desc.contains(&leaf_b));
    }

    #[test]
    fn arena_is_leaf() {
        let mut arena = Arena::new();
        let leaf = arena.alloc(NodeData::Window {
            window_id: 1,
            is_focused: false,
        });
        let split = arena.alloc(NodeData::Split {
            direction: SplitDirection::Vertical,
            ratios: vec![0.5, 0.5],
        });
        assert!(arena.is_leaf(leaf));
        assert!(!arena.is_leaf(split));
    }

    #[test]
    fn alloc_split_wires_parents_and_equal_shares() {
        let mut arena = Arena::new();
        let a = arena.alloc(NodeData::Window {
            window_id: 1,
            is_focused: false,
        });
        let b = arena.alloc(NodeData::Window {
            window_id: 2,
            is_focused: false,
        });
        let s = arena.alloc_split(SplitDirection::Vertical, None, &[a, b]);
        assert_eq!(arena.get(s).unwrap().children, vec![a, b]);
        assert!(arena.get(s).unwrap().parent.is_none());
        assert_eq!(arena.get(a).unwrap().parent, Some(s));
        assert_eq!(
            arena.get(s).unwrap().ratios(),
            Some(&[0.5, 0.5][..]),
            "shares match the child count by construction"
        );
    }

    #[test]
    fn attach_scales_shares_and_reparents() {
        let mut arena = Arena::new();
        let a = arena.alloc(NodeData::Window {
            window_id: 1,
            is_focused: false,
        });
        let s = arena.alloc_split(SplitDirection::Vertical, None, &[a]);
        let c = arena.alloc(NodeData::Window {
            window_id: 9,
            is_focused: false,
        });
        arena.attach(s, c);
        assert_eq!(arena.get(c).unwrap().parent, Some(s));
        // Two equal children after the attach: survivors scaled, newcomer equal.
        assert_eq!(arena.get(s).unwrap().ratios(), Some(&[0.5, 0.5][..]));
    }

    #[test]
    fn replace_child_swaps_slot_and_reparents() {
        let mut arena = Arena::new();
        let a = arena.alloc(NodeData::Window {
            window_id: 1,
            is_focused: false,
        });
        let b = arena.alloc(NodeData::Window {
            window_id: 2,
            is_focused: false,
        });
        let s = arena.alloc_split(SplitDirection::Vertical, None, &[a]);
        assert!(arena.replace_child(s, a, b));
        assert_eq!(arena.get(s).unwrap().children, vec![b]);
        assert_eq!(arena.get(b).unwrap().parent, Some(s));
        // The old child's parent is the caller's business — in add_window
        // the replaced child was just re-parented to its new split, and
        // clearing here would destroy that wiring.
        assert_eq!(arena.get(a).unwrap().parent, Some(s));
        assert!(
            !arena.replace_child(s, a, b),
            "missing old child is a no-op"
        );
    }

    #[test]
    fn set_children_wires_parents_without_touching_shares() {
        let mut arena = Arena::new();
        let a = arena.alloc(NodeData::Window {
            window_id: 1,
            is_focused: false,
        });
        let b = arena.alloc(NodeData::Window {
            window_id: 2,
            is_focused: false,
        });
        let s = arena.alloc(NodeData::Split {
            direction: SplitDirection::Vertical,
            ratios: vec![0.75],
        });
        arena.set_children(s, &[a, b]);
        assert_eq!(arena.get(s).unwrap().children, vec![a, b]);
        assert_eq!(
            arena.get(s).unwrap().ratios(),
            Some(&[0.75][..]),
            "wiring is not re-equalizing: share policy stays with the caller"
        );
        arena.equalize(s);
        assert_eq!(arena.get(s).unwrap().ratios(), Some(&[0.5, 0.5][..]));
    }
}
