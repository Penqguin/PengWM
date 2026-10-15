use pengwm_core::tree::WindowId;

use super::session::WindowPlacement;
use super::StateManager;

/// Owns the placement memory: the durable (bundle, title → workspace name)
/// record of where each tileable window lived when the daemon last quit,
/// consumed through one ask at routing. Membership itself stays
/// single-sourced in the trees (ADR-0003) — the records are restore
/// *input*, never a live second owner: a hit re-homes the window through
/// the normal routing funnel, a miss falls through to config routing and
/// the active workspace exactly as before.
impl StateManager {
    /// Where should this window live, per placement memory? Consumes the
    /// matched record — a hit is once; two identically-titled windows of
    /// one bundle must not inherit each other's home. `None` (no hit) is
    /// not an error: routing falls through to config, then the active
    /// workspace.
    pub(super) fn remembered_workspace(&mut self, window_id: WindowId, pid: i32) -> Option<usize> {
        let bundle = self.os.app_bundle_id(pid);
        let title = self.os.window_title(window_id);
        let pos = self.placement_memory.iter().position(|rec| {
            rec.matches(bundle.as_deref(), title.as_deref())
        })?;
        let rec = self.placement_memory.remove(pos);
        let idx = self
            .workspaces
            .iter()
            .position(|ws| ws.name == rec.workspace)?;
        log::info!(
            "Placement memory: window {} (pid {}, bundle {:?}, title {:?}) re-homed to {} ({})",
            window_id,
            pid,
            bundle,
            title,
            rec.workspace,
            idx
        );
        Some(idx)
    }

    /// One record per tileable window currently in a tree, keys read
    /// live. Called at save time (the quit funnel) so the next boot can
    /// re-home what the user arranged. Windows with no identity evidence
    /// (unreadable bundle AND title) get no record — they restore through
    /// config routing like any other window.
    pub(super) fn capture_placements(&self) -> Vec<WindowPlacement> {
        let mut records = Vec::new();
        for ws in &self.workspaces {
            for wid in ws.all_owned() {
                let Some(pid) = self.store.pid_for(wid) else {
                    continue;
                };
                let bundle = self.os.app_bundle_id(pid);
                let title = self.os.window_title(wid);
                if bundle.is_none() && title.is_none() {
                    continue;
                }
                records.push(WindowPlacement {
                    bundle,
                    title,
                    workspace: ws.name.clone(),
                });
            }
        }
        records
    }

    #[cfg(test)]
    pub(super) fn seed_placement_memory_for_test(&mut self, records: Vec<WindowPlacement>) {
        self.placement_memory = records;
    }

    #[cfg(test)]
    pub(super) fn placement_memory_len_for_test(&self) -> usize {
        self.placement_memory.len()
    }
}
