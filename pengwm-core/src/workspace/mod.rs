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
use crate::tree::{Arena, NodeId, SplitDirection, WindowId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    pub name: String,
    pub monitor_id: u32,
    pub focused_node: Option<NodeId>,
    /// tmux-popup style magnified window: drawn as a centered 75% overlay
    /// while the tiling underneath stays computed but obscured. Pinned
    /// across focus and workspace switches; cleared on close, and by a
    /// preset (an explicit arrangement).
    #[serde(default)]
    pub magnified: Option<WindowId>,
    /// Workspace-bound popups (dialogs, floating panels, restricted apps):
    /// tracked but never tiled — `layout()` renders each as a centered
    /// overlay above the tiling, which stays computed underneath. Cleared
    /// on close/hide; never persisted (sanitize rebuilds empty).
    #[serde(default)]
    pub popups: Vec<WindowId>,
    /// Share of the usable area a popup overlay takes (`popup_ratio`
    /// config; clamped at use). Magnify keeps its own fixed 0.75.
    #[serde(default = "default_popup_ratio")]
    pub popup_ratio: f64,
    /// Per-workspace position in `LayoutPreset::all()` for `opt+t` cycling.
    /// Defaults to Tiled (last). Updated by `apply_preset` / `cycle_preset`.
    #[serde(default = "default_preset_index")]
    pub preset_index: usize,
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
            magnified: None,
            popups: Vec::new(),
            popup_ratio: default_popup_ratio(),
            preset_index: default_preset_index(),
            pending_split: None,
            root: None,
            arena: Arena::new(),
            monitor_origin: origin,
            monitor_size: size,
            reserved: None,
        }
    }
}

fn default_preset_index() -> usize {
    // `LayoutPreset::all()` order ends with Tiled — the default arrangement.
    LayoutPreset::all()
        .iter()
        .position(|p| *p == LayoutPreset::Tiled)
        .unwrap_or(0)
}

/// Default share of the usable area a popup overlay takes. One definition:
/// the config default and `Workspace`'s serde default both reference it.
pub const POPUP_RATIO_DEFAULT: f64 = 0.75;
/// Lower bound for the configurable popup ratio — a zero share would make
/// the overlay disappear while the window stays tracked.
pub const POPUP_RATIO_MIN: f64 = 0.1;

fn default_popup_ratio() -> f64 {
    POPUP_RATIO_DEFAULT
}

/// Equal shares for `n` children. Callers guarantee `n >= 1`.
pub(super) fn equal_shares(n: usize) -> Vec<f64> {
    vec![1.0 / n as f64; n]
}
