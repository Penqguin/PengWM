use std::time::{Duration, Instant};

use crate::adapter::WindowListing;
use pengwm_core::tree::WindowId;

use super::layout_writer::LayoutWriteCache;
use super::{StateManager, WakeResync};

/// one owner of the AX blackout: the wake resync lifecycle (arm → probe →
/// commit) and the judgment rules that keep every sweep honest while AX is
/// blacked out. Two rules, stated here once and asked everywhere:
///
/// **No structural judgment while a resync is pending.** During the
/// blackout even live apps list nothing and every attribute reads
/// unreadable, so a sweeping judgment (death, mass-close, fullscreen
/// exit) would act on blackout artifacts — the root cause of the
/// `6f92075` gone-grace regression. Every sweep asks `may_judge`
/// instead of re-deriving the suppression.
///
/// **`NoAnswer` is no judgment.** An app whose poll could not be answered
/// by AX is the blackout signature, never mass-close evidence — every
/// sweep asks `listing_evidence`, which hands the window list through for
/// `Answered` listings (empty included: a closed-only-window app's
/// "zero windows" verdict is admissible) and stops `NoAnswer` cold.
///
/// The wake-pending bit the write-path grace reads is armed and cleared
/// here, at the exact points the resync state changes — the twin bit and
/// its hand-sync disappear with this module.
impl StateManager {
    /// How often `on_tick` re-probes AX while a wake resync is pending.
    /// Fast enough that the desktop settles promptly, slow enough that a
    /// long blackout costs a handful of polls rather than one per tick.
    pub(super) const WAKE_PROBE_INTERVAL: Duration = Duration::from_millis(500);
    /// Give up waiting for AX and resync anyway after this long. A resync
    /// into a still-dead AX is wasteful, but never resyncing is worse —
    /// this bounds the damage of a probe that can never succeed (every
    /// app legitimately windowless, say).
    pub(super) const WAKE_DEADLINE: Duration = Duration::from_secs(20);

    /// Machine woke from sleep. **Arms** the resync; it does not run it.
    ///
    /// `NSWorkspaceDidWake` arrives while the AX subsystem is still blacked
    /// out: `AXUIElementCopyAttributeValue(kAXWindows)` comes back empty for
    /// live apps and every cached element is stale. Resyncing at that moment
    /// is worse than doing nothing — the polls that are supposed to refresh
    /// `WindowElementCache` return nothing, so the stale refs survive, and
    /// the layout writes that follow all fail `kAXErrorInvalidUIElement`,
    /// which `refresh_element` cannot heal either, so every live window
    /// reports `Gone` and starts a 10s death timer. `on_tick` drives
    /// `drive_wake_resync` until AX answers.
    pub fn on_system_woke(&mut self) {
        let now = Instant::now();
        if self.wake_resync.is_some() {
            // Both `NSWorkspaceDidWake` and `ScreensDidWake` fire for one
            // wake; the first arm wins and the second is a no-op.
            log::debug!("System woke — resync already pending");
            return;
        }
        self.arm_wake_resync(now);
    }

    /// The arm point — the one place the resync state and the
    /// write-path's wake-pending grace bit are set together.
    fn arm_wake_resync(&mut self, now: Instant) {
        log::info!("System woke — deferring resync until AX responds");
        // Patient gone grace while the blackout lasts: every layout write
        // this window of time attempts reports live windows as Gone.
        self.layout_cache.note_wake_pending(true);
        self.wake_resync = Some(WakeResync {
            since: now,
            // Probe on the very next tick rather than after one interval.
            last_probe: now - Self::WAKE_PROBE_INTERVAL,
        });
    }

    /// The commit point — clears the resync state and the wake-pending
    /// grace bit together, so the protection never outlives the resync
    /// (deadline-force path included). Opens the post-commit settle
    /// window: listings may answer before window elements do, so the
    /// listing diff distrusts unreadable elements until it passes.
    fn commit_wake_resync(&mut self) {
        self.wake_resync = None;
        self.layout_cache.note_wake_pending(false);
        self.wake_committed_at = Some(Instant::now());
    }

    /// True within the post-commit settle window: the resync committed on
    /// any answered listing while window-element reads can still be blacked
    /// out, so an unreadable element there is still the blackout's
    /// signature, not death. The sweep re-judges once it passes.
    pub(super) fn in_wake_settle(&self) -> bool {
        self.wake_committed_at
            .is_some_and(|t| t.elapsed() < LayoutWriteCache::WAKE_GONE_GRACE)
    }

    /// Tick hook: retry the pending wake resync until AX answers, then
    /// commit it. Returns immediately when nothing is pending.
    pub(super) fn drive_wake_resync(&mut self, now: Instant) {
        let pending = match self.wake_resync {
            Some(p) => p,
            None => return,
        };
        if now.duration_since(pending.last_probe) < Self::WAKE_PROBE_INTERVAL {
            return;
        }
        self.wake_resync = Some(WakeResync {
            last_probe: now,
            ..pending
        });

        let expired = now.duration_since(pending.since) >= Self::WAKE_DEADLINE;
        if self.try_wake_resync(expired) {
            log::info!(
                "Wake resync committed {:?} after wake",
                now.duration_since(pending.since)
            );
            // The commit point cleared the resync state and the
            // wake-pending grace bit, including the deadline force — a
            // resync that never probed successfully still ends.
        } else {
            log::debug!("Wake resync: AX not answering yet, retrying");
        }
    }

    /// Age the probe timer so the next tick re-probes immediately
    /// (mirrors `WAKE_PROBE_INTERVAL` of wall clock passing).
    #[cfg(test)]
    pub(super) fn age_wake_probe_for_test(&mut self) {
        if let Some(pending) = self.wake_resync {
            self.wake_resync = Some(WakeResync {
                last_probe: Instant::now() - Self::WAKE_PROBE_INTERVAL,
                ..pending
            });
        }
    }

    /// Age the wake deadline so the next probe is the forced one.
    #[cfg(test)]
    pub(super) fn age_wake_deadline_for_test(&mut self) {
        if let Some(pending) = self.wake_resync {
            self.wake_resync = Some(WakeResync {
                since: Instant::now() - Self::WAKE_DEADLINE,
                last_probe: Instant::now() - Self::WAKE_PROBE_INTERVAL,
            });
            let _ = pending;
        }
    }

    /// Age the post-commit settle clock past its window (mirrors
    /// `LayoutWriteCache::WAKE_GONE_GRACE` of wall clock passing).
    #[cfg(test)]
    pub(super) fn age_wake_committed_for_test(&mut self) {
        self.wake_committed_at =
            Some(Instant::now() - LayoutWriteCache::WAKE_GONE_GRACE - Duration::from_millis(1));
    }

    /// True while a wake resync is still waiting on AX.
    #[cfg(test)]
    pub(super) fn wake_resync_pending(&self) -> bool {
        self.wake_resync.is_some()
    }

    /// The one funnel every judgment sweep asks: "may I judge?" While a
    /// wake resync is pending the answer is no — logged here with the
    /// sweep's own name so each site keeps its voice but not its own
    /// suppression check.
    pub(super) fn may_judge(&self, sweep: &str) -> bool {
        if self.wake_resync.is_none() {
            return true;
        }
        log::debug!("{}: skipped — wake resync pending", sweep);
        false
    }

    /// The one definition of admissible death evidence: an **answered**
    /// listing (even empty — the closed-only-window signature) is the
    /// app's own verdict on its windows; a `NoAnswer` is the blackout
    /// signature and judges nothing. Previously an empty answer was
    /// inadmissible in every case, which stranded an app whose only
    /// window closed (Preview, 1Password): its "zero windows" verdict is
    /// the fast close-detector and it never got to testify.
    pub(super) fn listing_evidence(listing: &WindowListing) -> Option<&[WindowId]> {
        match listing {
            WindowListing::Answered(listed) => Some(listed),
            WindowListing::NoAnswer => None,
        }
    }

    /// One resync attempt. Re-attaches observers and re-polls every app —
    /// polling is what replaces the stale refs in `WindowElementCache` —
    /// then, **only if the poll actually produced windows**, commits the
    /// rest: display geometry, cache clear, routing and re-tile.
    ///
    /// The poll result is the AX liveness probe (`listing_is_evidence`
    /// over the union of answers). Committing behind it is what keeps a
    /// blacked-out wake from clearing good state and writing through dead
    /// elements. `force` commits regardless, for the deadline.
    ///
    /// The commit point clears the resync state and the wake-pending grace
    /// bit, so the committed polls can be diffed against the tracked set by
    /// the sweep — but the commit itself opens the settle window (listings
    /// answer before window elements do after wake), so the diff inside it
    /// judges nothing: windows that closed during sleep are untracked by
    /// the sweep once the settle window passes and their dead elements are
    /// readable-as-absent under healthy AX.
    ///
    /// Returns whether the resync committed.
    fn try_wake_resync(&mut self, force: bool) -> bool {
        let pids: Vec<i32> = self
            .os
            .running_app_pids()
            .into_iter()
            .filter(|pid| !self.excluded_pids.contains(pid))
            .collect();
        for pid in &pids {
            self.os.attach_observer(*pid);
        }
        // Poll every app first: this refreshes the element cache and tells
        // us whether AX is answering at all.
        let polled = self.os.poll_windows_for_pids(&pids);
        let any_windows = polled.iter().any(|(_, listing)| {
            StateManager::listing_evidence(listing).is_some_and(|wins| !wins.is_empty())
        });
        if !any_windows && !force {
            // No app listed any window. On a machine that was managing
            // windows a moment ago that means AX is not answering yet,
            // not that every window closed during sleep — wait for the
            // next probe unless the deadline forces the commit.
            return false;
        }

        // Commit point. Drop the pending resync first so the fast grace
        // resumes — on the deadline-force path too, per the "nothing
        // outlives the resync" boundary — and open the settle window: the
        // listing diff inside the commit pass judges nothing (element
        // reads can still be blacked out); the sweep re-judges after it.
        self.commit_wake_resync();

        log::info!("System woke — resyncing windows and displays");
        // Heal the topology first: displays may have come or gone during
        // sleep. The same one pass the tick and the hotplug hints use —
        // orphans remap to primary, seats settle, geometry self-heals —
        // instead of the geometry-only refresh that used to skip workspaces
        // whose display died in sleep (their stale origin/size re-tiled
        // windows into no-man's land). The final re-tile below covers
        // `relayout`; `hidden`/`repark` run here.
        let tsync = self.displays.sync(&mut self.workspaces, &*self.os);
        self.park_topology_changes(tsync.hidden, tsync.repark);
        // Drop the layout-write cache: stale AX refs + moved displays mean
        // every "already applied" entry is a lie after sleep. The gone
        // grace restarts too — pre-sleep misses must not kill windows
        // while post-wake AX is still settling — and so does the pin
        // count, so post-wake writes aren't backoff-silenced.
        self.layout_cache.clear_on_wake();
        // Force the background sweep on next tick for windows created mid-sleep.
        self.last_window_sweep = Instant::now() - Duration::from_secs(5);
        self.frontmost_pid = self.os.frontmost_pid();
        // Tile anything that appeared during sleep, reusing the poll above
        // instead of querying every app a second time. Windows that closed
        // during sleep stay in the tree for this pass — the settle window
        // suspends the listing diff's element probe — and drain via the
        // sweep's diff or the write path's gone grace within the settle
        // window; both own their cleanup under healthy AX.
        for (pid, listing) in polled {
            let listed = StateManager::listing_evidence(&listing).unwrap_or(&[]);
            for window_id in listed.iter().copied() {
                if !self.store.contains(window_id) {
                    log::info!(
                        "wake resync: discovered untracked window {} pid {}",
                        window_id,
                        pid
                    );
                    self.on_window_created(window_id, pid);
                }
            }
        }
        // Re-tile every visible workspace.
        let visible: Vec<usize> = self.displays.active().values().copied().collect();
        for idx in visible {
            if idx < self.workspaces.len() {
                self.apply_layout(idx);
            }
        }
        // Everything re-stacked after sleep; stacking is globally unset, so
        // restore it fully: popups of the visible workspaces and the
        // untracked overlays (ADR-0002).
        self.reveal_stack(None);
        self.publish_bar_state();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter_test::Fault;
    use crate::state::tests::common::setup_with_handle;

    #[test]
    fn an_unreadable_element_inside_the_wake_settle_window_is_not_a_death() {
        // The regression: the resync commits on any answered listing, but
        // window-element reads can still be blacked out when it does — the
        // listing diff's element probe read None for a live window and
        // untracked it, and tiling after sleep broke. Inside the settle
        // window an unreadable element is the blackout's signature, not a
        // death; the sweep re-judges once AX is healthy.
        let (mut sm, handle) = setup_with_handle(1);
        sm.on_window_created(100, 42);
        sm.on_window_created(200, 42);

        // Sleep + wake: the deadline forces the commit.
        sm.on_system_woke();
        sm.age_wake_deadline_for_test();
        sm.age_wake_probe_for_test();
        sm.drive_wake_resync(Instant::now());
        assert!(
            !sm.wake_resync_pending(),
            "precondition: the resync committed"
        );

        // The artifact shape: the app answers without the window AND the
        // element read fails — the fake models both with one unlist. The
        // window is also genuinely dead here, so its writes fail Gone
        // (the fake would otherwise resurrect the rect on the commit
        // pass's re-tile write).
        handle.set_fault(100, Fault::Gone);
        handle.unlist_window(42, 100);
        sm.force_window_sweep_for_test();
        sm.on_tick();
        assert!(
            sm.store.contains(100),
            "an unreadable element inside the wake settle window was judged dead"
        );

        // Outside the settle window the same shape is genuinely dead —
        // the fast close-detector must still fire.
        sm.age_wake_committed_for_test();
        sm.force_window_sweep_for_test();
        sm.on_tick();
        assert!(
            !sm.store.contains(100),
            "outside the settle window the untrack must proceed"
        );
    }

    #[test]
    fn may_judge_gates_on_the_resync_state() {
        let (mut sm, _handle) = setup_with_handle(1);
        assert!(sm.may_judge("test sweep"), "no blackout — judgment is free");
        sm.on_system_woke();
        assert!(
            !sm.may_judge("test sweep"),
            "while a resync is pending, no structural judgment"
        );
        // Commit (deadline force) frees the gate again — nothing outlives
        // the resync.
        sm.age_wake_deadline_for_test();
        sm.age_wake_probe_for_test();
        sm.drive_wake_resync(Instant::now());
        assert!(sm.may_judge("test sweep"));
    }

    #[test]
    fn no_answer_is_never_evidence_but_an_answered_empty_listing_is() {
        use crate::adapter::WindowListing;
        assert!(StateManager::listing_evidence(&WindowListing::NoAnswer).is_none());
        assert!(StateManager::listing_evidence(&WindowListing::Answered(vec![])).is_some());
        assert!(StateManager::listing_evidence(&WindowListing::Answered(vec![10, 20])).is_some());
    }
}
