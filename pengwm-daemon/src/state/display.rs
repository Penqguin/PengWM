use std::collections::HashMap;

use crate::config::WorkspaceEntry;
use pengwm_core::workspace::Workspace;

use crate::adapter::{DisplayInfo, OsAdapter};

/// Owns the display ↔ workspace registry: `active_workspaces` (which flat
/// workspace index is visible per monitor) and the `workspace_entries` that
/// define the named workspace set cloned per display. `Vec<Workspace>` itself
/// stays on `StateManager` and is borrowed per call — so `HiddenTracker`,
/// `BarReserve` and `DragState` don't need to reach through a registry.
pub struct DisplaySet {
    active: HashMap<u32, usize>,
    entries: Vec<WorkspaceEntry>,
}

impl DisplaySet {
    pub fn new(entries: Vec<WorkspaceEntry>) -> Self {
        Self {
            active: HashMap::new(),
            entries,
        }
    }

    pub fn active(&self) -> &HashMap<u32, usize> {
        &self.active
    }

    pub fn active_mut(&mut self) -> &mut HashMap<u32, usize> {
        &mut self.active
    }

    pub fn entries(&self) -> &[WorkspaceEntry] {
        &self.entries
    }

    pub fn set_entries(&mut self, entries: Vec<WorkspaceEntry>) {
        self.entries = entries;
    }

    /// Initialize `workspaces` + `active` from the current displays. Called
    /// once from `StateManager::new`. Returns the number of workspaces created
    /// (for tests).
    pub fn init_workspaces(
        &mut self,
        workspaces: &mut Vec<Workspace>,
        displays: &[DisplayInfo],
    ) {
        workspaces.clear();
        self.active.clear();
        for display in displays {
            let base = workspaces.len();
            self.active.insert(display.id, base);
            for entry in &self.entries {
                workspaces.push(Workspace::new(
                    entry.name.clone(),
                    display.id,
                    display.origin,
                    display.size,
                ));
            }
        }
    }

    /// Handle `MonitorAdded`: clone the workspace entries onto the new display.
    /// Returns the new workspace indices (for the caller to reserve bar + publish).
    pub fn on_added(
        &mut self,
        display_id: u32,
        workspaces: &mut Vec<Workspace>,
        os: &dyn OsAdapter,
    ) -> Option<Vec<usize>> {
        let info = os
            .active_displays()
            .into_iter()
            .find(|d| d.id == display_id)?;
        let base = workspaces.len();
        self.active.insert(display_id, base);
        let mut created = Vec::new();
        for entry in &self.entries {
            workspaces.push(Workspace::new(
                entry.name.clone(),
                display_id,
                info.origin,
                info.size,
            ));
            created.push(workspaces.len() - 1);
        }
        Some(created)
    }

    /// Handle `MonitorRemoved`: reassign orphaned workspaces to the primary
    /// display, retain only those on still-active displays, and repair `active`.
    /// Returns the indices that need re-layout (caller does bar reservation +
    /// publish). Keeps behavior identical to the old `StateManager::on_monitor_removed`.
    pub fn on_removed(
        &mut self,
        removed_id: u32,
        workspaces: &mut Vec<Workspace>,
        os: &dyn OsAdapter,
    ) {
        let primary = os.primary_display_id();
        let primary_origin = os
            .active_displays()
            .into_iter()
            .find(|d| d.id == primary)
            .map(|d| d.origin)
            .unwrap_or((0, 0));
        for ws in workspaces.iter_mut() {
            if ws.monitor_id == removed_id {
                ws.monitor_id = primary;
                ws.set_monitor_origin(primary_origin);
            }
        }
        let active_displays = os.active_displays();
        workspaces.retain(|ws| active_displays.iter().any(|d| d.id == ws.monitor_id));
        if workspaces.is_empty() {
            workspaces.push(Workspace::new(
                "ws-1".into(),
                primary,
                primary_origin,
                (1920, 1080),
            ));
        }
        self.active.retain(|_, idx| *idx < workspaces.len());
        if self.active.is_empty() {
            self.active.insert(workspaces[0].monitor_id, 0);
        }
    }

    /// Handle `MonitorResized`: update geometry on matching workspaces.
    /// Returns the indices that need `apply_layout`.
    pub fn on_resized(
        &self,
        display_id: u32,
        workspaces: &mut [Workspace],
        os: &dyn OsAdapter,
    ) -> Vec<usize> {
        let Some(info) = os
            .active_displays()
            .into_iter()
            .find(|d| d.id == display_id)
        else {
            return Vec::new();
        };
        let mut affected = Vec::new();
        for (i, ws) in workspaces.iter_mut().enumerate() {
            if ws.monitor_id == display_id {
                ws.update_monitor_geometry(info.origin, info.size);
                affected.push(i);
            }
        }
        affected
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::DisplayInfo;
    use crate::adapter_test::TestAdapter;
    use pengwm_core::workspace::Workspace;

    fn test_displays_one() -> Vec<DisplayInfo> {
        vec![DisplayInfo {
            id: 1,
            origin: (0, 0),
            size: (1920, 1080),
        }]
    }

    fn test_displays_two() -> Vec<DisplayInfo> {
        vec![
            DisplayInfo {
                id: 1,
                origin: (0, 0),
                size: (1920, 1080),
            },
            DisplayInfo {
                id: 2,
                origin: (1920, 0),
                size: (1920, 1080),
            },
        ]
    }

    fn entries_two() -> Vec<WorkspaceEntry> {
        vec![
            WorkspaceEntry {
                name: "a".into(),
                apps: vec![],
            },
            WorkspaceEntry {
                name: "b".into(),
                apps: vec![],
            },
        ]
    }

    #[test]
    fn init_workspaces_creates_per_display_sets() {
        let mut ds = DisplaySet::new(entries_two());
        let mut wss = Vec::new();
        ds.init_workspaces(&mut wss, &test_displays_two());
        assert_eq!(wss.len(), 4);
        assert!(wss[..2].iter().all(|ws| ws.monitor_id == 1));
        assert!(wss[2..].iter().all(|ws| ws.monitor_id == 2));
        assert_eq!(ds.active.get(&1), Some(&0));
        assert_eq!(ds.active.get(&2), Some(&2));
    }

    #[test]
    fn on_added_clones_entries_for_new_display() {
        let mut ds = DisplaySet::new(entries_two());
        let mut wss: Vec<Workspace> = Vec::new();
        ds.init_workspaces(&mut wss, &test_displays_one());
        assert_eq!(wss.len(), 2);

        let mut adapter = TestAdapter::new();
        adapter.displays = test_displays_two();
        let created = ds.on_added(2, &mut wss, &adapter).unwrap();
        assert_eq!(created, vec![2, 3]);
        assert_eq!(wss.len(), 4);
        assert_eq!(ds.active.get(&2), Some(&2));
    }

    #[test]
    fn on_removed_reassigns_and_cleans() {
        let mut ds = DisplaySet::new(entries_two());
        let mut wss: Vec<Workspace> = Vec::new();
        ds.init_workspaces(&mut wss, &test_displays_two());
        // Simulate display 2 removed — only display 1 remains. Orphaned
        // workspaces are reassigned to primary, so all 4 are kept (migrated).
        let mut adapter = TestAdapter::new();
        adapter.displays = test_displays_one();
        ds.on_removed(2, &mut wss, &adapter);
        assert!(wss.iter().all(|ws| ws.monitor_id == 1));
        assert_eq!(wss.len(), 4);
    }

    #[test]
    fn on_resized_updates_geometry_and_returns_affected() {
        let mut ds = DisplaySet::new(entries_two());
        let mut wss: Vec<Workspace> = Vec::new();
        ds.init_workspaces(&mut wss, &test_displays_two());
        let mut adapter = TestAdapter::new();
        adapter.displays = vec![
            DisplayInfo {
                id: 1,
                origin: (0, 0),
                size: (2560, 1440),
            },
            DisplayInfo {
                id: 2,
                origin: (2560, 0),
                size: (1920, 1080),
            },
        ];
        let affected = ds.on_resized(1, &mut wss, &adapter);
        assert_eq!(affected, vec![0, 1]);
        assert_eq!(wss[0].monitor_size(), (2560, 1440));
        assert_eq!(wss[2].monitor_size(), (1920, 1080));
    }
}
