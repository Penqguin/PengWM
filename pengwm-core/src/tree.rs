//! Arena-based tree data structures for the BSP layout.
//!
//! Every Node is stored in a flat HashMap identified by a u64 NodeId.
//! Parent/child relationships use IDs, not pointers — no Rc<RefCell> needed.
//!
//! A Node is either a leaf (holds one window) or a split (divides space among children).

use serde::{Serialize, Deserialize};

// ---------------------------------------------------------------------------
// Type aliases
// ---------------------------------------------------------------------------

/// Unique identifier for a node in the arena tree.
pub type NodeId = u64;

/// Unique identifier for a macOS window (CGWindowID).
pub type WindowId = u64;

// ---------------------------------------------------------------------------
// Core enums
// ---------------------------------------------------------------------------

/// The axis along which a parent node splits its children.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum SplitDirection {
    Horizontal,
    Vertical,
}

/// The data payload stored in each node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NodeData {
    Window {
        window_id: WindowId,
        /// Whether this window currently has keyboard focus.
        is_focused: bool,
    },
    Split {
        direction: SplitDirection,
        /// Per-child proportional sizes. Must sum to 1.0.
        ratios: Vec<f32>,
    },
}

// ---------------------------------------------------------------------------
// Node
// ---------------------------------------------------------------------------

/// A single node in the arena tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub id: NodeId,
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    pub data: NodeData,
}

// ---------------------------------------------------------------------------
// Arena
// ---------------------------------------------------------------------------

/// Flat storage for all nodes. Owned by a Workspace.
///
//  Pseudocode:
//  - new()                          -> empty arena, next_id = 0
//  - alloc(data: NodeData)          -> NodeId (increment next_id, insert into map)
//  - get(id: NodeId)                -> Option<&Node>
//  - get_mut(id: NodeId)            -> Option<&mut Node>
//  - remove(id: NodeId)             -> Option<Node> (also orphan its children)
//  - reparent(child, new_parent)    -> detach + attach
//
//  The root node is tracked separately in the Workspace struct.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Arena {
    nodes: std::collections::HashMap<NodeId, Node>,
    next_id: NodeId,
}

impl Arena {
    pub fn new() -> Self {
        todo!("create empty arena")
    }

    /// Allocate a new node with the given data and return its assigned NodeId.
    pub fn alloc(&mut self, data: NodeData) -> NodeId {
        todo!("increment next_id, insert Node, return id")
    }

    pub fn get(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(&id)
    }

    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.get_mut(&id)
    }

    /// Remove a node and all its descendants from the arena.
    /// Returns the removed subtree root data.
    pub fn remove(&mut self, id: NodeId) -> Option<Node> {
        todo!("recursively collect descendant ids, remove all from map")
    }

    /// Detach `child` from its current parent and attach to `new_parent`.
    pub fn reparent(&mut self, child: NodeId, new_parent: NodeId) {
        todo!("remove child from old parent.children, push to new_parent.children, set child.parent")
    }

    /// Number of live nodes in the arena.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }
}
