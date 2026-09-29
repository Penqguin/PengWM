use crate::tree::{Arena, NodeData, NodeId, SplitDirection, WindowId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Rect {
            x,
            y,
            width,
            height,
        }
    }
}

/// Tolerance for "already at target" comparisons: matches the 8px the
/// daemon's misplaced sweep and drag settle use. Anything within this is
/// "at target" everywhere in the system. Single copy — the daemon's writer,
/// layout-write cache and reconcile all import this instead of defining
/// their own.
pub const LAYOUT_EPSILON: f64 = 8.0;

/// Pure epsilon comparison for verify-and-retry. No FFI so unit-testable
/// on any platform.
pub fn rects_close(a: Rect, b: Rect, eps: f64) -> bool {
    (a.x - b.x).abs() <= eps
        && (a.y - b.y).abs() <= eps
        && (a.width - b.width).abs() <= eps
        && (a.height - b.height).abs() <= eps
}

/// Grace before a displaced rect reads as external rather than animation
/// settling. Shared by the moved-note and the misplaced sweep — one value
/// everywhere in the system.
pub const DISPLACE_GRACE: Duration = Duration::from_millis(500);

/// Full-rect displacement check: true when `actual` disagrees with `target`
/// anywhere beyond `LAYOUT_EPSILON`. The one predicate behind both the
/// moved-note and the misplaced sweep.
pub fn rects_displaced(a: Rect, b: Rect) -> bool {
    !rects_close(a, b, LAYOUT_EPSILON)
}

pub fn calculate_layout(
    node_id: NodeId,
    bounding: Rect,
    arena: &Arena,
    output: &mut HashMap<WindowId, Rect>,
    gap_size: f64,
) {
    let node = match arena.get(node_id) {
        Some(n) => n,
        None => return,
    };

    match &node.data {
        NodeData::Window { window_id, .. } => {
            output.insert(*window_id, bounding);
        }
        NodeData::Split { direction, ratios } => {
            debug_assert_eq!(
                node.children.len(),
                ratios.len(),
                "split ratios len {} != children len {}",
                ratios.len(),
                node.children.len()
            );
            let child_rects = split_n(bounding, ratios, gap_size, *direction);
            for (&child_id, rect) in node.children.iter().zip(child_rects.iter()) {
                calculate_layout(child_id, *rect, arena, output, gap_size);
            }
        }
    }
}

pub(crate) fn screen_local_to_global(local: Rect, monitor_origin: (i32, i32)) -> Rect {
    Rect {
        x: local.x + monitor_origin.0 as f64,
        y: local.y + monitor_origin.1 as f64,
        width: local.width,
        height: local.height,
    }
}

pub(crate) fn inset_rect(rect: Rect, gap: f64) -> Rect {
    let double = gap * 2.0;
    Rect {
        x: rect.x + gap,
        y: rect.y + gap,
        width: (rect.width - double).max(0.0),
        height: (rect.height - double).max(0.0),
    }
}

fn split_n(bounding: Rect, ratios: &[f64], gap_size: f64, direction: SplitDirection) -> Vec<Rect> {
    let n = ratios.len();
    if n == 0 {
        return vec![];
    }
    if n == 1 {
        return vec![bounding];
    }

    let total_gap = (n - 1) as f64 * gap_size;
    let ratio_sum: f64 = ratios.iter().copied().sum();
    let ratio_sum = if ratio_sum == 0.0 { 1.0 } else { ratio_sum };

    let mut rects = Vec::with_capacity(n);
    match direction {
        SplitDirection::Horizontal => {
            let available = (bounding.height - total_gap).max(0.0);
            let mut offset = bounding.y;
            for (i, &ratio) in ratios.iter().enumerate() {
                let size = if i == n - 1 {
                    (bounding.y + bounding.height - offset).max(0.0)
                } else {
                    (available * ratio / ratio_sum).max(0.0)
                };
                rects.push(Rect::new(bounding.x, offset, bounding.width, size));
                offset += size + gap_size;
            }
        }
        SplitDirection::Vertical => {
            let available = (bounding.width - total_gap).max(0.0);
            let mut offset = bounding.x;
            for (i, &ratio) in ratios.iter().enumerate() {
                let size = if i == n - 1 {
                    (bounding.x + bounding.width - offset).max(0.0)
                } else {
                    (available * ratio / ratio_sum).max(0.0)
                };
                rects.push(Rect::new(offset, bounding.y, size, bounding.height));
                offset += size + gap_size;
            }
        }
    }
    rects
}
