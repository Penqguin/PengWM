use super::rect::Rect;
use serde::{Deserialize, Serialize};

/// Global-coordinate 1×1 rect at the bottom-right display corner for hidden
/// windows. Placing it at `origin + size - 1` tucks traffic lights off-screen;
/// AppKit clamps the title bar (~28px) into view, leaving a dark strip that
/// is visible in Mission Control as a daemon-down escape hatch without the
/// bright red/yellow/green chrome. Monocle siblings stay far offscreen; only
/// `StateManager::hide_workspace` uses this.
pub fn hidden_rect(origin: (i32, i32), size: (u32, u32)) -> Rect {
    Rect::new(
        origin.0 as f64 + size.0 as f64 - 1.0,
        origin.1 as f64 + size.1 as f64 - 1.0,
        1.0,
        1.0,
    )
}

/// Far off-screen rect for monocle siblings — fully invisible even when
/// `AXPosition` is clamped. Layout restores correct size on switch.
pub fn far_offscreen_rect() -> Rect {
    Rect {
        x: -100_000.0,
        y: -100_000.0,
        width: 0.0,
        height: 0.0,
    }
}

/// Where a hidden window should be placed. The `StateManager` computes the
/// variant; the `OsAdapter` matches on it instead of inspecting raw coordinates.
/// `BottomEdge` keeps the title bar clamped in Mission Control as a daemon-down
/// escape hatch; `FarOffscreen` is truly invisible for monocle siblings.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum HidePlacement {
    /// 1×1 at the bottom-right corner of the owning monitor.
    BottomEdge(Rect),
    /// 0×0 at -100k,-100k, fully invisible.
    FarOffscreen,
}

impl HidePlacement {
    pub fn rect(&self) -> Rect {
        match *self {
            HidePlacement::BottomEdge(r) => r,
            HidePlacement::FarOffscreen => far_offscreen_rect(),
        }
    }

    pub fn is_far_offscreen(&self) -> bool {
        matches!(self, HidePlacement::FarOffscreen)
    }
}
