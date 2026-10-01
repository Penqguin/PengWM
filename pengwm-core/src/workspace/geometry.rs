use std::collections::HashMap;

use crate::layout::Rect;
use crate::tree::WindowId;

use super::Workspace;

impl Workspace {
    /// Compute global-coordinate rects for every window using the stored monitor
    /// geometry. Tiling is always computed underneath; when `magnified` is set
    /// the magnified window is overwritten with a centered 75%x75% overlay
    /// (tmux-popup style) inside the bar reservation + outer gap.
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

        crate::layout::calculate_layout(root, inset, &self.arena, &mut output, gap_inner);
        if let Some(mag) = self.magnified {
            if output.contains_key(&mag) {
                let w = inset.width * 0.75;
                let h = inset.height * 0.75;
                let local = Rect::new(
                    inset.x + (inset.width - w) / 2.0,
                    inset.y + (inset.height - h) / 2.0,
                    w,
                    h,
                );
                let global = crate::layout::screen_local_to_global(local, self.monitor_origin);
                output.insert(mag, global);
            }
        }
        for (wid, rect) in output.iter_mut() {
            if Some(*wid) == self.magnified {
                continue;
            }
            *rect = crate::layout::screen_local_to_global(*rect, self.monitor_origin);
        }

        output
    }

    /// Toggle magnify on the focused window: pin it as the overlay, or
    /// unpin if it is already magnified. Pinned across focus changes.
    pub fn toggle_magnify(&mut self) {
        let focused = self.focused_window_id();
        match (focused, self.magnified) {
            (Some(f), Some(m)) if f == m => self.magnified = None,
            (Some(f), _) => self.magnified = Some(f),
            (None, _) => self.magnified = None,
        }
    }

    /// Clear a stale magnify pin (close / untrack path).
    pub fn clear_magnify_if(&mut self, window_id: WindowId) {
        if self.magnified == Some(window_id) {
            self.magnified = None;
        }
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
