use std::collections::{BTreeMap, HashMap};

use crate::config::WorkspaceEntry;
use pengwm_core::tree::{Direction, WindowId};
use pengwm_core::workspace::Workspace;

use crate::adapter::{DisplayInfo, OsAdapter};

/// Owns the display ↔ workspace registry: `active_workspaces` (which flat
/// workspace index is visible per monitor) and the `workspace_entries` that
/// define the named workspace set cloned per display. `Vec<Workspace>` itself
/// stays on `StateManager` and is borrowed per call — so `WindowStore`,
/// `BarReserve` and `DragState` don't need to reach through a registry.
/// `active` is a `BTreeMap` so iteration order is deterministic (no HashMap
/// random fallback in `active_workspace_idx`). Also owns the routing policy
/// (`max_tiles` + per-monitor overflow/routing) so callers have one interface
/// for "workspaces on monitor X" (Q1 deepening).
pub struct DisplaySet {
    active: BTreeMap<u32, usize>,
    entries: Vec<WorkspaceEntry>,
    max_tiles: usize,
}

impl DisplaySet {
    pub fn new(entries: Vec<WorkspaceEntry>) -> Self {
        Self {
            active: BTreeMap::new(),
            entries,
            max_tiles: 4,
        }
    }

    pub fn with_max_tiles(entries: Vec<WorkspaceEntry>, max_tiles: usize) -> Self {
        Self {
            active: BTreeMap::new(),
            entries,
            max_tiles: max_tiles.max(1),
        }
    }

    pub fn active(&self) -> &BTreeMap<u32, usize> {
        &self.active
    }

    pub fn active_mut(&mut self) -> &mut BTreeMap<u32, usize> {
        &mut self.active
    }

    pub fn entries(&self) -> &[WorkspaceEntry] {
        &self.entries
    }

    pub fn set_entries(&mut self, entries: Vec<WorkspaceEntry>) {
        self.entries = entries;
    }

    pub fn max_tiles(&self) -> usize {
        self.max_tiles
    }

    pub fn set_max_tiles(&mut self, n: usize) {
        self.max_tiles = n.max(1);
    }

    // -- per-monitor helpers (Q1: one interface for monitor-local workspace sets) --

    /// Flat indices of workspaces on `monitor`.
    pub fn workspaces_on(&self, monitor: u32, workspaces: &[Workspace]) -> Vec<usize> {
        workspaces
            .iter()
            .enumerate()
            .filter(|(_, ws)| ws.monitor_id == monitor)
            .map(|(i, _)| i)
            .collect()
    }

    /// Flat index of the visible workspace on `monitor`, if any.
    pub fn visible_on(&self, monitor: u32) -> Option<usize> {
        self.active.get(&monitor).copied()
    }

    /// Whether flat workspace `idx` is the visible one on its monitor.
    /// Moved up from `StateManager::is_workspace_visible` so visibility
    /// reads sit with the `active` map they query (locality).
    pub fn is_visible(&self, idx: usize, workspaces: &[Workspace]) -> bool {
        if idx >= workspaces.len() {
            return false;
        }
        let mon = workspaces[idx].monitor_id;
        self.active.get(&mon).copied() == Some(idx)
    }

    /// Visible workspace on `monitor`, falling back to the first workspace
    /// owned by that monitor. Unifies the active-or-first fallback
    /// duplicated in focus/move-to-display (locality).
    pub fn visible_or_first(&self, monitor: u32, workspaces: &[Workspace]) -> Option<usize> {
        match self.active.get(&monitor).copied() {
            Some(idx) if idx < workspaces.len() => Some(idx),
            _ => workspaces.iter().position(|ws| ws.monitor_id == monitor),
        }
    }

    /// Resolve a 1-based per-monitor workspace id `n` (from `Command::Workspace`)
    /// into a flat workspace index, using `current_idx`'s monitor.
    pub fn resolve_workspace(
        &self,
        current_idx: usize,
        n: usize,
        workspaces: &[Workspace],
    ) -> Option<usize> {
        if n == 0 || current_idx >= workspaces.len() {
            return None;
        }
        let monitor = workspaces[current_idx].monitor_id;
        let on_mon = self.workspaces_on(monitor, workspaces);
        if n > on_mon.len() {
            return None;
        }
        Some(on_mon[n - 1])
    }

    /// First workspace after `start` (wrapping within the same monitor) with
    /// room for another window. `None` when every workspace on that monitor is
    /// at capacity. Absorbed from `capacity::next_with_room` (deleted).
    pub fn next_with_room(&self, workspaces: &[Workspace], start: usize) -> Option<usize> {
        if start >= workspaces.len() {
            return None;
        }
        let monitor = workspaces[start].monitor_id;
        let indices: Vec<usize> = workspaces
            .iter()
            .enumerate()
            .filter(|(_, ws)| ws.monitor_id == monitor)
            .map(|(i, _)| i)
            .collect();
        let pos = indices.iter().position(|&idx| idx == start)?;
        let n = indices.len();
        for offset in 1..n {
            let idx = indices[(pos + offset) % n];
            if workspaces[idx].window_count() < self.max_tiles {
                return Some(idx);
            }
        }
        None
    }

    /// Overflow redirect: `target` itself when it has room, else the next
    /// workspace on the same monitor with room. `None` when the monitor is
    /// full. Unifies the three redirect sites (create / move / move-display)
    /// so capacity policy lives with `max_tiles` (locality).
    pub fn target_with_room(&self, workspaces: &[Workspace], target: usize) -> Option<usize> {
        if target >= workspaces.len() {
            return None;
        }
        if workspaces[target].window_count() < self.max_tiles {
            return Some(target);
        }
        self.next_with_room(workspaces, target)
    }

    /// Decide a focused-window move onto `target`: `target` itself with room,
    /// else the overflow redirect on the same monitor. `None` when the move
    /// is a no-op (`from == target`) or no workspace on the monitor has room.
    /// The caller moves the window and re-layouts visible ends — DisplaySet
    /// answers, commands execute.
    pub fn plan_move(
        &self,
        workspaces: &[Workspace],
        from: usize,
        target: usize,
    ) -> Option<MoveDecision> {
        if from == target || from >= workspaces.len() {
            return None;
        }
        self.target_with_room(workspaces, target)
            .map(|to| MoveDecision { from, to })
    }

    /// Visible-or-first workspace on the display in `direction` from
    /// `current`'s monitor. `None` when there is no display that way, or it
    /// owns no workspace. Answers the first half of both cross-display
    /// commands so neither re-branches on direction + fallback.
    pub fn direction_target(
        &self,
        workspaces: &[Workspace],
        current: usize,
        direction: Direction,
        os: &dyn OsAdapter,
    ) -> Option<usize> {
        if current >= workspaces.len() {
            return None;
        }
        let current_mon = workspaces[current].monitor_id;
        let target_mon = self.display_in_direction(current_mon, direction, os)?;
        self.visible_or_first(target_mon, workspaces)
    }

    /// Heuristic for "which workspace is active": the workspace that contains a
    /// window belonging to `frontmost_pid`, mapped through `active` per
    /// monitor. Falls back to arbitrary `active` entry. Absorbed from `Router`.
    pub fn active_workspace_idx(
        &self,
        workspaces: &[Workspace],
        pid_to_windows: &HashMap<i32, Vec<WindowId>>,
        frontmost_pid: Option<i32>,
    ) -> usize {
        if let Some(pid) = frontmost_pid {
            if let Some(windows) = pid_to_windows.get(&pid) {
                for &window_id in windows {
                    for ws in workspaces {
                        if ws.find_window(window_id).is_some() {
                            if let Some(&idx) = self.active.get(&ws.monitor_id) {
                                if idx < workspaces.len() {
                                    return idx;
                                }
                            }
                        }
                    }
                }
            }
        }
        self.active.values().next().copied().unwrap_or(0)
    }

    /// Name of the configured workspace `pid`'s app is assigned to, matched
    /// case-insensitively against bundle id first, then app display name.
    pub fn configured_workspace_name_for_pid(&self, pid: i32, os: &dyn OsAdapter) -> Option<&str> {
        if self.entries.is_empty() {
            return None;
        }
        let bundle = os.app_bundle_id(pid);
        let app_name = os.app_name(pid);
        self.entries
            .iter()
            .find(|entry| {
                entry.apps.iter().any(|app| {
                    bundle
                        .as_deref()
                        .is_some_and(|b| b.eq_ignore_ascii_case(app))
                        || app_name
                            .as_deref()
                            .is_some_and(|n| n.eq_ignore_ascii_case(app))
                })
            })
            .map(|e| e.name.as_str())
    }

    /// Flat workspace index a new window from `pid` should land in: the
    /// configured workspace for the app on the active monitor. `None` when the
    /// app isn't assigned. Absorbed from `Router`.
    pub fn routed_workspace_idx(
        &self,
        pid: i32,
        workspaces: &[Workspace],
        active_idx: usize,
        os: &dyn OsAdapter,
    ) -> Option<usize> {
        if active_idx >= workspaces.len() {
            return None;
        }
        let monitor = workspaces[active_idx].monitor_id;
        let name = self.configured_workspace_name_for_pid(pid, os)?;
        workspaces
            .iter()
            .position(|ws| ws.name == name && ws.monitor_id == monitor)
    }

    /// Closest display in `direction` from `from`, by center-to-center vector.
    /// Absorbed from `StateManager::find_display_in_direction` so all
    /// geometry queries flow through one interface (locality).
    pub fn display_in_direction(
        &self,
        from: u32,
        direction: Direction,
        os: &dyn OsAdapter,
    ) -> Option<u32> {
        let displays = os.active_displays();
        let from_disp = displays.iter().find(|d| d.id == from)?;
        let from_cx = from_disp.origin.0 as f64 + from_disp.size.0 as f64 / 2.0;
        let from_cy = from_disp.origin.1 as f64 + from_disp.size.1 as f64 / 2.0;
        let mut best: Option<(u32, f64)> = None;
        for d in &displays {
            if d.id == from {
                continue;
            }
            let cx = d.origin.0 as f64 + d.size.0 as f64 / 2.0;
            let cy = d.origin.1 as f64 + d.size.1 as f64 / 2.0;
            let dx = cx - from_cx;
            let dy = cy - from_cy;
            let is_match = match direction {
                Direction::Left => dx < 0.0 && dx.abs() >= dy.abs(),
                Direction::Right => dx > 0.0 && dx.abs() >= dy.abs(),
                Direction::Up => dy < 0.0 && dy.abs() > dx.abs(),
                Direction::Down => dy > 0.0 && dy.abs() > dx.abs(),
            };
            if !is_match {
                continue;
            }
            let dist = dx * dx + dy * dy;
            if best.map(|(_, bd)| dist < bd).unwrap_or(true) {
                best = Some((d.id, dist));
            }
        }
        best.map(|(id, _)| id)
    }

    /// Initialize `workspaces` + `active` from the current displays. Called
    /// once from `StateManager::new`. Returns the number of workspaces created
    /// (for tests).
    pub fn init_workspaces(&mut self, workspaces: &mut Vec<Workspace>, displays: &[DisplayInfo]) {
        workspaces.clear();
        self.active.clear();
        // Per-monitor affinity: entries with `monitor` set only appear on that
        // monitor; `None` means cloned to every monitor (back-compat).
        for display in displays {
            let mut base_for_display: Option<usize> = None;
            for entry in &self.entries {
                if !Self::entry_applies_to_display(entry, display) {
                    continue;
                }
                if base_for_display.is_none() {
                    base_for_display = Some(workspaces.len());
                }
                workspaces.push(Workspace::new(
                    entry.name.clone(),
                    display.id,
                    display.origin,
                    display.size,
                ));
            }
            if let Some(base) = base_for_display {
                self.active.insert(display.id, base);
            }
        }
        // If no workspace matched any display (e.g. all had affinity for a
        // disconnected monitor), fall back to cloning `None`-affinity or all
        // entries onto the first display so we never start empty.
        if workspaces.is_empty() && !displays.is_empty() {
            let first = &displays[0];
            for entry in &self.entries {
                workspaces.push(Workspace::new(
                    entry.name.clone(),
                    first.id,
                    first.origin,
                    first.size,
                ));
            }
            if !workspaces.is_empty() {
                self.active.insert(first.id, 0);
            }
        }
    }

    pub fn entry_applies_to_display(entry: &WorkspaceEntry, display: &DisplayInfo) -> bool {
        match &entry.monitor {
            None => true,
            Some(crate::config::MonitorRef::Index(id)) => *id == display.id,
            Some(crate::config::MonitorRef::Name(name)) => {
                // DisplayInfo has no name yet; allow numeric string match for
                // back-compat. A real display-name resolver can be added once
                // OsAdapter exposes display names.
                name == &display.id.to_string()
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
        let mut created = Vec::new();
        let mut first_for_display: Option<usize> = None;
        for entry in &self.entries {
            if !Self::entry_applies_to_display(entry, &info) {
                continue;
            }
            if first_for_display.is_none() {
                first_for_display = Some(workspaces.len());
            }
            workspaces.push(Workspace::new(
                entry.name.clone(),
                display_id,
                info.origin,
                info.size,
            ));
            created.push(workspaces.len() - 1);
        }
        if created.is_empty() {
            return None;
        }
        self.active.insert(display_id, first_for_display.unwrap());
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

/// A decided focused-window move: the `from` workspace loses the window,
/// the `to` workspace gains it. Answered by `DisplaySet`, executed by the
/// caller (tree mutation + visible-end re-layouts stay with `StateManager`).
pub struct MoveDecision {
    pub from: usize,
    pub to: usize,
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
                monitor: None,
                autostart: vec![],
            },
            WorkspaceEntry {
                name: "b".into(),
                apps: vec![],
                monitor: None,
                autostart: vec![],
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

    // -- routing helpers (absorbed from Router + capacity) --

    #[test]
    fn next_with_room_wraps_within_monitor() {
        let ds = DisplaySet::with_max_tiles(vec![], 2);
        let mut wss = vec![
            Workspace::new("a".into(), 1, (0, 0), (1920, 1080)),
            Workspace::new("b".into(), 1, (0, 0), (1920, 1080)),
            Workspace::new("c".into(), 2, (1920, 0), (1920, 1080)),
        ];
        wss[0].add_window(1, None);
        wss[0].add_window(2, None);
        assert_eq!(ds.next_with_room(&wss, 0), Some(1));
        // From 1 the next on same monitor is 0 which is full → None
        assert_eq!(ds.next_with_room(&wss, 1), None);
    }

    #[test]
    fn next_with_room_ignores_other_monitor() {
        let ds = DisplaySet::with_max_tiles(vec![], 2);
        let mut wss = vec![
            Workspace::new("a".into(), 1, (0, 0), (1920, 1080)),
            Workspace::new("b".into(), 2, (1920, 0), (1920, 1080)),
        ];
        wss[0].add_window(1, None);
        wss[0].add_window(2, None);
        assert_eq!(ds.next_with_room(&wss, 0), None);
    }

    #[test]
    fn resolve_workspace_on_monitor() {
        let mut ds = DisplaySet::new(entries_two());
        let mut wss = Vec::new();
        ds.init_workspaces(&mut wss, &test_displays_two());
        // wss: [a@1,b@1,a@2,b@2] ; from idx 0 (monitor 1), n=2 → flat 1
        assert_eq!(ds.resolve_workspace(0, 2, &wss), Some(1));
        assert_eq!(ds.resolve_workspace(0, 1, &wss), Some(0));
        // from monitor 2
        assert_eq!(ds.resolve_workspace(2, 2, &wss), Some(3));
        assert_eq!(ds.resolve_workspace(0, 3, &wss), None);
    }

    #[test]
    fn workspaces_on_filters() {
        let mut ds = DisplaySet::new(entries_two());
        let mut wss = Vec::new();
        ds.init_workspaces(&mut wss, &test_displays_two());
        assert_eq!(ds.workspaces_on(1, &wss), vec![0, 1]);
        assert_eq!(ds.workspaces_on(2, &wss), vec![2, 3]);
    }

    #[test]
    fn configured_name_matches_bundle_case_insensitive() {
        let ds = DisplaySet::new(crate::config::default_workspaces());
        let adapter = TestAdapter::new();
        adapter.inject_bundle_id(10, "com.google.Chrome".into());
        let name = ds.configured_workspace_name_for_pid(10, &adapter).unwrap();
        assert_eq!(name, "Browsing");
    }

    #[test]
    fn routed_idx_on_active_monitor() {
        let ds = DisplaySet::new(crate::config::default_workspaces());
        let adapter = TestAdapter::new();
        adapter.inject_bundle_id(10, "com.apple.Safari".into());
        let wss = vec![
            Workspace::new("Development".into(), 1, (0, 0), (1920, 1080)),
            Workspace::new("Browsing".into(), 1, (0, 0), (1920, 1080)),
        ];
        let idx = ds.routed_workspace_idx(10, &wss, 0, &adapter);
        assert_eq!(idx, Some(1));
    }

    #[test]
    fn active_idx_falls_back_when_frontmost_has_no_window() {
        let mut ds = DisplaySet::new(crate::config::default_workspaces());
        let mut wss = Vec::new();
        ds.init_workspaces(
            &mut wss,
            &[DisplayInfo {
                id: 1,
                origin: (0, 0),
                size: (1920, 1080),
            }],
        );
        let pid_to_windows: HashMap<i32, Vec<WindowId>> = HashMap::new();
        let idx = ds.active_workspace_idx(&wss, &pid_to_windows, Some(99));
        assert_eq!(idx, 0);
    }

    // -- move decisions --

    #[test]
    fn plan_move_targets_with_room_and_rejects_noop() {
        let ds = DisplaySet::with_max_tiles(vec![], 2);
        let wss = vec![
            Workspace::new("a".into(), 1, (0, 0), (1920, 1080)),
            Workspace::new("b".into(), 1, (0, 0), (1920, 1080)),
        ];
        let dec = ds.plan_move(&wss, 0, 1).unwrap();
        assert_eq!((dec.from, dec.to), (0, 1));
        assert!(ds.plan_move(&wss, 0, 0).is_none());
    }

    #[test]
    fn plan_move_redirects_overflow_and_fails_when_full() {
        let ds = DisplaySet::with_max_tiles(vec![], 1);
        let mut wss = vec![
            Workspace::new("a".into(), 1, (0, 0), (1920, 1080)),
            Workspace::new("b".into(), 1, (0, 0), (1920, 1080)),
        ];
        wss[1].add_window(9, None);
        // Target 1 is full → overflow redirects onto 0 (here, back on `from`;
        // the caller then moves the window onto itself, a harmless no-op).
        let dec = ds.plan_move(&wss, 0, 1).unwrap();
        assert_eq!((dec.from, dec.to), (0, 0));
        // Both full → no decision.
        wss[0].add_window(8, None);
        assert!(ds.plan_move(&wss, 0, 1).is_none());
    }

    #[test]
    fn direction_target_resolves_visible_or_first() {
        let mut ds = DisplaySet::new(entries_two());
        let mut wss = Vec::new();
        ds.init_workspaces(&mut wss, &test_displays_two());
        let mut adapter = TestAdapter::new();
        adapter.displays = test_displays_two();
        use pengwm_core::tree::Direction;
        // From monitor 1 rightward → visible workspace on monitor 2 (idx 2).
        assert_eq!(
            ds.direction_target(&wss, 0, Direction::Right, &adapter),
            Some(2)
        );
        assert!(ds
            .direction_target(&wss, 0, Direction::Left, &adapter)
            .is_none());
    }

    #[test]
    fn direction_uses_height_for_source_center() {
        // Regression: `from_cy` once used the display width, shifting the
        // source center down whenever width != height. Display 2 sits where
        // only the shifted center reads as "up".
        let ds = DisplaySet::new(entries_two());
        let wss = vec![
            Workspace::new("a".into(), 1, (0, 0), (1920, 1080)),
            Workspace::new("b".into(), 2, (1360, -560), (800, 800)),
        ];
        let mut adapter = TestAdapter::new();
        adapter.displays = vec![
            DisplayInfo {
                id: 1,
                origin: (0, 0),
                size: (1920, 1080),
            },
            DisplayInfo {
                id: 2,
                origin: (1360, -560),
                size: (800, 800),
            },
        ];
        use pengwm_core::tree::Direction;
        // True center (960, 540): display 2 is rightward, not up.
        assert!(ds
            .direction_target(&wss, 0, Direction::Up, &adapter)
            .is_none());
        assert_eq!(
            ds.direction_target(&wss, 0, Direction::Right, &adapter),
            Some(1)
        );
    }
}
