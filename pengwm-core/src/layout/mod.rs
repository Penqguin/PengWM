mod bar;
mod hide;
mod query;
mod rect;
#[cfg(test)]
mod tests;
mod write;

pub use bar::bar_strip_rect;
pub(crate) use bar::subtract_strip;
pub use hide::{far_offscreen_rect, hidden_rect, HidePlacement};
pub use query::window_at_point;
pub use rect::{
    calculate_layout, centered_overlay_rect, rects_close, rects_displaced, Rect, DISPLACE_GRACE,
    LAYOUT_EPSILON,
};
pub(crate) use rect::{inset_rect, screen_local_to_global};
pub use write::WriteOutcome;
