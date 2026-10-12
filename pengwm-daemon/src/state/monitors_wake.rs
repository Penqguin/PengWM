use super::StateManager;

/// Owns monitor lifecycle and the topology-sync executor. **All** topology
/// healing flows through `DisplaySet::sync` (one planner, one answer type);
/// this file executes what it answers. Display geometry events
/// (`MonitorAdded`/`MonitorRemoved`/`MonitorResized`) are hints only — the
/// sync re-reads the live display list itself, and the tick calls the same
/// funnel so a lost or out-of-order event heals within one sweep.
/// Wake resync and the blackout judgment rules live in `state::blackout`.
impl StateManager {
    /// Execute a topology answer's park half: newly-hidden workspaces off-
    /// screen, and already-hidden workspaces whose geometry healed re-parked
    /// at the fresh coordinates. The two passes (`run_topology_sync`, the
    /// wake resync) share one shape — this is it.
    pub(super) fn park_topology_changes(&mut self, hidden: Vec<usize>, repark: Vec<usize>) {
        for idx in hidden {
            self.hide_workspace(idx);
        }
        for idx in repark {
            if idx < self.workspaces.len() {
                self.hide_workspace(idx);
            }
        }
    }

    /// Run one complete topology healing pass and execute what it answered:
    /// drop the write cache's "already applied" evidence (displays came and
    /// went), park newly seatless workspaces, re-park pre-hidden workspaces
    /// whose geometry healed, re-lay out the visible set, publish the bar.
    /// A no-op answer runs no machinery at all — cheap on the tick.
    pub fn run_topology_sync(&mut self) {
        let sync = self.displays.sync(&mut self.workspaces, &*self.os);
        if sync.is_noop() {
            return;
        }
        log::info!(
            "topology sync: shown {:?}, hidden {:?}, re-parked {:?}, re-laid {:?}",
            sync.shown,
            sync.hidden,
            sync.repark,
            sync.relayout
        );
        // Every cached "already applied" rect may name coordinates nothing
        // serves anymore (`clear_topology` keeps pending gone-grace timers —
        // a topology change is not death evidence).
        self.layout_cache.clear_topology();
        self.park_topology_changes(sync.hidden, sync.repark);
        for idx in sync.relayout {
            if idx < self.workspaces.len() {
                self.apply_layout(idx);
            }
        }
        self.publish_bar_state();
    }

    pub fn on_monitor_added(&mut self, _display_id: u32) {
        self.run_topology_sync();
    }

    pub fn on_monitor_removed(&mut self, _display_id: u32) {
        self.run_topology_sync();
    }

    pub fn on_monitor_resized(&mut self, _display_id: u32) {
        self.run_topology_sync();
    }
}
