//! Owns focus timing and acceptance, not membership or output assignment.
//!
//! Observations enter through the same explicit-time operations in production
//! and tests. Workspace/DisplaySet retain focus truth; this module owns when
//! to update it and preserves publication and fullscreen-parking checkpoints.
use std::time::{Duration, Instant};

use pengwm_core::tree::WindowId;

use super::StateManager;

const FOCUS_QUIET_PERIOD: Duration = Duration::from_millis(250);
const SWITCH_PROTECTION: Duration = Duration::from_millis(150);

/// Temporal state is private here: callers report intents, not field edits.
#[derive(Default)]
pub(super) struct FocusState {
    dirty_since: Option<Instant>,
    switch_until: Option<Instant>,
    /// Last settled OS observation, even if acceptance rejects the window.
    last_window: Option<WindowId>,
}

impl StateManager {
    /// Direct observations (including activation) do not arm the quiet drain.
    pub fn on_window_focused(&mut self, window_id: WindowId) {
        self.accept_focus_at(window_id, Instant::now());
    }

    /// External notifications arm settling and attempt low-latency acceptance.
    pub fn on_window_focused_event(&mut self, window_id: WindowId) {
        self.on_window_focused_event_at(window_id, Instant::now());
    }

    pub(super) fn on_window_focused_event_at(&mut self, window_id: WindowId, now: Instant) {
        self.focus.dirty_since = Some(now);
        self.accept_focus_at(window_id, now);
    }

    /// Called after a successful selection's placement/focus/stacking effects.
    pub(super) fn protect_workspace_switch(&mut self, now: Instant) {
        self.focus.switch_until = Some(now + SWITCH_PROTECTION);
    }

    /// Structural reset clears protection but retains quiet work and memory.
    pub(super) fn reset_focus_protection(&mut self) {
        self.focus.switch_until = None;
    }

    /// Inspect whether a settle remains armed, including across wake blackout.
    #[cfg(test)]
    pub(super) fn focus_settle_dirty(&self) -> bool {
        self.focus.dirty_since.is_some()
    }

    pub(super) fn accept_focus_at(&mut self, window_id: WindowId, now: Instant) {
        // Ignore focus from parked windows (hidden parks steal focus via
        // clamped title bars; fullscreen parks have no tree slot to focus).
        if self.store.is_parked(window_id) {
            log::debug!("on_window_focused: ignoring parked window {}", window_id);
            return;
        }
        // Fullscreen enter usually re-fires focused-window-changed — the
        // cheap fast path into parking, which decides with its own probe.
        // Cheap even when idle: focused events are rare, and the gate
        // answers "no" in one AX read.
        if self.try_park_fullscreen(window_id) {
            return;
        }
        let Some(i) = self.find_workspace_of_any(window_id) else {
            return;
        };
        // A hidden workspace cannot become visible through focus observations.
        if !self.displays.is_visible(i, &self.workspaces) {
            log::debug!(
                "on_window_focused: ignoring hidden-workspace window {} (ws {})",
                window_id,
                i
            );
            return;
        }
        self.workspaces[i].focus_window(window_id);
        let mon_id = self.workspaces[i].monitor_id;
        // Workspace focus has already changed. Switch protection holds
        // only the focused-output seat; visibility independently rejects
        // hidden members, even after this deadline has expired.
        if let Some(until) = self.focus.switch_until {
            if now < until {
                self.publish_bar_state();
                return;
            } else {
                self.focus.switch_until = None;
            }
        }
        self.displays.set_focused_output(mon_id);
        self.publish_bar_state();
    }

    /// Re-read OS focus after the quiet period (docs/specs/0005-focus-timing.md).
    /// OS observation outranks notification arrival order. With OS None, try
    /// settled memory if it is still visible; otherwise leave the seat alone.
    ///
    /// Suppressed while a wake resync is pending: the drain's answer is an
    /// AX read (`frontmost_pid` / the focused-window ask), and through the
    /// blackout it reads None → ladder (b) would use settled focus memory
    /// on blackout evidence while the resync is still probing. The settle
    /// stays armed (not consumed) and commits on the first post-commit
    /// tick — the resync commit itself re-reads `frontmost_pid`, so the
    /// settle's inputs are healthy by then.
    pub(in crate::state) fn commit_settled_focus(&mut self, now: Instant) {
        let Some(since) = self.focus.dirty_since else {
            return;
        };
        if !self.may_judge("focus settle") {
            // Armed, not consumed: the settle window resumed after wake.
            return;
        }
        if now.duration_since(since) < FOCUS_QUIET_PERIOD {
            return;
        }
        self.focus.dirty_since = None;
        // An explicit switch owns this stretch of focus truth; the
        // switch's own bookkeeping is the authority.
        if let Some(until) = self.focus.switch_until {
            if now < until {
                return;
            }
        }
        let frontmost = self.os.frontmost_pid();
        // The frontmost cell feeds the active-workspace fallback: fresh
        // per settle so it never falls so far behind reality that the
        // fallback answers a process that isn't the one the user is in.
        self.frontmost_pid = frontmost;
        let window = frontmost.and_then(|pid| self.os.focused_window_for_pid(pid));
        match window {
            Some(window_id) => {
                // Memory records OS observation before acceptance, including a
                // hidden/parked/unknown candidate. Only OS None uses fallback.
                self.focus.last_window = Some(window_id);
                self.accept_focus_at(window_id, now);
            }
            None => {
                // Ladder (b): no OS focus (desktop clicked, keyboardless,
                // focus claims nothing) → the last focused window, if it
                // is still a visible tree member.
                if let Some(wid) = self.focus.last_window {
                    let member_of = self.find_workspace_of_any(wid);
                    if let Some(idx) = member_of {
                        if self.displays.is_visible(idx, &self.workspaces) {
                            log::debug!(
                                "focus settle: no OS focus — falling back to last focused window {}",
                                wid
                            );
                            self.accept_focus_at(wid, now);
                            return;
                        }
                    }
                }
                // Ladder (c): no windows visible (or the last focus went
                // hidden) — the focused workspace *seat* is the memory:
                // nothing to change, nothing to cross-check.
                log::debug!(
                    "focus settle: no OS focus and no visible last window — workspace seat stands"
                );
            }
        }
    }
}
