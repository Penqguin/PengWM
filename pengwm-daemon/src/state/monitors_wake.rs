use std::time::{Duration, Instant};

use pengwm_core::tree::WindowId;

use super::{StateManager, WakeResync};

/// Owns monitor lifecycle and wake resync. Display geometry flows through
/// `DisplaySet::on_added/on_removed/on_resized`; wake rebuilds stale AX
/// state through the existing `OsAdapter` interface — no new adapter method.
impl StateManager {
    pub fn on_monitor_added(&mut self, display_id: u32) {
        let Some(sync) = self
            .displays
            .on_added(display_id, &mut self.workspaces, &*self.os)
        else {
            return;
        };
        for idx in sync.relayout {
            self.apply_layout(idx);
        }
        self.apply_bar_reservation();
        self.publish_bar_state();
    }

    pub fn on_monitor_removed(&mut self, _display_id: u32) {
        let sync = self
            .displays
            .on_removed(_display_id, &mut self.workspaces, &*self.os);
        for idx in sync.hidden {
            self.hide_workspace(idx);
        }
        for idx in sync.relayout {
            if idx < self.workspaces.len() {
                self.apply_layout(idx);
            }
        }
        self.apply_bar_reservation();
        self.publish_bar_state();
    }

    pub fn on_monitor_resized(&mut self, display_id: u32) {
        let affected = self
            .displays
            .on_resized(display_id, &mut self.workspaces, &*self.os);
        for idx in affected {
            self.apply_layout(idx);
        }
        self.apply_bar_reservation();
        self.publish_bar_state();
    }

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
        log::info!("System woke — deferring resync until AX responds");
        self.wake_resync = Some(WakeResync {
            since: now,
            // Probe on the very next tick rather than after one interval.
            last_probe: now - Self::WAKE_PROBE_INTERVAL,
        });
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
            self.wake_resync = None;
        } else if expired {
            log::warn!(
                "AX still unresponsive {:?} after wake — giving up on the resync probe",
                now.duration_since(pending.since)
            );
            self.wake_resync = None;
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

    /// True while a wake resync is still waiting on AX.
    #[cfg(test)]
    pub(super) fn wake_resync_pending(&self) -> bool {
        self.wake_resync.is_some()
    }

    /// One resync attempt. Re-attaches observers and re-polls every app —
    /// polling is what replaces the stale refs in `WindowElementCache` —
    /// then, **only if the poll actually produced windows**, commits the
    /// rest: display geometry, cache clear, routing and re-tile.
    ///
    /// The poll result is the AX liveness probe. Committing behind it is
    /// what keeps a blacked-out wake from clearing good state and writing
    /// through dead elements. `force` commits regardless, for the deadline.
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
        let polled: Vec<(i32, Vec<WindowId>)> = pids
            .iter()
            .map(|&pid| (pid, self.os.poll_windows_for_pid(pid)))
            .collect();
        let any_windows = polled.iter().any(|(_, wins)| !wins.is_empty());
        if !any_windows && !force {
            // Every app reported zero windows. On a machine that was
            // managing windows a moment ago that means AX is still dead,
            // not that every window closed during sleep.
            return false;
        }

        log::info!("System woke — resyncing windows and displays");
        // Refresh workspace geometry from current displays.
        let displays = self.os.active_displays();
        for ws in self.workspaces.iter_mut() {
            if let Some(info) = displays.iter().find(|d| d.id == ws.monitor_id) {
                ws.update_monitor_geometry(info.origin, info.size);
            }
        }
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
        // instead of querying every app a second time.
        for (pid, windows) in polled {
            for window_id in windows {
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
        // Re-tile every visible workspace + bar.
        let visible: Vec<usize> = self.displays.active().values().copied().collect();
        for idx in visible {
            if idx < self.workspaces.len() {
                self.apply_layout(idx);
            }
        }
        self.apply_bar_reservation();
        self.publish_bar_state();
        true
    }
}
