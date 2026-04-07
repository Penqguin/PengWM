//! Binary Space Partitioning (BSP) tree for window layout management.
//! This module handles the core logic of splitting and organizing window rectangles.

use serde::{Serialize, Deserialize};
use crate::core::types::WindowId;
use crate::core::geometry::Rect;
use indextree::NodeId;

/// Represents the axis along which a node is split.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum SplitAxis {
    /// Split left and right.
    Horizontal,
    /// Split top and bottom.
    Vertical,
}

/// Data stored in each node of the BSP tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NodeData {
    /// A branch node that splits its area into two children.
    Split {
        /// The axis of the split.
        axis: SplitAxis,
        /// The ratio of the split (0.0 to 1.0).
        ratio: f32,
    },
    /// A leaf node that contains one or more windows.
    Leaf {
        /// The window currently visible in this tile.
        visible_window: Option<WindowId>,
        /// Background windows stacked behind the visible one.
        stack: Vec<WindowId>,
    },
}

/// The BSP tree structure using an arena-based tree representation.
pub struct BspTree {
    /// The arena where all nodes are stored.
    pub arena: indextree::Arena<NodeData>,
    /// The root node of the tree.
    pub root: Option<NodeId>,
}

impl BspTree {
    /// Creates a new, empty BSP tree.
    pub fn new() -> Self {
        Self {
            arena: indextree::Arena::new(),
            root: None,
        }
    }

    /// Checks if a window is already managed by this tree.
    pub fn contains_window(&self, window: WindowId) -> bool {
        if let Some(root) = self.root {
            root.descendants(&self.arena).any(|id| {
                if let Some(node) = self.arena.get(id) {
                    match node.get() {
                        NodeData::Leaf { visible_window, stack } => {
                            visible_window == &Some(window) || stack.contains(&window)
                        }
                        _ => false,
                    }
                } else {
                    false
                }
            })
        } else {
            false
        }
    }

    /// Count the number of active leaf nodes in the tree.
    pub fn count_leaves(&self) -> usize {
        if let Some(root) = self.root {
            root.descendants(&self.arena)
                .filter(|id| {
                    matches!(self.arena.get(*id).map(|n| n.get()), Some(NodeData::Leaf { .. }))
                })
                .count()
        } else {
            0
        }
    }

    /// Insert a window into the tree at the specified focused node, respecting max_tiles.
    /// If max_tiles is reached, the window is stacked behind the currently visible one.
    pub fn insert_window(
        &mut self,
        window: WindowId,
        focused_node: Option<NodeId>,
        max_tiles: usize,
    ) -> NodeId {
        let current_leaves = self.count_leaves();

        // Scenario: Empty tree - create the root leaf.
        if self.root.is_none() {
            let node = self.arena.new_node(NodeData::Leaf {
                visible_window: Some(window),
                stack: Vec::new(),
            });
            self.root = Some(node);
            return node;
        }

        let target_node = focused_node.unwrap_or(self.root.unwrap());

        // Scenario B: At Limit - Overflow Strategy: Stacking in the current tile.
        if current_leaves >= max_tiles {
            if let Some(node_ref) = self.arena.get_mut(target_node) {
                if let NodeData::Leaf {
                    visible_window,
                    stack,
                } = node_ref.get_mut()
                {
                    if let Some(prev_window) = visible_window.replace(window) {
                        stack.push(prev_window);
                    }
                    return target_node;
                }
            }
        }

        // Scenario A: Under Limit - Standard BSP split of the current leaf.
        let old_data = {
            let node_ref = self.arena.get_mut(target_node).expect("Target node exists");
            let data = node_ref.get().clone();
            
            // Choose split axis based on the current aspect ratio or toggle (default horizontal).
            // This could be made smarter by taking the node's rectangle as input.
            *node_ref.get_mut() = NodeData::Split {
                axis: SplitAxis::Horizontal,
                ratio: 0.5,
            };
            data
        };

        // Create two new leaves: one with the old content, one with the new window.
        let left_child = self.arena.new_node(old_data);
        let right_child = self.arena.new_node(NodeData::Leaf {
            visible_window: Some(window),
            stack: Vec::new(),
        });

        // Attach children to the previously leaf-now-split node.
        target_node.append(left_child, &mut self.arena);
        target_node.append(right_child, &mut self.arena);

        right_child
    }

    /// Recursively calculate the rectangles for each visible window.
    pub fn calculate_layout(&self, root_rect: Rect, gap_inner: i32, gap_outer: i32) -> Vec<(WindowId, Rect)> {
        let mut layouts = Vec::new();
        if let Some(root) = self.root {
            // Apply outer gaps to the entire workspace.
            let inner_rect = root_rect.inflate(-gap_outer, -gap_outer);
            self.calculate_node_layout(root, inner_rect, gap_inner, &mut layouts);
        }
        layouts
    }

    /// Internal recursive helper for layout calculation.
    fn calculate_node_layout(
        &self,
        node_id: NodeId,
        rect: Rect,
        gap_inner: i32,
        layouts: &mut Vec<(WindowId, Rect)>,
    ) {
        let node = self.arena.get(node_id).expect("Node exists").get();
        match node {
            NodeData::Leaf { visible_window: Some(win), .. } => {
                layouts.push((*win, rect));
            }
            NodeData::Split { axis, ratio } => {
                let children: Vec<NodeId> = node_id.children(&self.arena).collect();
                if children.len() == 2 {
                    let (left_rect, right_rect) = self.split_rect(rect, *axis, *ratio, gap_inner);
                    self.calculate_node_layout(children[0], left_rect, gap_inner, layouts);
                    self.calculate_node_layout(children[1], right_rect, gap_inner, layouts);
                }
            }
            _ => {}
        }
    }

    /// Splits a rectangle into two based on axis, ratio, and inner gap.
    fn split_rect(&self, rect: Rect, axis: SplitAxis, ratio: f32, gap: i32) -> (Rect, Rect) {
        match axis {
            SplitAxis::Horizontal => {
                let left_width = (rect.width() as f32 * ratio) as i32 - (gap / 2);
                let right_width = rect.width() - left_width - gap;
                
                let left = Rect::new(rect.origin, euclid::default::Size2D::new(left_width, rect.height()));
                let right = Rect::new(
                    euclid::default::Point2D::new(rect.min_x() + left_width + gap, rect.min_y()),
                    euclid::default::Size2D::new(right_width, rect.height())
                );
                (left, right)
            }
            SplitAxis::Vertical => {
                let top_height = (rect.height() as f32 * ratio) as i32 - (gap / 2);
                let bottom_height = rect.height() - top_height - gap;
                
                let top = Rect::new(rect.origin, euclid::default::Size2D::new(rect.width(), top_height));
                let bottom = Rect::new(
                    euclid::default::Point2D::new(rect.min_x(), rect.min_y() + top_height + gap),
                    euclid::default::Size2D::new(rect.width(), bottom_height)
                );
                (top, bottom)
            }
        }
    }
}
