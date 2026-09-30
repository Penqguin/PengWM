use std::time::Instant;

use pengwm_core::tree::WindowId;

use super::StateManager;
use crate::event_loop::DaemonEvent;

/// Owns the window/app lifecycle: creation, destruction, hide/show, focus,
/// moves, and app launch/termination/activation. `StateManager` retains the
/// workspace tree, `DisplaySet`, layout application and `BarSender` — this
/// module only orchestrates them. Shared helpers are `pub(super)` so
/// `reconcile.rs` keeps calling the same interface.
impl StateManager {
    pub fn on_window_created(&mut self, window_id: WindowId, pid: i32) {
        if self.excluded_pids.contains(&pid) {
            log::debug!("Ignoring window {} from excluded pid {}", window_id, pid);
            return;
        }
        self.store.register(window_id, pid);
        if self.find_workspace_for_window(window_id).is_some() {
            return;
        }

        let active = self.active_workspace_idx();
        let preferred = self
            .displays
            .routed_workspace_idx(pid, &self.workspaces, &*self.os)
            .unwrap_or(active);
        self.add_window_to_workspace(window_id, pid, preferred);
        self.publish_bar_state();
    }

    /// Route `window_id` into a workspace and retile. Prefers `preferred`,
    /// overflowing to the next workspace with capacity. Returns the target
    /// workspace index, or `None` when every workspace is full.
    pub(super) fn add_window_to_workspace(
        &mut self,
        window_id: WindowId,
        pid: i32,
        preferred: usize,
    ) -> Option<usize> {
        if self.workspaces[preferred].find_window(window_id).is_some() {
            return Some(preferred);
        }
        let target = match self.displays.target_with_room(&self.workspaces, preferred) {
            Some(idx) => {
                if idx != preferred {
                    log::info!(
                        "Workspace {} full ({} >= {}), routing new window to workspace {}",
                        preferred,
                        self.workspaces[preferred].window_count(),
                        self.displays.max_tiles(),
                        idx
                    );
                }
                idx
            }
            None => {
                log::warn!(
                    "All workspaces at capacity ({}), leaving window {} untracked",
                    self.displays.max_tiles(),
                    window_id
                );
                return None;
            }
        };

        let ws = &mut self.workspaces[target];
        if !self.restricted_apps.is_empty() {
            if let Some(bundle_id) = self.os.app_bundle_id(pid) {
                if self.restricted_apps.contains(&bundle_id) {
                    log::info!("App {} is restricted — enabling monocle", bundle_id);
                    ws.monocle = true;
                }
            }
        }

        ws.add_window(window_id, None);
        if self.displays.is_visible(target, &self.workspaces) {
            self.apply_layout(target);
        }
        Some(target)
    }

    pub fn on_window_destroyed(&mut self, window_id: WindowId) {
        for i in 0..self.workspaces.len() {
            if self.workspaces[i].find_window(window_id).is_some() {
                let is_visible = self.displays.is_visible(i, &self.workspaces);
                let ws = &mut self.workspaces[i];
                ws.remove_window(window_id);
                if is_visible {
                    self.apply_layout(i);
                }
                break;
            }
        }
        self.store.unregister(window_id);
        self.layout_cache.forget(window_id);
        self.publish_bar_state();
    }

    /// A window was minimized or its app hidden: drop it from the tree (like a
    /// close) so it stops occupying tiled space, but keep pid tracking so it
    /// can be retiled when it becomes visible again.
    pub fn on_window_hidden(&mut self, window_id: WindowId) {
        if let Some(idx) = self.store.hide(window_id, &self.workspaces) {
            debug_assert!(idx < self.workspaces.len());
            self.workspaces[idx].remove_window(window_id);
            if self.displays.is_visible(idx, &self.workspaces) {
                self.apply_layout(idx);
            }
            self.publish_bar_state();
        } else {
            log::debug!("WindowHidden: window {} not tracked, ignoring", window_id);
        }
    }

    /// A window was deminiaturized or its app unhidden: retile it back into
    /// the workspace it came from (or the active one).
    pub fn on_window_shown(&mut self, window_id: WindowId) {
        if self.find_workspace_for_window(window_id).is_some() {
            return;
        }
        let pid = match self.store.pid_for(window_id) {
            Some(p) => p,
            None => {
                log::debug!("WindowShown: unknown window {}, ignoring", window_id);
                return;
            }
        };
        let remembered = self.store.reveal(window_id);
        let preferred = remembered
            .filter(|&idx| idx < self.workspaces.len())
            .unwrap_or_else(|| {
                self.displays
                    .routed_workspace_idx(pid, &self.workspaces, &*self.os)
                    .unwrap_or_else(|| self.active_workspace_idx())
            });
        if self
            .add_window_to_workspace(window_id, pid, preferred)
            .is_some()
        {
            self.publish_bar_state();
        }
    }

    pub fn on_window_focused(&mut self, window_id: WindowId) {
        // Ignore focus from windows parked at the bottom-edge hidden rect —
        // clamped title bars retain hit-testing and would otherwise steal focus
        // or flip DisplaySet::active.
        if self.store.is_hidden(window_id) {
            log::debug!("on_window_focused: ignoring hidden window {}", window_id);
            return;
        }
        for i in 0..self.workspaces.len() {
            if self.workspaces[i].find_window(window_id).is_some() {
                self.workspaces[i].focus_window(window_id);
                let mon_id = self.workspaces[i].monitor_id;
                // Debounce: don't mutate DisplaySet::active on focus notifications
                // that arrive immediately after an explicit workspace switch — they
                // are stale observer events for the window that was just hidden
                // and would drag the old workspace back into view.
                if let Some(until) = self.switch_debounce_until {
                    if Instant::now() < until {
                        self.publish_bar_state();
                        return;
                    } else {
                        self.switch_debounce_until = None;
                    }
                }
                let prev = self.displays.active_mut().insert(mon_id, i);
                self.displays.set_focused_output(mon_id);
                if let Some(prev_idx) = prev {
                    if prev_idx != i {
                        self.hide_workspace(prev_idx);
                    }
                }
                self.publish_bar_state();
                return;
            }
        }
    }

    pub(super) fn find_workspace_for_window(&self, window_id: WindowId) -> Option<usize> {
        self.workspaces
            .iter()
            .position(|ws| ws.find_window(window_id).is_some())
    }

    pub fn on_window_moved(&mut self, window_id: WindowId, x: f64, y: f64) {
        if self.store.is_hidden(window_id) {
            log::debug!("on_window_moved: ignoring hidden window {}", window_id);
            return;
        }
        let now = Instant::now();
        // The event coords drive the drag gesture; displacement reads the OS
        // rect (cheap, no reflow) so the note shares the sweep's full-rect
        // predicate instead of trusting possibly-coalesced event coords.
        let actual = self.os.window_rect(window_id);
        self.layout_cache.note_displaced(window_id, actual);
        self.drag.on_moved(
            window_id,
            x,
            y,
            &self.workspaces,
            &self.last_layout_rects,
            now,
        );
    }

    /// Poll apps for windows the store has never seen and tile them via the
    /// normal `on_window_created` path (routing, capacity, monocle, layout
    /// all shared — no duplicate logic). One discovery loop for the tick
    /// sweep, the app-activate fast path, and wake resync — callers pass
    /// one pid or all running pids. Only brand-new window ids are touched:
    /// tracked-but-untiled windows are either hidden (owned by
    /// `HiddenTracker`) or intentionally overflowed at capacity.
    pub(super) fn sync_untracked_windows(&mut self, pids: impl IntoIterator<Item = i32>) {
        for pid in pids {
            if self.excluded_pids.contains(&pid) {
                continue;
            }
            for window_id in self.os.poll_windows_for_pid(pid) {
                if !self.store.contains(window_id) {
                    log::info!(
                        "sync_untracked: discovered untracked window {} pid {}",
                        window_id,
                        pid
                    );
                    self.on_window_created(window_id, pid);
                }
            }
        }
    }
    pub fn on_app_launched(&mut self, pid: i32) {
        log::info!("App launched: pid={}", pid);
        if self.excluded_pids.contains(&pid) {
            log::debug!("Skipping excluded app launch: pid={}", pid);
            return;
        }
        self.os.attach_observer(pid);
        for window_id in self.os.poll_windows_for_pid(pid) {
            self.store.register(window_id, pid);
            let _ = self
                .event_tx
                .try_send(DaemonEvent::WindowCreated(window_id, pid));
        }
    }

    pub fn on_app_terminated(&mut self, pid: i32) {
        log::info!("App terminated: pid={}", pid);
        self.os.detach_observer(pid);
        let windows = self.store.remove_pid(pid);
        for window_id in windows {
            self.layout_cache.forget(window_id);
            let _ = self
                .event_tx
                .try_send(DaemonEvent::WindowDestroyed(window_id));
        }
        self.publish_bar_state();
    }

    pub fn on_app_activated(&mut self, pid: i32) {
        log::debug!("App activated: pid={}", pid);
        self.frontmost_pid = Some(pid);
        // Fast path for tear-offs / incognito: the new window usually takes
        // focus, which fires AppActivated even when WindowCreated was missed.
        // A single-pid poll here tiles it immediately instead of waiting for
        // the 2s background sweep. Manageable-filtering still excludes PiP.
        self.sync_untracked_windows([pid]);
        if let Some(window_id) = self.os.focused_window_for_pid(pid) {
            self.on_window_focused(window_id);
        } else {
            self.publish_bar_state();
        }
    }
}
