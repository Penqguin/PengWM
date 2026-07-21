use serde::{Serialize, Deserialize};

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
        ratios: Vec<f32>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub id: NodeId,
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    pub data: NodeData,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Arena {
    pub nodes: std::collections::HashMap<NodeId, Node>,
    next_id: NodeId,
}

impl Arena {
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
            .map_or(false, |n| matches!(n.data, NodeData::Window { .. }))
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
}
