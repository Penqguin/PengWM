use super::rect::Rect;
use crate::config::BarPosition;

/// The global-coordinate rect of a bar strip spanning one edge of a display.
/// `position` picks the edge; `thickness` is the strip's width (left/right) or
/// height (top/bottom). One answer shared by the daemon's reservation and the
/// bar's self-positioning so the two processes can't drift apart.
pub fn bar_strip_rect(
    origin: (i32, i32),
    size: (u32, u32),
    position: BarPosition,
    thickness: i32,
) -> Rect {
    let t = thickness.max(1) as f64;
    let (ox, oy) = (origin.0 as f64, origin.1 as f64);
    let (w, h) = (size.0 as f64, size.1 as f64);
    match position {
        BarPosition::Top => Rect::new(ox, oy, w, t),
        BarPosition::Bottom => Rect::new(ox, oy + h - t, w, t),
        BarPosition::Left => Rect::new(ox, oy, t, h),
        BarPosition::Right => Rect::new(ox + w - t, oy, t, h),
    }
}

/// Subtract a reserved edge strip (in monitor-local coordinates) from the
/// monitor rect. `strip` is expected to span the full width (top/bottom) or
/// full height (left/right) and sit flush against one edge. Anything that
/// doesn't look like an edge strip is ignored. No EPSILON — both rects are
/// derived from the same integer monitor geometry via `bar_strip_rect`, so
/// exact equality holds.
pub(crate) fn subtract_strip(monitor: Rect, strip: Rect) -> Rect {
    let overlaps_x = strip.x < monitor.x + monitor.width && strip.x + strip.width > monitor.x;
    let overlaps_y = strip.y < monitor.y + monitor.height && strip.y + strip.height > monitor.y;
    if !overlaps_x || !overlaps_y {
        return monitor;
    }

    let spans_width = strip.x == monitor.x && strip.x + strip.width == monitor.x + monitor.width;
    let spans_height = strip.y == monitor.y && strip.y + strip.height == monitor.y + monitor.height;

    if spans_width {
        if strip.y == monitor.y && strip.y + strip.height < monitor.y + monitor.height {
            // top edge
            let cut = strip.y + strip.height;
            let remaining = monitor.y + monitor.height - cut;
            return Rect::new(monitor.x, cut, monitor.width, remaining.max(0.0));
        }
        if strip.y + strip.height == monitor.y + monitor.height && strip.y > monitor.y {
            // bottom edge
            let remaining = strip.y - monitor.y;
            return Rect::new(monitor.x, monitor.y, monitor.width, remaining.max(0.0));
        }
        return monitor;
    }

    if spans_height {
        if strip.x == monitor.x && strip.x + strip.width < monitor.x + monitor.width {
            // left edge
            let cut = strip.x + strip.width;
            let remaining = monitor.x + monitor.width - cut;
            return Rect::new(cut, monitor.y, remaining.max(0.0), monitor.height);
        }
        if strip.x + strip.width == monitor.x + monitor.width && strip.x > monitor.x {
            // right edge
            let remaining = strip.x - monitor.x;
            return Rect::new(monitor.x, monitor.y, remaining.max(0.0), monitor.height);
        }
    }

    monitor
}
