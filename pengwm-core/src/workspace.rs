//! A Workspace represents one virtual desktop bound to a single monitor.
//!
//! Each workspace owns its own Arena tree. The daemon may have N workspaces
//! mapped across M monitors (e.g. workspace 1-3 on display A, 4-6 on display B).

use serde::{Serialize, Deserialize};
use crate::tree::{self, Arena, NodeId, NodeData, WindowId};

// ---------------------------------------------------------------------------
// Workspace
// ---------------------------------------------------------------------------

/// A virtual desktop with an arena tree, bound to a physical monitor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    pub name: String,
    //  root: Option<NodeId>,
    //  Arena (the tree),

    //  CGDirectDisplayID
    pub monitor_id: u32,

    //  In global coordinate space
    pub monitor_origin: (i32, i32),
    //  In points
    pub monitor_size: (u32, u32),

    // Tracks which node currently has keyboard focus
    pub focused_node: Option<NodeId>,
}

impl Workspace {
    pub fn new(name: String, monitor_id: u32, origin: (i32, i32), size: (u32, u32)) -> Self {
        todo!("create workspace with empty arena and no root")
    }

    // -----------------------------------------------------------------------
    // Window management
    // -----------------------------------------------------------------------

    /// Insert a window into the tree. If the tree is empty, this window becomes
    /// the root leaf. Otherwise, split the focused node and place the new window
    /// as a sibling.
    pub fn add_window(&mut self, window_id: WindowId) -> NodeId {
        //  if no root → alloc a Window leaf, set as root, return
        //  if focused node exists → replace it with a Split, create two children:
        //    - one with the old leaf data
        //    - one with the new window as a leaf
        //  return the new window's NodeId
        todo!()
    }

    /// Remove a window from the tree. If its parent split now has only one child,
    /// collapse the split and promote the remaining child.
    pub fn remove_window(&mut self, window_id: WindowId) {
        //  find the leaf node containing window_id
        //  remove it from the arena
        //  if the parent split now has 0 children → remove parent, set root = None
        //  if the parent split now has 1 child  → collapse: replace parent with the child
        todo!()
    }

    /// Mark a window as focused and un-focus the previously focused window.
    pub fn focus_window(&mut self, window_id: WindowId) {
        //  update is_focused on the old focused_node and the new one
        todo!()
    }

    /// Move focus in a cardinal direction by navigating the tree.
    pub fn focus_neighbor(&mut self, direction: tree::SplitDirection) {
        //  starting from focused_node, walk the tree up to find the nearest split
        //  of the opposite orientation, then descend into the adjacent branch
        todo!()
    }

    /// Swap the focused window with a neighbor in the given direction.
    pub fn swap_window(&mut self, direction: tree::SplitDirection) {
        //  similar traversal to focus_neighbor, but swap the NodeData between leaves
        todo!()
    }

    // -----------------------------------------------------------------------
    // Tree queries
    // -----------------------------------------------------------------------

    /// Walk the arena and return the NodeId of the leaf containing `window_id`.
    pub fn find_window(&self, window_id: WindowId) -> Option<NodeId> {
        todo!("iterate arena nodes looking for NodeData::Window with matching window_id")
    }

    /// Return all window IDs in this workspace.
    pub fn all_windows(&self) -> Vec<WindowId> {
        todo!("collect every window_id from every Window leaf in the arena")
    }

    /// Total number of window leaves currently in the tree.
    pub fn window_count(&self) -> usize {
        self.all_windows().len()
    }

    // -----------------------------------------------------------------------
    // Monitor
    // -----------------------------------------------------------------------

    /// Update monitor geometry (e.g. after a resolution change).
    pub fn update_monitor_geometry(&mut self, origin: (i32, i32), size: (u32, u32)) {
        self.monitor_origin = origin;
        self.monitor_size = size;
    }

    /// Check whether this workspace belongs to the given display.
    pub fn is_on_monitor(&self, display_id: u32) -> bool {
        self.monitor_id == display_id
    }
}
