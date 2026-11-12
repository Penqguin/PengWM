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
    ///
    /// Suppressed while a wake resync is pending: the *detection* is a cheap
    /// CG query that blackouts don't touch, but the repair is a structural
    /// judgment — membership and the visible arrangement must survive the
    /// change (ADR-0006), so the heal is deferred to the resync commit's
    /// own `displays.sync`, not run mid-blackout.
    ///
    /// Topology changes never re-home members (ADR-0006): the heal is
    /// incremental — reseat + relayout + park — and never calls `wm_reset`.
    /// Reset is reserved for structural config changes or an explicit
    /// command.
    pub fn run_topology_sync(&mut self) {
        if !self.may_judge("topology sync") {
            return;
        }
        // Any actual topology change is healed incrementally: reseat the
        // affected seats, re-park newly-hidden workspaces, re-lay out the
        // visible set. Membership and the visible arrangement are
        // PRESERVED — a topology change never re-homes a member
        // (ADR-0006). The seatbelt (live_placement) survives, so a
        // window the store lost comes back to where it lives this session.
        // Reset (wm_reset) is reserved for structural config changes or
        // an explicit command — never an automatic topology reaction.
        let previous: Vec<_> = self.workspaces.iter().map(|ws| ws.monitor_id).collect();
        let mut sync = self.displays.sync(&mut self.workspaces, &*self.os);
        if let Some(target) = self.relocate_detached_overflow(&previous) {
            if !sync.relayout.contains(&target) {
                sync.relayout.push(target);
            }
        }
        if sync.is_noop() {
            return;
        }
        self.retain_workspace_pills();
        log::info!(
            "topology changed: shown {:?}, hidden {:?}, re-parked {:?}, re-laid {:?} — incremental heal",
            sync.shown,
            sync.hidden,
            sync.repark,
            sync.relayout
        );
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
