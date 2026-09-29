use std::time::{Duration, Instant};

use super::StateManager;

/// Owns monitor lifecycle and wake resync. Display geometry flows through
/// `DisplaySet::on_added/on_removed/on_resized`; wake rebuilds stale AX
/// state through the existing `OsAdapter` interface — no new adapter method.
impl StateManager {
    pub fn on_monitor_added(&mut self, display_id: u32) {
        if self
            .displays
            .on_added(display_id, &mut self.workspaces, &*self.os)
            .is_none()
        {
            return;
        }
        self.apply_bar_reservation();
        self.publish_bar_state();
    }

    pub fn on_monitor_removed(&mut self, _display_id: u32) {
        self.displays
            .on_removed(_display_id, &mut self.workspaces, &*self.os);
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

    /// Machine woke from sleep: AX refs are stale, display geometry may have
    /// changed, observers may be dead. Resync everything through the existing
    /// `OsAdapter` interface — no new adapter method:
    /// `poll_windows_for_pid` refreshes `WindowElementCache`, `attach_observer`
    /// re-registers, clearing `applied_rects` forces `apply_layout` to rewrite.
    pub fn on_system_woke(&mut self) {
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
        // while post-wake AX is still blacked out — and so does the pin
        // count, so post-wake writes aren't backoff-silenced.
        self.layout_cache.clear_on_wake();
        // Force the background sweep on next tick for windows created mid-sleep.
        self.last_window_sweep = Instant::now() - Duration::from_secs(5);
        self.frontmost_pid = self.os.frontmost_pid();
        // Re-attach observers on every running app, then re-poll through the
        // shared discovery loop. Polling re-inserts into `WindowElementCache`
        // (releasing stale refs) and re-registers moved/destroyed
        // notifications; the loop tiles windows created mid-sleep.
        let pids: Vec<i32> = self
            .os
            .running_app_pids()
            .into_iter()
            .filter(|pid| !self.excluded_pids.contains(pid))
            .collect();
        for pid in &pids {
            self.os.attach_observer(*pid);
        }
        self.sync_untracked_windows(pids);
        // Re-tile every visible workspace + bar.
        let visible: Vec<usize> = self.displays.active().values().copied().collect();
        for idx in visible {
            if idx < self.workspaces.len() {
                self.apply_layout(idx);
            }
        }
        self.apply_bar_reservation();
        self.publish_bar_state();
    }
}
