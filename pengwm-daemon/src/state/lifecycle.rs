use std::time::Instant;

use crate::adapter::WindowClass;
use pengwm_core::layout::Rect;
use pengwm_core::tree::WindowId;
use pengwm_core::workspace::position_of_member;

use super::store::{ParkReason, RoutingVerdict};
use super::StateManager;
use crate::event_loop::DaemonEvent;

/// Owns the window/app lifecycle: creation, destruction, hide/show, focus,
/// moves, and app launch/termination/activation. `StateManager` retains the
/// workspace tree, `DisplaySet`, layout application and `BarSender` — this
/// module only orchestrates them. Shared helpers are `pub(super)` so
/// `reconcile.rs` keeps calling the same interface.
impl StateManager {
    pub fn on_window_created(&mut self, window_id: WindowId, pid: i32) {
        self.route_window_created(window_id, pid);
        #[cfg(test)]
        self.assert_identity_invariant();
    }

    /// The routing body of `on_window_created` (ADR-0003): classify at
    /// discovery, decide once, note the verdict, place the window.
    fn route_window_created(&mut self, window_id: WindowId, pid: i32) {
        if self.excluded_pids.contains(&pid) {
            log::debug!("Ignoring window {} from excluded pid {}", window_id, pid);
            return;
        }
        self.store.register(window_id, pid);
        if self.find_workspace_of_any(window_id).is_some() {
            return;
        }

        // The class was classified at the discovery gate and rides the
        // element cache. Uninjected/unknown → Standard: the pre-popup
        // behavior (tile) for any path that somehow evented a window
        // without caching it.
        let class = self
            .os
            .window_kind(window_id)
            .unwrap_or(WindowClass::Standard);
        if !class.is_manageable() {
            log::debug!(
                "WindowCreated {} class {:?} dropped at routing (unmanageable)",
                window_id,
                class
            );
            return;
        }
        let restricted = self.app_is_restricted(pid);
        // Untracked floats when (a) discovery's CG-level gate said so and
        // the gate isn't disabled, or (b) the bundle is in `float_apps`.
        // Popups are decided before the float list: the subrole answer is
        // the more specific signal.
        let float_app = self.app_is_float_app(pid);
        if class == WindowClass::Untracked && self.float_level_overlays && !restricted {
            log::info!(
                "WindowCreated untracked overlay: {} pid={} — left out of the tree",
                window_id,
                pid
            );
            // The verdict is the store's now: the class rode the evictable
            // element cache, and the fallback re-classification reads a
            // bare `Standard` for float-listed bundles on cache misses.
            self.store.note_routed(window_id, RoutingVerdict::Untracked);
            self.publish_bar_state();
            return;
        }
        if class.is_popup() || restricted {
            log::info!(
                "WindowCreated popup: {} pid={} class={:?} restricted={}",
                window_id,
                pid,
                class,
                restricted
            );
            self.store.note_routed(window_id, RoutingVerdict::Popup);
            self.add_popup_to_workspace(window_id);
            self.publish_bar_state();
            return;
        }
        // Precedence (ADR-0002): popup subroles — and restricted's
        // force-popup role — decide *before* the float list; the float
        // list is the tile-boundary fallback below.
        if float_app {
            log::info!(
                "WindowCreated untracked (float_apps): {} pid={} — left out of the tree",
                window_id,
                pid
            );
            self.store.note_routed(window_id, RoutingVerdict::Untracked);
            self.publish_bar_state();
            return;
        }

        let active = self.active_workspace_idx();
        let preferred = self.routed_workspace_for(pid).unwrap_or(active);
        self.store.note_routed(window_id, RoutingVerdict::Tiled);
        self.add_window_to_workspace(window_id, preferred);
        // Discovery-time fast path: a window discovered already fullscreen
        // (daemon restart, or created straight into fullscreen) tiles for
        // one instant, then its refused first write would catch it anyway —
        // parking decides with its own probe.
        self.try_park_fullscreen(window_id);
        self.publish_bar_state();
    }

    /// Attach a popup to the workspace active on the monitor containing the
    /// window's current rect — a dialog centers on the screen the app drew
    /// it on (Q8). Falls back to the active workspace, the same canonical
    /// fallback the tree-routing path uses. No capacity check: popups never
    /// consume `max_tiles`.
    fn add_popup_to_workspace(&mut self, window_id: WindowId) {
        let rect = self.os.window_rect(window_id);
        let target = self
            .workspace_for_containing_monitor(rect)
            .filter(|&idx| idx < self.workspaces.len())
            .unwrap_or_else(|| self.active_workspace_idx());
        if self.workspaces[target].add_popup(window_id)
            && self.displays.is_visible(target, &self.workspaces)
        {
            self.apply_layout(target);
        }
    }

    /// Flat index of the workspace active on the monitor whose bounds
    /// contain `rect`'s center. `None` when no display contains it (or that
    /// display shows no workspace).
    fn workspace_for_containing_monitor(&self, rect: Option<Rect>) -> Option<usize> {
        let rect = rect?;
        let cx = rect.x + rect.width / 2.0;
        let cy = rect.y + rect.height / 2.0;
        for display in self.os.active_displays() {
            let (ox, oy) = display.origin;
            let (w, h) = (display.size.0 as f64, display.size.1 as f64);
            if Rect::new(ox as f64, oy as f64, w, h).contains_point(cx, cy) {
                return self.displays.visible_on(display.id);
            }
        }
        None
    }

    /// `restricted_apps` semantics: every manageable window of these bundles
    /// becomes a workspace-bound popup — never tiled. Bundle id matched
    /// case-insensitively, like the workspace `apps` routing.
    pub(super) fn app_is_restricted(&self, pid: i32) -> bool {
        let bundle = self.os.app_bundle_id(pid);
        self.restricted_apps.iter().any(|app| {
            bundle
                .as_deref()
                .is_some_and(|b| b.eq_ignore_ascii_case(app))
        })
    }

    /// `float_apps` semantics (ADR-0002): every window of these bundles is
    /// untracked — never tiled, on top across workspace switches until
    /// closed. Same case-insensitive bundle-or-name match as
    /// `restricted_apps`.
    pub(super) fn app_is_float_app(&self, pid: i32) -> bool {
        let bundle = self.os.app_bundle_id(pid);
        self.float_apps.iter().any(|app| {
            bundle
                .as_deref()
                .is_some_and(|b| b.eq_ignore_ascii_case(app))
        })
    }

    /// Route `window_id` into a workspace and retile. Prefers `preferred`,
    /// overflowing to the next workspace with capacity. Returns the target
    /// workspace index, or `None` when every workspace is full.
    pub(super) fn add_window_to_workspace(
        &mut self,
        window_id: WindowId,
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
        // Restricted apps tile like everything else (monocle is gone).

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
            if self.workspaces[i].is_popup(window_id) {
                let is_visible = self.displays.is_visible(i, &self.workspaces);
                self.workspaces[i].remove_popup(window_id);
                if is_visible {
                    self.apply_layout(i);
                }
                break;
            }
        }
        self.store.unregister(window_id);
        self.layout_cache.forget(window_id);
        // One funnel for OS-side eviction: destroyed notification, missed
        // notification sweep, gone-grace, and app termination all reach
        // here, so the cached element and its registrations never outlive
        // the window on any path.
        self.os.forget_window(window_id);
        #[cfg(test)]
        self.assert_identity_invariant();
        self.publish_bar_state();
    }

    /// A window was minimized or its app hidden: drop it from its membership
    /// (tree member or popup) so it stops being laid out, but keep pid
    /// tracking so it can be retiled when it becomes visible again.
    pub fn on_window_hidden(&mut self, window_id: WindowId) {
        if self.store.is_fullscreen_parked(window_id) {
            // A hide landed while the window sat fullscreen-parked: upgrade
            // the parking reason and keep the remembered idx. The fullscreen
            // exit probe will now left it alone — the hidden flow owns the
            // window from here, exactly as if it had never fullscreened.
            log::debug!(
                "WindowHidden: fullscreen-parked window {} upgrades to hidden park",
                window_id
            );
            self.store.upgrade_park_to_hide(window_id);
            return;
        }
        let Some(idx) = self.find_workspace_of_any(window_id) else {
            log::debug!("WindowHidden: window {} not tracked, ignoring", window_id);
            return;
        };
        debug_assert!(idx < self.workspaces.len());
        if self.workspaces[idx].is_popup(window_id) {
            self.store.park_popup(window_id, idx);
            self.workspaces[idx].remove_popup(window_id);
        } else {
            self.store
                .park(window_id, &self.workspaces, ParkReason::Hide);
            self.workspaces[idx].remove_window(window_id);
        }
        if self.displays.is_visible(idx, &self.workspaces) {
            self.apply_layout(idx);
        }
        #[cfg(test)]
        self.assert_identity_invariant();
        self.publish_bar_state();
    }

    /// A window was deminiaturized or its app unhidden: re-attach it where
    /// it came from — a popup back into its workspace's popup set, a tree
    /// window back into the remembered (or routed) workspace.
    pub fn on_window_shown(&mut self, window_id: WindowId) {
        self.reveal_window_shown(window_id);
        #[cfg(test)]
        self.assert_identity_invariant();
    }

    /// The reveal body of `on_window_shown`.
    fn reveal_window_shown(&mut self, window_id: WindowId) {
        if self.store.is_fullscreen_parked(window_id) {
            // The probe owns this window's reveal; a stray shown event must
            // not shortcut it.
            log::debug!(
                "WindowShown: fullscreen-parked window {}, ignoring",
                window_id
            );
            return;
        }
        if self.find_workspace_of_any(window_id).is_some() {
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
        if let Some(entry) = remembered.filter(|e| e.popup && e.idx < self.workspaces.len()) {
            self.workspaces[entry.idx].add_popup(window_id);
            if self.displays.is_visible(entry.idx, &self.workspaces) {
                self.apply_layout(entry.idx);
            }
            self.publish_bar_state();
            return;
        }
        self.reveal_tree_window(window_id, pid, remembered.map(|e| e.idx));
    }

    /// The shared tree-window tail of the reveal funnel: re-tile at the
    /// remembered workspace, overflowing through the normal capacity
    /// routing (`add_window_to_workspace` → `target_with_room`) when the
    /// remembered slot no longer has room. Publishes bar state on re-tile.
    fn reveal_tree_window(&mut self, window_id: WindowId, pid: i32, remembered: Option<usize>) {
        let preferred = remembered
            .filter(|&idx| idx < self.workspaces.len())
            .unwrap_or_else(|| {
                self.routed_workspace_for(pid)
                    .unwrap_or_else(|| self.active_workspace_idx())
            });
        if self.add_window_to_workspace(window_id, preferred).is_some() {
            self.publish_bar_state();
        }
    }

    /// True when the window is native fullscreen but this StateManager is
    /// still acting on it — parked, removed from the tree, siblings
    /// reflowed. The parking gate owns the `AXFullScreen` probe; callers
    /// answer the after-write *decision* (`AfterWrite::ProbeFullscreen`)
    /// or a focus event, and never decide fullscreen geometry themselves.
    /// Anything fullscreen-parked or exempt (popups) reports `false`; the
    /// strategy knob gates the whole mechanism (`[windows] fullscreen =
    /// "tile"` = legacy behavior).
    pub(super) fn try_park_fullscreen(&mut self, window_id: WindowId) -> bool {
        if !self.fullscreen_parking_enabled() {
            return false;
        }
        if !self.store.contains(window_id) || self.store.is_parked(window_id) {
            return false;
        }
        // Popups never tile anyway; parking would be no-op overhead.
        if self.is_popup_anywhere(window_id) {
            return false;
        }
        // The fullscreen probe is the gate: a merely pinned (zoomed / busy
        // app) window answers false here and keeps its tree slot.
        if !self.os.window_is_fullscreen(window_id) {
            return false;
        }
        let Some(idx) = self
            .store
            .park(window_id, &self.workspaces, ParkReason::Fullscreen)
        else {
            log::debug!(
                "FullscreenEnter: window {} not in any tree, ignoring",
                window_id
            );
            return false;
        };
        log::info!(
            "FullscreenEnter: parking window {} (workspace {}), siblings reflow",
            window_id,
            idx
        );
        self.workspaces[idx].remove_window(window_id);
        if self.displays.is_visible(idx, &self.workspaces) {
            self.apply_layout(idx);
        }
        #[cfg(test)]
        self.assert_identity_invariant();
        self.publish_bar_state();
        true
    }

    /// The window left native fullscreen: reveal it through the same funnel
    /// as `on_window_shown` — remembered workspace first, capacity overflow
    /// beyond it. Popups never get here (they are exempt from parking).
    pub fn on_fullscreen_exit(&mut self, window_id: WindowId) {
        let Some(entry) = self.store.reveal(window_id) else {
            return;
        };
        let Some(pid) = self.store.pid_for(window_id) else {
            log::debug!("FullscreenExit: unknown window {}, ignoring", window_id);
            return;
        };
        log::info!(
            "FullscreenExit: revealing window {} into workspace {:?}",
            window_id,
            entry.idx
        );
        debug_assert!(!entry.popup, "popups are exempt from fullscreen parking");
        self.reveal_tree_window(window_id, pid, Some(entry.idx));
        #[cfg(test)]
        self.assert_identity_invariant();
    }

    pub fn on_window_focused(&mut self, window_id: WindowId) {
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
                        // Focus-driven switch: the incoming workspace is
                        // already visible — same stacking restoration as
                        // the explicit switch path.
                        self.reveal_stack(Some(mon_id));
                    }
                }
                self.publish_bar_state();
                return;
            }
        }
    }

    /// Workspace index holding `window_id` as a tree member *or* a popup.
    /// The membership-aware find: destroy/hide/show cleanup needs both.
    pub(super) fn find_workspace_of_any(&self, window_id: WindowId) -> Option<usize> {
        position_of_member(&self.workspaces, window_id)
    }

    /// True when any workspace tracks `window_id` as a popup. Popups are
    /// excluded from tree-only funnels by this check: drag hit-testing,
    /// the displaced note, the misplaced sweep, capacity.
    pub(super) fn is_popup_anywhere(&self, window_id: WindowId) -> bool {
        self.workspaces.iter().any(|ws| ws.is_popup(window_id))
    }

    /// The verdict-tree invariant, enforced by the test harness after
    /// every identity-mutating funnel (ADR-0003): an unparked Popup must
    /// sit in some workspace's popup set and never a tree member;
    /// Untracked must be a member of nothing and never parked; an
    /// unparked Tiled must never sit in a popup set. (The positive
    /// Tiled⇒tree-member side is deliberately unchecked: capacity-
    /// overflow windows are legitimately tracked-but-untiled between
    /// layouts.)
    #[cfg(test)]
    pub(super) fn assert_identity_invariant(&self) {
        for &wid in self.store.all_window_pids().keys() {
            match self.store.routing_verdict(wid) {
                None => {} // never routed: lives as it did pre-refactor
                Some(RoutingVerdict::Popup) => {
                    // An unparked popup must sit in a popup set; a parked
                    // one must have left it. Exactly one of the two.
                    assert!(
                        self.store.is_parked(wid) ^ self.is_popup_anywhere(wid),
                        "window {} is Popup-verdicted: parked xor popup-member must hold",
                        wid
                    );
                    assert!(
                        self.workspaces
                            .iter()
                            .all(|ws| ws.find_window(wid).is_none()),
                        "window {} is Popup-verdicted but sits in a tree",
                        wid
                    );
                }
                Some(RoutingVerdict::Untracked) => {
                    assert!(
                        !self.store.is_parked(wid),
                        "window {} is Untracked: ADR-0002 says never parked",
                        wid
                    );
                    assert!(
                        !self.is_popup_anywhere(wid),
                        "window {} is Untracked but sits in a popup set",
                        wid
                    );
                    assert!(
                        self.workspaces
                            .iter()
                            .all(|ws| ws.find_window(wid).is_none()),
                        "window {} is Untracked but sits in a tree",
                        wid
                    );
                }
                Some(RoutingVerdict::Tiled) => {
                    assert!(
                        self.workspaces.iter().all(|ws| !ws.is_popup(wid)),
                        "window {} is Tiled-verdicted but sits in a popup set",
                        wid
                    );
                }
            }
        }
    }

    pub fn on_window_moved(&mut self, window_id: WindowId, x: f64, y: f64) {
        if self.store.is_parked(window_id) {
            // Hidden parks (bottom-edge rects) and fullscreen parks (macOS
            // owns the geometry) both stop the layout machinery here —
            // nothing the WM tracks re-asserts on their rects.
            log::debug!("on_window_moved: ignoring parked window {}", window_id);
            return;
        }
        // Popups are free-drag after placement (placed once at creation and
        // on wake); untracked windows are user-positioned overlays with no
        // placement at all. Both: never displaced-noted (nothing re-centers
        // them) and never a drag-swap participant. The verdict is the
        // durable ask — one lookup, no per-workspace scan.
        match self.store.routing_verdict(window_id) {
            Some(RoutingVerdict::Popup) | Some(RoutingVerdict::Untracked) => return,
            _ => {}
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
    /// normal `on_window_created` path (classification, routing, capacity,
    /// layout all shared — no duplicate logic). One discovery loop for the
    /// tick sweep, the app-activate fast path, and wake resync — callers
    /// pass one pid or all running pids. Only brand-new window ids are
    /// touched: tracked-but-untiled windows are hidden (owned by
    /// `WindowStore`), popups (owned by the workspace), or intentionally
    /// overflowed at capacity.
    ///
    /// The same poll also serves the reverse direction: each pid's listing
    /// is diffed against its tracked windows, and a tracked window the app
    /// no longer lists is untracked immediately — the fast close-detector
    /// for missed destroyed notifications (see `untrack_unlisted_windows`).
    pub(super) fn sync_untracked_windows(&mut self, pids: impl IntoIterator<Item = i32>) {
        let mut listings: Vec<(i32, Vec<WindowId>)> = Vec::new();
        for pid in pids {
            if self.excluded_pids.contains(&pid) {
                continue;
            }
            listings.push((pid, self.os.poll_windows_for_pid(pid)));
        }
        // Untrack dead members first, so the discovery layouts below
        // compute targets against a tree that no longer holds them.
        self.untrack_unlisted_windows(&listings);
        for (pid, windows) in listings {
            for window_id in windows {
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

    /// Untrack every tracked window its own pid's listing no longer
    /// contains. The app listed other windows but not ours: evidence of
    /// death that needs no grace and no destroyed notification — the
    /// missed-notification paths (1Password exit, a Preview window opened
    /// from Finder, a stale ref failing CFEqual) previously stranded the
    /// dead window in the tree until some unrelated event triggered a
    /// layout, then the write-path grace added up to 10s more.
    ///
    /// Judgement rules that make the fast path safe outside wake:
    /// - **Zero listing is no judgment.** An app whose poll returns
    ///   nothing is the post-wake blackout signature, not a mass close.
    /// - **Suppressed while a wake resync is pending** for the same
    ///   reason: during the blackout even live apps list nothing, and a
    ///   partial listing from a recovering app is ambiguous.
    ///
    /// Untracking runs the normal `on_window_destroyed` path (remove,
    /// retile, bar publish), so a closed window's siblings re-tile within
    /// one sweep cycle.
    pub(super) fn untrack_unlisted_windows(&mut self, listings: &[(i32, Vec<WindowId>)]) {
        if !self.may_judge("untrack_unlisted") {
            return;
        }
        for (pid, listed) in listings {
            if !StateManager::listing_is_evidence(listed) {
                continue;
            }
            let tracked = self
                .store
                .windows_for(*pid)
                .map(|w| w.to_vec())
                .unwrap_or_default();
            for window_id in tracked {
                if !listed.contains(&window_id) {
                    log::info!(
                        "untrack_unlisted: window {} pid {} no longer listed by its app, untracking",
                        window_id,
                        pid
                    );
                    self.on_window_destroyed(window_id);
                }
            }
        }
    }

    /// A destroyed notification arrived for an element that could not be
    /// mapped to a window id. The owning pid still tells us who reported
    /// the death: poll that pid and untrack its tracked-but-unlisted
    /// windows — the same evidence the sweep uses, scoped to the one app
    /// that reported a death, so the response is immediate rather than
    /// waiting for the next 2s sweep.
    pub fn on_unknown_window_destroyed(&mut self, pid: i32) {
        if self.excluded_pids.contains(&pid) || !self.may_judge("unknown-destroyed probe") {
            return;
        }
        let windows = self.os.poll_windows_for_pid(pid);
        if !StateManager::listing_is_evidence(&windows) {
            // Zero listing: no judgment (blackout signature).
            return;
        }
        self.untrack_unlisted_windows(&[(pid, windows)]);
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
        // The label cache is pid-keyed and dies with the pid; lingering
        // entries would mislabel a recycled pid forever.
        self.app_labels.remove(&pid);
        let windows = self.store.remove_pid(pid);
        for window_id in windows {
            // Synchronous eviction: the queued WindowDestroyed dispatches
            // reach the same funnel later, but the observer (and with it
            // every per-window AX registration) is already gone — nothing
            // needs the cached elements from here on.
            self.os.forget_window(window_id);
            self.layout_cache.forget(window_id);
            let _ = self
                .event_tx
                .try_send(DaemonEvent::WindowDestroyed(window_id));
        }
        // Evict what the per-window loop can't see: elements the observer
        // cached for windows the store never tracked (an excluded app's
        // post-bootstrap windows route out before `register`). Without
        // this, those refs outlive the app forever.
        self.os.forget_pid(pid);
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
