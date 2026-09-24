mod add_remove;
mod focus_swap;
mod geometry;
mod preset;
mod split;
#[cfg(test)]
mod tests;

pub use preset::{
    clamp_main_ratio, LayoutPreset, MAIN_RATIO_MAX, MAIN_RATIO_MIN, MIN_PANE_SHARE, RESIZE_STEP,
};

use crate::layout::Rect;
use crate::tree::{Arena, NodeId, SplitDirection};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    pub name: String,
    pub monitor_id: u32,
    pub focused_node: Option<NodeId>,
    pub monocle: bool,
    pending_split: Option<SplitDirection>,
    root: Option<NodeId>,
    arena: Arena,
    monitor_origin: (i32, i32),
    monitor_size: (u32, u32),
    /// Global-coordinate region of the monitor that is off-limits to windows
    /// (e.g. a status bar strip). Applied in `layout()` before the gap inset.
    reserved: Option<Rect>,
}

impl Workspace {
    pub fn new(name: String, monitor_id: u32, origin: (i32, i32), size: (u32, u32)) -> Self {
        Self {
            name,
            monitor_id,
            focused_node: None,
            monocle: false,
            pending_split: None,
            root: None,
            arena: Arena::new(),
            monitor_origin: origin,
            monitor_size: size,
            reserved: None,
        }
    }
}

/// Equal shares for `n` children. Callers guarantee `n >= 1`.
pub(super) fn equal_shares(n: usize) -> Vec<f64> {
    vec![1.0 / n as f64; n]
}
