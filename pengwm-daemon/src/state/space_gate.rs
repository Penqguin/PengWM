//! The native-Space gate (ADR-0007).
//!
//! PengWM manages only the first user desktop of each display. macOS
//! Spaces are per display (with "Displays have separate Spaces"), so the
//! gate is per display too: while display A shows Desktop 2, its windows
//! are not touched, while display B on Desktop 1 stays fully managed.
//!
//! The rule is an *action* gate, not a membership filter: the model keeps
//! tracking windows wherever they are, and every OS-visible write
//! (`apply_layout`, hide placements, raises, focus, commands) is gated by
//! the acting display. On return the display is resynced from the model,
//! including the hide placements deferred while it was away.
//!
//! The detector is the read-only SkyLight snapshot in `macos::spaces`;
//! `None` (unavailable) fails open — every display stays fully managed.
//! The snapshot is polled at most every [`SPACE_POLL_INTERVAL`] on the
//! tick and refreshed immediately by the ActiveSpaceDidChange
//! notification (and by every command, which cannot afford staleness).

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use pengwm_core::tree::WindowId;

use super::StateManager;

/// Tick poll pace. The notification is the prompt path; this catches
/// transitions macOS does not post (window drags between spaces, some
/// fullscreen changes).
const SPACE_POLL_INTERVAL: Duration = Duration::from_millis(100);

impl StateManager {
    /// Refresh the gate from the adapter and act on transitions.
    ///
    /// Entering limbo: log and clear the drag state — a drag in flight
    /// must not keep swapping tiles on a desktop the user is leaving.
    /// Leaving limbo: resync the display (visible layout, deferred hides,
    /// stacking). `force` bypasses the tick throttle for the
    /// notification/command paths, where 100 ms of staleness could let an
    /// action land on the wrong Space.
    pub(super) fn refresh_space_gate(&mut self, now: Instant, force: bool) {
        if !force {
            if let Some(last) = self.space_snapshot_at {
                if now.duration_since(last) < SPACE_POLL_INTERVAL {
                    return;
                }
            }
        }
        self.space_snapshot_at = Some(now);

        let Some(states) = self.os.displays_on_first_desktop() else {
            if !self.space_detector_warned {
                self.space_detector_warned = true;
                log::warn!(
                    "native-space gate unavailable — every display stays fully managed (read-only query; no SIP changes involved)"
                );
            }
            self.leave_all_limbo("detector unavailable");
            return;
        };
        if self.space_detector_warned {
            self.space_detector_warned = false;
            log::info!("native-space gate recovered");
        }

        // Only displays that host a workspace matter. A display missing
        // from the snapshot is allowed (fail open).
        let next: BTreeSet<u32> = self
            .displays
            .active()
            .keys()
            .copied()
            .filter(|monitor| states.get(monitor) == Some(&false))
            .collect();
        let entered: Vec<u32> = next
            .iter()
            .filter(|monitor| !self.space_limbo.contains(*monitor))
            .copied()
            .collect();
        let left: Vec<u32> = self
            .space_limbo
            .iter()
            .filter(|monitor| !next.contains(*monitor))
            .copied()
            .collect();
        self.space_limbo = next;

        if !entered.is_empty() {
            self.retain_workspace_pills();
        }
        for monitor in entered {
            log::info!(
                "display {} entered a non-first native desktop — WM idle there until it returns",
                monitor
            );
            self.drag.clear();
        }
        for monitor in left {
            log::info!(
                "display {} returned to its first native desktop — resyncing",
                monitor
            );
            self.resync_display(monitor);
        }
        // Workspaces can move displays while deferred hides wait (a
        // hotplug remap): drain any whose new display is allowed instead
        // of stranding the entry until a monitor id happens to return.
        self.drain_deferred_hides();
    }

    /// The one gate question: may the WM act on this display right now?
    pub(super) fn display_may_act(&self, monitor: u32) -> bool {
        !self.space_limbo.contains(&monitor)
    }

    /// The gate for commands that act on the focused workspace.
    pub(super) fn active_display_may_act(&self) -> bool {
        match self.workspaces.get(self.active_workspace_idx()) {
            Some(ws) => self.display_may_act(ws.monitor_id),
            // No workspace answer: nothing to act on anyway.
            None => true,
        }
    }

    /// The gate for a workspace-indexed action / judgment.
    pub(super) fn workspace_may_act(&self, workspace_idx: usize) -> bool {
        self.workspaces
            .get(workspace_idx)
            .map(|ws| self.display_may_act(ws.monitor_id))
            .unwrap_or(true)
    }

    /// The gate for a window-indexed judgment: a window homed to a display
    /// in limbo must not be untracked, hidden, or revealed. Windows with
    /// no known home (untracked overlays, fresh discoveries) are judged
    /// normally.
    pub(super) fn window_may_act(&self, window_id: WindowId) -> bool {
        self.window_home_monitor(window_id)
            .map(|monitor| self.display_may_act(monitor))
            .unwrap_or(true)
    }

    /// The display a tracked window belongs to: its tree workspace, its
    /// popup workspace, or the workspace a parked window remembers.
    pub(super) fn window_home_monitor(&self, window_id: WindowId) -> Option<u32> {
        let idx = self
            .find_workspace_of_any(window_id)
            .or_else(|| self.workspaces.iter().position(|ws| ws.is_popup(window_id)))
            .or_else(|| self.store.parked_idx(window_id))?;
        self.workspaces.get(idx).map(|ws| ws.monitor_id)
    }

    /// Queue a hide placement that cannot be written while its display is
    /// away. Drained by [`Self::resync_display`] on return; stale entries
    /// (window destroyed, moved to another workspace) are discarded there.
    pub(super) fn defer_hide(&mut self, window_id: WindowId, workspace_idx: usize) {
        self.deferred_hides.insert(window_id, workspace_idx);
    }

    /// Re-assert one returning display from the model: drain its deferred
    /// hide placements, re-tile whichever workspace it shows, restore
    /// stacking. Scoped by design — a display still in limbo is untouched.
    fn resync_display(&mut self, monitor: u32) {
        // Deferred hides first, so windows admitted to hidden workspaces
        // while away go offscreen before the visible layout is re-asserted.
        self.drain_deferred_hides();

        if let Some(idx) = self.displays.visible_on(monitor) {
            if idx < self.workspaces.len() {
                self.apply_layout(idx);
            }
        }
        self.reveal_stack(Some(monitor));
        self.publish_bar_state();
    }

    /// Write every deferred hide whose workspace is currently on an
    /// allowed display and is not visible. Entries still waiting on a
    /// gated display stay queued; entries whose window left the workspace
    /// (destroyed or moved) and entries whose workspace became visible
    /// are dropped — the visible layout owns those windows.
    fn drain_deferred_hides(&mut self) {
        if self.deferred_hides.is_empty() {
            return;
        }
        enum Action {
            Keep,
            Drop,
            Hide,
        }
        let actions: Vec<(WindowId, usize, Action)> = self
            .deferred_hides
            .iter()
            .map(|(&window_id, &workspace_idx)| {
                let Some(ws) = self.workspaces.get(workspace_idx) else {
                    return (window_id, workspace_idx, Action::Drop);
                };
                if self.displays.is_visible(workspace_idx, &self.workspaces) {
                    return (window_id, workspace_idx, Action::Drop);
                }
                if !self.display_may_act(ws.monitor_id) {
                    return (window_id, workspace_idx, Action::Keep);
                }
                let member = self.find_workspace_of_any(window_id) == Some(workspace_idx)
                    || ws.is_popup(window_id);
                if !member {
                    return (window_id, workspace_idx, Action::Drop);
                }
                (window_id, workspace_idx, Action::Hide)
            })
            .collect();
        for (window_id, workspace_idx, action) in actions {
            match action {
                Action::Keep => {}
                Action::Drop => {
                    self.deferred_hides.remove(&window_id);
                }
                Action::Hide => {
                    self.deferred_hides.remove(&window_id);
                    self.hide_window_in_workspace(window_id, workspace_idx);
                }
            }
        }
    }

    /// Detector vanished: fail open and resync every display that was
    /// gated (their deferred writes must not be stranded).
    fn leave_all_limbo(&mut self, reason: &str) {
        if self.space_limbo.is_empty() {
            return;
        }
        let left: Vec<u32> = self.space_limbo.iter().copied().collect();
        self.space_limbo.clear();
        for monitor in left {
            log::info!("display {} gate cleared ({}) — resyncing", monitor, reason);
            self.resync_display(monitor);
        }
    }

    /// Force an immediate snapshot (the `SpaceChanged` notification).
    pub(crate) fn on_space_changed(&mut self) {
        self.refresh_space_gate(Instant::now(), true);
    }

    #[cfg(test)]
    pub(super) fn refresh_space_gate_for_test(&mut self) {
        self.refresh_space_gate(Instant::now(), true);
    }
}
