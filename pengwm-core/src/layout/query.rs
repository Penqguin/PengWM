use super::rect::Rect;
use crate::tree::WindowId;
use std::collections::HashMap;

/// The window whose rect contains the point (`x`, `y`), ignoring `exclude`.
/// `None` when no other window contains the point. Drives drag-to-swap
/// overlap detection over a layout output.
pub fn window_at_point(
    rects: &HashMap<WindowId, Rect>,
    x: f64,
    y: f64,
    exclude: WindowId,
) -> Option<WindowId> {
    rects.iter().find_map(|(&window_id, rect)| {
        if window_id == exclude {
            return None;
        }
        if x >= rect.x && x <= rect.x + rect.width && y >= rect.y && y <= rect.y + rect.height {
            Some(window_id)
        } else {
            None
        }
    })
}
