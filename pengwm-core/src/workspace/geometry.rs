use std::collections::HashMap;

use crate::layout::Rect;
use crate::tree::{NodeData, WindowId};

use super::Workspace;

fn offscreen_rect(_reference: Rect) -> Rect {
    // Far off-screen for monocle siblings — must stay fully invisible
    // even when clamped, unlike hide_workspace which deliberately uses
    // hidden_rect (bottom-right clamped strip) as a daemon-down escape hatch.
    crate::layout::far_offscreen_rect()
}

impl Workspace {
    /// Compute global-coordinate rects for every window using the stored monitor
    /// geometry. Handles monocle internally: the focused window fills the monitor
    /// (minus outer gap); all siblings get offscreen rects.
    pub fn layout(&self, gap_inner: f64, gap_outer: f64) -> HashMap<WindowId, Rect> {
        let Some(root) = self.root else {
            return HashMap::new();
        };

        let monitor_rect = Rect::new(
            0.0,
            0.0,
            self.monitor_size.0 as f64,
            self.monitor_size.1 as f64,
        );
        let usable = self.usable_rect(monitor_rect);
        let inset = crate::layout::inset_rect(usable, gap_outer);
        let mut output = HashMap::new();

        if self.monocle {
            if let Some(focused) = self.focused_node {
                if let Some(node) = self.arena.get(focused) {
                    if let NodeData::Window { window_id, .. } = &node.data {
                        let global =
                            crate::layout::screen_local_to_global(inset, self.monitor_origin);
                        output.insert(*window_id, global);
                    }
                }
            }
            // Keep the original size (inset) while moving far off-screen
            // so the window isn't shrunk to 1x1 and can restore without flicker.
            let offscreen = offscreen_rect(inset);
            for wid in self.arena.all_windows() {
                output.entry(wid).or_insert(offscreen);
            }
        } else {
            crate::layout::calculate_layout(root, inset, &self.arena, &mut output, gap_inner);
            for rect in output.values_mut() {
                *rect = crate::layout::screen_local_to_global(*rect, self.monitor_origin);
            }
        }

        output
    }

    pub fn toggle_monocle(&mut self) {
        self.monocle = !self.monocle;
    }

    /// Global-coordinate origin of the monitor this workspace tiles on.
    pub fn set_monitor_origin(&mut self, origin: (i32, i32)) {
        self.monitor_origin = origin;
    }

    pub fn monitor_origin(&self) -> (i32, i32) {
        self.monitor_origin
    }

    pub fn monitor_size(&self) -> (u32, u32) {
        self.monitor_size
    }

    pub fn update_monitor_geometry(&mut self, origin: (i32, i32), size: (u32, u32)) {
        self.monitor_origin = origin;
        self.monitor_size = size;
    }

    /// Reserve a region of the monitor (in global coordinates) that windows
    /// must avoid — used for the bar strip. `None` clears the reservation.
    pub fn set_reserved_rect(&mut self, global: Option<Rect>) {
        self.reserved = global;
    }

    pub fn reserved_rect(&self) -> Option<Rect> {
        self.reserved
    }

    /// Subtract the reserved region (an edge strip spanning the monitor) from
    /// the full monitor rect, producing the tiling area.
    pub(super) fn usable_rect(&self, monitor: Rect) -> Rect {
        let Some(reserved) = self.reserved else {
            return monitor;
        };
        let local = Rect::new(
            reserved.x - self.monitor_origin.0 as f64,
            reserved.y - self.monitor_origin.1 as f64,
            reserved.width,
            reserved.height,
        );
        crate::layout::subtract_strip(monitor, local)
    }
}
