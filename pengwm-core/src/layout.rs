//! Pure layout engine — no FFI, no macOS types.
//!
//! All math uses local coordinate space (origin at 0,0).
//! The daemon applies monitor offsets just before calling AXUIElement.

use serde::{Serialize, Deserialize};
use crate::tree::{Arena, NodeData, NodeId, WindowId};

// ---------------------------------------------------------------------------
// Rect
// ---------------------------------------------------------------------------

/// A 2D axis-aligned rectangle in local screen coordinates.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Rect { x, y, width, height }
    }
}

// ---------------------------------------------------------------------------
// Layout algorithm
// ---------------------------------------------------------------------------

/// Recursively compute the position and size of every window leaf in the tree.
///
/// # Arguments
/// * `node_id`    — starting node (pass the workspace root)
/// * `bounding`   — the rectangle this node is constrained to
/// * `arena`      — the workspace's node storage
/// * `output`     — accumulator: maps WindowId → final Rect
/// * `gap_size`   — inner + outer gap in points
///
//  Pseudocode:
//  - Look up node in arena.
//  - If NodeData::Window:
//       inset bounding by gap_size on all sides
//       insert (window_id → inset_rect) into output
//  - If NodeData::Split:
//       for each child, in order:
//           compute child's proportional sub-rectangle based on direction + ratios
//           subtract half the gap between adjacent children
//           recurse with calculate_layout(child_id, sub_rect, ...)
pub fn calculate_layout(
    node_id: NodeId,
    bounding: Rect,
    arena: &Arena,
    output: &mut std::collections::HashMap<WindowId, Rect>,
    gap_size: f64,
) {
    todo!("recursive layout math")
}

/// Convert a local-coordinate rect to global by adding the monitor's origin offset.
///
/// This is the last transform before calling AXUIElementSetAttributeValue.
pub fn screen_local_to_global(
    local: Rect,
    monitor_origin: (i32, i32),
) -> Rect {
    todo!("add monitor_origin.0 to x, monitor_origin.1 to y")
}

// ---------------------------------------------------------------------------
// Helpers (optional, used internally by calculate_layout)
// ---------------------------------------------------------------------------

/// Split a rectangle horizontally, returning (left_rect, right_rect).
fn split_horizontal(bounding: Rect, ratio: f32, gap: f64) -> (Rect, Rect) {
    todo!("compute left and right child rects with gap between them")
}

/// Split a rectangle vertically, returning (top_rect, bottom_rect).
fn split_vertical(bounding: Rect, ratio: f32, gap: f64) -> (Rect, Rect) {
    todo!("compute top and bottom child rects with gap between them")
}

#[cfg(test)]
mod tests {
    // TODO: test calculate_layout with:
    //   - single root leaf
    //   - one horizontal split with two windows
    //   - nested horizontal + vertical splits
    //   - gap_size = 0
    //   - non-uniform ratios
}
