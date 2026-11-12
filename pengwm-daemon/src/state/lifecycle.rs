use std::time::Instant;

use crate::adapter::WindowClass;
use crate::adapter::WindowListing;
use pengwm_core::layout::Rect;
use pengwm_core::tree::WindowId;
use pengwm_core::workspace::PendingKind;

use super::store::RoutingVerdict;
use super::{AppIdentity, StateManager};
use crate::event_loop::DaemonEvent;

/// Owns the window/app lifecycle: creation, destruction, hide/show, focus,
/// moves, and app launch/termination/activation. Routing and event-specific
/// publication stay here; membership transitions execute in `membership`,
/// focus acceptance and temporal decisions in `focus`.
/// Shared helpers are `pub(super)` so reconciliation uses the same funnels.
impl StateManager {
    pub fn on_window_created(&mut self, window_id: WindowId, pid: i32) {
        let identity = self.app_identity(pid);
        self.route_window_created(window_id, pid, &identity);
        #[cfg(test)]
        self.assert_identity_invariant();
    }

    /// The routing body of `on_window_created` (ADR-0003): classify at
    /// discovery, decide once, note the verdict, place the window.
    /// `identity` is the pass's per-pid NSWorkspace resolution — see
    /// `AppIdentity`; the burst discovery loop passes one per pid.
    fn route_window_created(&mut self, window_id: WindowId, pid: i32, identity: &AppIdentity) {
        if self.excluded_pids.contains(&pid) {
            log::debug!("Ignoring window {} from excluded pid {}", window_id, pid);
            return;
        }
        if self.store.is_parked(window_id) {
            return;
        }
        if self.store.pid_for(window_id).is_some_and(|old| old != pid) {
            self.on_window_destroyed(window_id);
        }
        if self.find_workspace_of_any(window_id).is_some() {
            return;
        }
        self.store.register(window_id, pid);
        if self.find_workspace_of_any(window_id).is_some() {
            return;
        }

        // The class rides the element cache — but routing is the moment
        // popup-vs-tile is DECIDED, so the subrole is re-read fresh here
        // (`window_class_fresh`): first-read-wins in the cache froze a
        // burst-read Dialog lie (Firefox after a hotplug) into a
        // permanent popup fate. A fresh read that fails (unreadable
        // element) falls back to the cached answer, exactly like the
        // old default read did.
        let class = self
            .os
            .window_class_fresh(window_id)
            .or_else(|| self.os.window_kind(window_id))
            .unwrap_or(WindowClass::Standard);
        if !class.is_manageable() {
            log::debug!(
                "WindowCreated {} class {:?} dropped at routing (unmanageable)",
                window_id,
                class
            );
            return;
        }
        if !self.window_is_tileable(Some(class), identity) {
            let restricted = self.app_is_restricted(identity.bundle.as_deref());
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
            // An Untracked class with the level gate enabled, or an app in
            // `float_apps`, remains outside workspace membership.
            if class == WindowClass::Untracked && self.float_level_overlays
                || self.app_is_float_app(identity)
            {
                log::info!(
                    "WindowCreated untracked overlay: {} pid={} — left out of the tree",
                    window_id,
                    pid
                );
                // The verdict is the store's now: the class rode the
                // evictable element cache, and fallback re-classification
                // can lose the original CG-level evidence.
                self.store.note_routed(window_id, RoutingVerdict::Untracked);
                self.publish_bar_state();
                return;
            }
            log::debug!(
                "WindowCreated {} class {:?} rejected by placement policy",
                window_id,
                class
            );
            return;
        }

        // The one placement ask (`state::placement`): the seatbelt —
        // the most current fact about this window, and a rediscovery
        // answer only (ADR-0005: a birth never re-homes to a
        // remembered workspace). Then the config entry match (what the
        // app *is*). Then the active workspace — unassigned windows
        // open where the user is.
        let remembered = self.where_should(window_id, pid);
        // The stale record's pending intent and opening order ride the
        // seatbelt answer only: an expired tombstone (confirmed destroy)
        // means this is a new window, which opens with a fresh order and
        // the placement policy's kind, not the dead window's.
        let previous = remembered.and_then(|_| {
            self.live_placement
                .get(&window_id)
                .filter(|rec| rec.pid == pid)
                .map(|rec| (rec.pending, rec.order))
        });
        if let Some((_, order)) = previous {
            self.store.restore_order(window_id, order);
        }
        let routed = self.routed_workspace_for(identity);
        let preferred = remembered
            .or(routed)
            .unwrap_or_else(|| self.active_workspace_idx());
        let kind = if remembered.is_some() {
            previous
                .and_then(|(pending, _)| pending)
                .map(|p| p.kind)
                .unwrap_or(PendingKind::Returning)
        } else if routed.is_some() {
            PendingKind::Explicit
        } else {
            PendingKind::Traveling
        };
        log::debug!(
            "WindowCreated tiled: {} pid={} app={:?} bundle={:?} workspace={} display={} origin={:?} size={:?}",
            window_id,
            pid,
            identity.name,
            identity.bundle,
            preferred,
            self.workspaces[preferred].monitor_id,
            self.workspaces[preferred].monitor_origin(),
            self.workspaces[preferred].monitor_size()
        );
        self.store.note_routed(window_id, RoutingVerdict::Tiled);
        self.store
            .note_identity(window_id, self.os.window_identity(window_id));
        // The shared admit funnel executes the routing tail now: a member
        // of a hidden workspace takes the hide placement inside
        // `admit_window` (→ `place_admitted`). The initial
        // hide pass ran inside `new()` while every tree was still empty —
        // everything discovered after it (bootstrap poll, sweep, new
        // windows) needs the same treatment, and gets it from the same
        // seam.
        self.admit_window(window_id, preferred, kind);
        // One in-memory focus for both shapes: `add_window` focused the
        // window if admission tiled it, and this sets `focused_overlay`
        // if it is still waiting — including on a hidden workspace, so
        // the overlay is focused when the user switches there.
        self.workspaces[preferred].focus_window(window_id);
        if self.workspaces[preferred]
            .pending_window(window_id)
            .is_some()
            && self.displays.is_visible(preferred, &self.workspaces)
            && self.workspace_may_act(preferred)
        {
            self.os.focus_window(window_id);
        }
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
        self.admit_discovered_popup(window_id, target);
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
    /// case-insensitively, like the workspace `apps` routing. The bundle
    /// comes from the pass's `AppIdentity` — see that type for why it is
    /// not re-read here.
    pub(super) fn app_is_restricted(&self, bundle: Option<&str>) -> bool {
        self.restricted_apps
            .iter()
            .any(|app| bundle.is_some_and(|b| b.eq_ignore_ascii_case(app)))
    }

    /// Shared admission policy for fresh discoveries and verified restart
    /// recovery. Popup and untracked handling still belongs to discovery;
    /// this predicate answers only whether a window may enter the tile tree.
    pub(super) fn window_is_tileable(
        &self,
        class: Option<WindowClass>,
        identity: &AppIdentity,
    ) -> bool {
        let class_allows_tiles = class == Some(WindowClass::Standard)
            || (class == Some(WindowClass::Untracked) && !self.float_level_overlays);
        class_allows_tiles
            && !self.app_is_float_app(identity)
            && !self.app_is_restricted(identity.bundle.as_deref())
    }

    /// `float_apps` semantics (ADR-0002): every window of these apps is
    /// untracked — never tiled, on top across workspace switches until
    /// closed. Matched case-insensitively against the bundle id OR the
    /// display name (the config doc's promise; games especially have
    /// canonical bundles absent from a real install). Two layers, per the
    /// "permanent for all users" rule: the user's own `float_apps`
    /// config, then the built-in game/client ignore list
    /// (`BUILTIN_GAME_FLOAT_APPS`) that no config can remove — replace
    /// the AppIdentity-threaded call in place of a bare-bundle ask.
    pub(super) fn app_is_float_app(&self, identity: &AppIdentity) -> bool {
        let bundle = identity.bundle.as_deref();
        let name = identity.name.as_deref();
        let matches = |list: &Vec<String>| {
            list.iter().any(|app| {
                bundle.is_some_and(|b| b.eq_ignore_ascii_case(app))
                    || name.is_some_and(|n| n.eq_ignore_ascii_case(app))
            })
        };
        matches(&self.float_apps) || matches(&self.builtin_game_float_apps())
    }

    /// Materialize the built-in list — a function seam rather than a
    /// bare constant use so tests can pin the contents without a global.
    fn builtin_game_float_apps(&self) -> Vec<String> {
        crate::config::BUILTIN_GAME_FLOAT_APPS
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    pub fn on_window_destroyed(&mut self, window_id: WindowId) {
        self.destroy_window_common(window_id, true);
    }

    /// An untrack driven by indirect evidence only — the listing sweep
    /// ("its app no longer lists it"), a Gone write outcome, the
    /// unknown-destroyed probe. No destroyed notification was seen for
    /// this id, so the seatbelt tombstone stays trusted: an app that
    /// temporarily stops listing a live window is exactly the
    /// misjudgment the seatbelt's rediscovery exists for.
    pub(super) fn on_window_unlisted(&mut self, window_id: WindowId) {
        self.destroy_window_common(window_id, false);
    }

    fn destroy_window_common(&mut self, window_id: WindowId, confirmed_destroy: bool) {
        self.remove_member(window_id, confirmed_destroy);
        #[cfg(test)]
        self.assert_identity_invariant();
        self.publish_bar_state();
    }

    /// A window was minimized or its app hidden: drop it from its membership
    /// (tree member or popup) so it stops being laid out, but keep pid
    /// tracking so it can be retiled when it becomes visible again.
    pub fn on_window_hidden(&mut self, window_id: WindowId) {
        if self.park_hidden_member(window_id) {
            #[cfg(test)]
            self.assert_identity_invariant();
            self.publish_bar_state();
        }
    }

    /// A window was deminiaturized or its app unhidden: re-attach it where
    /// it came from — a popup back into its workspace's popup set, a tree
    /// window back into the remembered (or routed) workspace.
    pub fn on_window_shown(&mut self, window_id: WindowId) {
        if self.return_hidden_member(window_id) {
            self.publish_bar_state();
        }
        #[cfg(test)]
        self.assert_identity_invariant();
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
        if !self.park_fullscreen_member(window_id) {
            return false;
        }
        #[cfg(test)]
        self.assert_identity_invariant();
        self.publish_bar_state();
        true
    }

    /// The window left native fullscreen: reveal it through the same funnel
    /// as `on_window_shown` — remembered workspace, waiting there as an overlay when full. Popups never get here (they are exempt from parking).
    pub fn on_fullscreen_exit(&mut self, window_id: WindowId) {
        if !self.return_fullscreen_member(window_id) {
            return;
        }
        self.publish_bar_state();
        #[cfg(test)]
        self.assert_identity_invariant();
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

    pub fn on_window_moved(&mut self, window_id: WindowId, rect: Rect) {
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
        // The event's rect IS the OS read — the notification callback read
        // the full rect on its own element moments ago, on this same
        // thread, and only fired the event because that read succeeded.
        // A re-read here would buy the same values a second time (the
        // coalescing the old comment feared affects notification
        // frequency, never these coords) at two more AX round-trips per
        // move event — dozens per second during a drag.
        self.layout_cache.note_displaced(window_id, Some(rect));
        self.drag.on_moved(
            window_id,
            rect.x,
            rect.y,
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
    ///
    /// Suppressed while a wake resync is pending, like every judgment
    /// sweep: the sweep's poll is the listing evidence that drives both
    /// its halves (untrack and discovery), and through the blackout a
    /// partially answering app risks the first-call-wins routing verdict
    /// being recorded from unreadable class reads (a real popup routed as
    /// a tiled `Standard` sibling — a verdict a later re-read can't fix).
    /// The resync commit runs its own discovery pass from its own poll.
    pub(super) fn sync_untracked_windows(&mut self, pids: impl IntoIterator<Item = i32>) {
        if !self.may_judge("discovery sweep") {
            return;
        }
        let pids: Vec<i32> = pids
            .into_iter()
            .filter(|pid| !self.excluded_pids.contains(pid))
            .collect();
        let listings = self.os.poll_windows_for_pids(&pids);
        // Untrack dead members first, so the discovery layouts below
        // compute targets against a tree that no longer holds them.
        self.untrack_unlisted_windows(&listings);
        // One batched identity ask for the whole pass: the adapter's memo
        // resolves every miss against one running-apps scan, so the pass
        // pays A pids' identities with one NSWorkspace sweep instead of
        // two scans per pid.
        let identities = self.os.app_identities(&pids);
        for (pid, listing) in listings {
            // One identity resolution per pid, not per window: the pass's
            // NSWorkspace scans are paid once and every window of the app
            // routes against the same answer.
            let identity = identities.get(&pid).cloned().unwrap_or_default();
            let listed = StateManager::listing_evidence(&listing).unwrap_or(&[]);
            for window_id in listed.iter().copied() {
                if !self.store.contains(window_id) {
                    log::info!(
                        "sync_untracked: discovered untracked window {} pid {}",
                        window_id,
                        pid
                    );
                    self.route_window_created(window_id, pid, &identity);
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
    /// - **A `NoAnswer` listing is no judgment.** An app AX could not
    ///   answer for is the post-wake blackout signature, not a mass close.
    /// - **A live element outranks the omission.** An answered listing
    ///   that omits a window whose element still answers is a listing
    ///   artifact, not death: macOS Space transitions make apps answer
    ///   without their off-Space windows (visible tiled members
    ///   included), so the omission is judged only after the element
    ///   read says the window no longer exists. The fast close-detector
    ///   is unchanged — a closed window's element is dead, the read
    ///   fails, the untrack proceeds.
    /// - **An answered listing judges, empty included.** Closing the app's
    ///   only window (a PDF in Preview, the 1Password vault) leaves the
    ///   app running with *zero* windows — the app's own "I have none"
    ///   verdict is the fast close-detector for exactly the case the
    ///   destroyed notification is most likely to miss.
    /// - **A member of a hidden workspace is never judged.** macOS omits
    ///   off-Space windows from listings, and only *listed* windows get
    ///   their element refs re-polled — so a hidden member fails both
    ///   discriminators below while perfectly alive (the wake repro:
    ///   sleep → sweep untracks it → rediscovery re-homes it onto the
    ///   active workspace). Tree membership *is* the evidence it is
    ///   alive; its death verdict defers to reveal time, where the
    ///   write path's gone grace owns cleanup.
    /// - **Suppressed while a wake resync is pending** for the same
    ///   reason: during the blackout even live apps list nothing, and a
    ///   partial listing from a recovering app is ambiguous.
    ///
    /// Untracking runs the normal `on_window_destroyed` path (remove,
    /// retile, bar publish), so a closed window's siblings re-tile within
    /// one sweep cycle.
    pub(super) fn untrack_unlisted_windows(&mut self, listings: &[(i32, WindowListing)]) {
        if !self.may_judge("untrack_unlisted") {
            return;
        }
        for (pid, listing) in listings {
            let Some(listed) = StateManager::listing_evidence(listing) else {
                // NoAnswer: the blackout signature, no judgment.
                continue;
            };
            let tracked = self
                .store
                .windows_for(*pid)
                .map(|w| w.to_vec())
                .unwrap_or_default();
            for window_id in tracked {
                if !listed.contains(&window_id) {
                    // A member of a hidden workspace is *supposed* to be
                    // unlisted (off-Space omission) and its element ref is
                    // never refreshed (only listed windows are re-polled) —
                    // both discriminators below can misread it as dead. See
                    // the rule above: tree membership shields it; the
                    // verdict defers to reveal time.
                    if let Some(idx) = self.find_workspace_of_any(window_id) {
                        if !self.displays.is_visible(idx, &self.workspaces) {
                            log::debug!(
                                "untrack_unlisted: window {} pid {} unlisted but a member of hidden workspace {} — alive by membership, not judged",
                                window_id,
                                pid,
                                idx
                            );
                            continue;
                        }
                    }
                    // A window homed to a display showing a non-first
                    // native desktop is off the WM's active surface: its
                    // listing omission is as ambiguous as an off-Space
                    // one, and the element read can fail across the
                    // transition. Defer the death verdict to the return
                    // resync / the write path's gone grace.
                    if !self.window_may_act(window_id) {
                        log::debug!(
                            "untrack_unlisted: window {} pid {} homed to a Space-gated display — not judged",
                            window_id,
                            pid
                        );
                        continue;
                    }
                    // An answered listing that omits a window whose
                    // element still answers is a listing artifact, not a
                    // death. macOS Space transitions make apps answer
                    // `kAXWindows` without their off-Space windows —
                    // visible tiled members included — so judging the
                    // omission untracked the whole desktop on every
                    // Space round-trip, and rediscovery re-routed each
                    // window (config-routed apps back to their entry;
                    // manually-moved windows onto the active workspace —
                    // the user's repro). The element read is the
                    // discriminator: a genuinely closed window's element
                    // is dead, the read fails, and the untrack proceeds
                    // exactly as before. Destroyed notifications, app
                    // termination and the dead-pid sweep still own real
                    // deaths; the fullscreen-parked Space guard is
                    // subsumed by the same rule (a parked window's
                    // element answers while its Space is inactive).
                    if self.os.window_rect(window_id).is_some() {
                        log::debug!(
                            "untrack_unlisted: window {} pid {} unlisted but its element still answers — listing artifact, not a death",
                            window_id,
                            pid
                        );
                        continue;
                    }
                    // Unreadable element: death only once AX is healthy.
                    // The resync commits on any answered listing while
                    // element reads can still be blacked out (its own
                    // commit pass runs inside the settle window it just
                    // opened) — an unreadable element there is the
                    // blackout's signature, not a death. The sweep
                    // re-judges once the settle window passes.
                    if self.in_wake_settle() {
                        log::debug!(
                            "untrack_unlisted: window {} pid {} unlisted and unreadable inside the wake settle window — blackout signature, not a death",
                            window_id,
                            pid
                        );
                        continue;
                    }
                    log::info!(
                        "untrack_unlisted: window {} pid {} no longer listed by its app, untracking",
                        window_id,
                        pid
                    );
                    self.on_window_unlisted(window_id);
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
        let listing = self.os.poll_windows_for_pid(pid);
        if StateManager::listing_evidence(&listing).is_none() {
            // NoAnswer: no judgment (blackout signature).
            return;
        }
        self.untrack_unlisted_windows(&[(pid, listing)]);
    }
    pub fn on_app_launched(&mut self, pid: i32) {
        log::info!("App launched: pid={}", pid);
        if self.excluded_pids.contains(&pid) {
            log::debug!("Skipping excluded app launch: pid={}", pid);
            return;
        }
        self.os.attach_observer(pid);
        if let WindowListing::Answered(windows) = self.os.poll_windows_for_pid(pid) {
            for window_id in windows {
                self.store.register(window_id, pid);
                let _ = self
                    .event_tx
                    .try_send(DaemonEvent::WindowCreated(window_id, pid));
            }
        }
    }

    pub fn on_app_terminated(&mut self, pid: i32) {
        log::info!("App terminated: pid={}", pid);
        self.os.detach_observer(pid);
        // The label cache is pid-keyed and dies with the pid; lingering
        // entries would mislabel a recycled pid forever.
        self.app_labels.remove(&pid);
        let windows = self.store.remove_pid(pid);
        // The seatbelt dies with the pid, like the label cache: a
        // recycled pid must not inherit the dead app's homes.
        self.expire_pid(pid);
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
