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
    focused_output: Option<u32>,
}

impl DisplaySet {
    pub fn new(entries: Vec<WorkspaceEntry>) -> Self {
        Self {
            active: BTreeMap::new(),
            entries,
            max_tiles: 4,
            focused_output: None,
        }
    }

    pub fn with_max_tiles(entries: Vec<WorkspaceEntry>, max_tiles: usize) -> Self {
        Self {
            active: BTreeMap::new(),
            entries,
            max_tiles: max_tiles.max(1),
            focused_output: None,
        }
    }

    pub fn focused_output(&self) -> Option<u32> {
        self.focused_output
    }

    pub fn set_focused_output(&mut self, monitor: u32) {
        self.focused_output = Some(monitor);
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

    // -- global pool helpers (one tree per name; `monitor_id` is assignment) --

    /// Flat indices of workspaces assigned to `monitor`.
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

    /// Resolve a 1-based global workspace id `n` (from `Command::Workspace`)
    /// into a flat workspace index. Global config order, any output (#1/Q4).
    pub fn resolve_workspace(&self, n: usize, workspaces: &[Workspace]) -> Option<usize> {
        if n == 0 || n > workspaces.len() {
            return None;
        }
        Some(n - 1)
    }

    /// First workspace after `start` (wrapping over the global pool) with
    /// room for another window. `None` when every workspace is at capacity.
    /// Global spill (#1/Q5): overflow is no longer confined to one monitor.
    pub fn next_with_room(&self, workspaces: &[Workspace], start: usize) -> Option<usize> {
        if start >= workspaces.len() || workspaces.is_empty() {
            return None;
        }
        let n = workspaces.len();
        for offset in 1..=n {
            let idx = (start + offset) % n;
            if workspaces[idx].window_count() < self.max_tiles {
                return Some(idx);
            }
        }
        None
    }

    /// Overflow redirect: `target` itself when it has room, else the next
    /// global workspace with room. `None` when the pool is full.
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
    /// else the global overflow redirect. `None` when the move
    /// is a no-op (`from == target`) or every workspace is full.
    /// The caller moves the window and re-layouts visible ends — DisplaySet
    /// answers, commands execute. (Moves to another output bypass the cap
    /// via `move_window_to_display` — #2/Q10 — and never reach this funnel.)
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

    /// Decide a switch to global workspace `target` from the focused output
    /// (#1/Q2): no-op when already shown here; swap assignment when visible
    /// on another output (the other output falls back to the workspace just
    /// left); plain pull when hidden. Mutates `active` + `monitor_id` /
    /// geometry (via `os` display infos) and answers what the caller must
    /// hide and re-layout — DisplaySet answers, commands execute.
    pub fn plan_switch(
        &mut self,
        target: usize,
        workspaces: &mut [Workspace],
        os: &dyn OsAdapter,
    ) -> Option<SwitchDecision> {
        if target >= workspaces.len() {
            return None;
        }
        let here = self.focused_output?;
        let current = self.active.get(&here).copied()?;
        if target == current || current >= workspaces.len() {
            return None;
        }
        let other_output = self
            .active
            .iter()
            .find(|(_, &idx)| idx == target)
            .map(|(&mon, _)| mon);
        match other_output {
            Some(other) if other != here => {
                // Swap: target comes here, the workspace just left goes there.
                let infos = os.active_displays();
                let here_info = infos.iter().find(|d| d.id == here);
                let other_info = infos.iter().find(|d| d.id == other);
                if let (Some(h), Some(o)) = (here_info, other_info) {
                    workspaces[target].monitor_id = here;
                    workspaces[target].update_monitor_geometry(h.origin, h.size);
                    workspaces[current].monitor_id = other;
                    workspaces[current].update_monitor_geometry(o.origin, o.size);
                }
                self.active.insert(here, target);
                self.active.insert(other, current);
                Some(SwitchDecision {
                    show: target,
                    hide: vec![],
                    relayout: vec![target, current],
                })
            }
            _ => {
                // Pull a hidden workspace onto the focused output.
                let infos = os.active_displays();
                if let Some(info) = infos.iter().find(|d| d.id == here) {
                    workspaces[target].monitor_id = here;
                    workspaces[target].update_monitor_geometry(info.origin, info.size);
                }
                self.active.insert(here, target);
                Some(SwitchDecision {
                    show: target,
                    hide: vec![current],
                    relayout: vec![target],
                })
            }
        }
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
    /// window belonging to `frontmost_pid` when that workspace is visible;
    /// else the visible workspace on `focused_output`; else an arbitrary
    /// `active` entry. The explicit focus cell is the primary answer (#1/Q8);
    /// the pid heuristic is fallback for external (mouse) focus changes.
    pub fn active_workspace_idx(
        &self,
        workspaces: &[Workspace],
        pid_to_windows: &HashMap<i32, Vec<WindowId>>,
        frontmost_pid: Option<i32>,
    ) -> usize {
        if let Some(pid) = frontmost_pid {
            if let Some(windows) = pid_to_windows.get(&pid) {
                for &window_id in windows {
                    for (idx, ws) in workspaces.iter().enumerate() {
                        if ws.find_window(window_id).is_some() && self.is_visible(idx, workspaces) {
                            return idx;
                        }
                    }
                }
            }
        }
        if let Some(mon) = self.focused_output {
            if let Some(&idx) = self.active.get(&mon) {
                if idx < workspaces.len() {
                    return idx;
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
    /// global workspace matching the app's configured name (#1/Q5), wherever
    /// it currently lives. No auto-pull: the window lands there even when it
    /// is hidden on another output. `None` when the app isn't assigned.
    pub fn routed_workspace_idx(
        &self,
        pid: i32,
        workspaces: &[Workspace],
        os: &dyn OsAdapter,
    ) -> Option<usize> {
        let name = self.configured_workspace_name_for_pid(pid, os)?;
        workspaces.iter().position(|ws| ws.name == name)
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

    /// Initialize the global pool: one `Workspace` per config entry, each
    /// output showing exactly one workspace. `monitor` affinity is an
    /// initial-output hint (#1/Q3); output `i` initially shows entry `i`
    /// (wrapping), the rest are hidden. Sets `focused_output` to `primary_id`.
    /// Returns the number of workspaces created (for tests).
    pub fn init_workspaces(
        &mut self,
        workspaces: &mut Vec<Workspace>,
        displays: &[DisplayInfo],
        primary_id: u32,
    ) -> usize {
        workspaces.clear();
        self.active.clear();
        if displays.is_empty() {
            return 0;
        }
        let primary = displays
            .iter()
            .find(|d| d.id == primary_id)
            .or_else(|| displays.first())
            .unwrap();
        // One tree per entry; home output = hint match else primary.
        for entry in &self.entries {
            let home = displays
                .iter()
                .find(|d| Self::entry_applies_to_display(entry, d))
                .map(|d| d.id);
            // `entry_applies_to_display` returns true for hint-less entries on
            // every display, so `find` yields the first display — re-point
            // hint-less entries at primary for a stable home.
            let home_id = match &entry.monitor {
                None => primary.id,
                Some(_) => home.unwrap_or(primary.id),
            };
            let info = displays.iter().find(|d| d.id == home_id).unwrap_or(primary);
            workspaces.push(Workspace::new(
                entry.name.clone(),
                home_id,
                info.origin,
                info.size,
            ));
        }
        // Output i shows entry i (wrapping); reassign geometry to the output.
        for (i, display) in displays.iter().enumerate() {
            if workspaces.is_empty() {
                break;
            }
            let idx = i % workspaces.len();
            workspaces[idx].monitor_id = display.id;
            workspaces[idx].update_monitor_geometry(display.origin, display.size);
            self.active.insert(display.id, idx);
        }
        self.focused_output = Some(primary.id);
        workspaces.len()
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

    /// Handle `MonitorAdded`: the new output shows the first hidden workspace,
    /// else pulls the first global workspace (swap). No new trees are created
    /// (#1/Q7). Answers what to show/lay out; the caller executes (#3/Q12).
    pub fn on_added(
        &mut self,
        display_id: u32,
        workspaces: &mut [Workspace],
        os: &dyn OsAdapter,
    ) -> Option<TopologySync> {
        let info = os
            .active_displays()
            .into_iter()
            .find(|d| d.id == display_id)?;
        if workspaces.is_empty() {
            return None;
        }
        let visible: std::collections::HashSet<usize> = self.active.values().copied().collect();
        let shown = (0..workspaces.len())
            .find(|idx| !visible.contains(idx))
            .unwrap_or(0);
        workspaces[shown].monitor_id = display_id;
        workspaces[shown].update_monitor_geometry(info.origin, info.size);
        self.active.insert(display_id, shown);
        Some(TopologySync {
            shown: vec![shown],
            hidden: vec![],
            relayout: vec![shown],
        })
    }

    /// Handle `MonitorRemoved`: workspaces assigned to the removed output move
    /// to primary (hidden there unless primary shows them); the removed id
    /// leaves `active`. Names are already unique so no dedup is needed.
    /// Answers the sync; the caller hides + re-layouts + publishes (#3/Q12).
    pub fn on_removed(
        &mut self,
        removed_id: u32,
        workspaces: &mut Vec<Workspace>,
        os: &dyn OsAdapter,
    ) -> TopologySync {
        let was_visible = self.active.get(&removed_id).copied();
        let primary = os.primary_display_id();
        let primary_info = os.active_displays().into_iter().find(|d| d.id == primary);
        let (primary_origin, primary_size) = primary_info
            .map(|d| (d.origin, d.size))
            .unwrap_or(((0, 0), (1920, 1080)));
        for ws in workspaces.iter_mut() {
            if ws.monitor_id == removed_id {
                ws.monitor_id = primary;
                ws.update_monitor_geometry(primary_origin, primary_size);
            }
        }
        self.active.remove(&removed_id);
        if self.focused_output == Some(removed_id) {
            self.focused_output = Some(primary);
        }
        if workspaces.is_empty() {
            workspaces.push(Workspace::new(
                "ws-1".into(),
                primary,
                primary_origin,
                primary_size,
            ));
        }
        // Every live output shows exactly one workspace.
        let live: Vec<u32> = os.active_displays().iter().map(|d| d.id).collect();
        for mon in &live {
            if !self.active.contains_key(mon) {
                let visible: std::collections::HashSet<usize> =
                    self.active.values().copied().collect();
                let shown = (0..workspaces.len()).find(|idx| !visible.contains(idx));
                if let Some(idx) = shown {
                    if let Some(info) = os.active_displays().into_iter().find(|d| &d.id == mon) {
                        workspaces[idx].monitor_id = *mon;
                        workspaces[idx].update_monitor_geometry(info.origin, info.size);
                    }
                    self.active.insert(*mon, idx);
                } else if !workspaces.is_empty() {
                    self.active.insert(*mon, 0);
                }
            }
        }
        let visible_now: std::collections::HashSet<usize> = self.active.values().copied().collect();
        let hidden = was_visible
            .filter(|idx| !visible_now.contains(idx))
            .into_iter()
            .collect();
        TopologySync {
            shown: vec![],
            hidden,
            relayout: visible_now.into_iter().collect(),
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
#[derive(Debug)]
pub struct MoveDecision {
    pub from: usize,
    pub to: usize,
}

/// A decided workspace switch: `show` becomes visible on the focused output,
/// `hide` are the now-hidden siblings to park offscreen, `relayout` the
/// workspaces needing `apply_layout`. Answered (plus assignment mutation)
/// by `DisplaySet`, executed by the caller.
#[derive(Debug)]
pub struct SwitchDecision {
    pub show: usize,
    pub hide: Vec<usize>,
    pub relayout: Vec<usize>,
}

/// A decided monitor-topology sync: `shown` workspaces need layout on their
/// (new) output, `hidden` are newly parked offscreen, `relayout` the full
/// set needing `apply_layout`. Answered by `DisplaySet`, executed by the
/// caller (#3/Q12: hide + layout + bar stay with `StateManager`).
#[derive(Debug)]
pub struct TopologySync {
    pub shown: Vec<usize>,
    pub hidden: Vec<usize>,
    pub relayout: Vec<usize>,
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
    fn init_workspaces_creates_global_pool() {
        let mut ds = DisplaySet::new(entries_two());
        let mut wss = Vec::new();
        ds.init_workspaces(&mut wss, &test_displays_two(), 1);
        // One tree per entry globally; output i shows entry i.
        assert_eq!(wss.len(), 2);
        assert_eq!(wss[0].monitor_id, 1);
        assert_eq!(wss[1].monitor_id, 2);
        assert_eq!(ds.active.get(&1), Some(&0));
        assert_eq!(ds.active.get(&2), Some(&1));
        assert_eq!(ds.focused_output(), Some(1));
    }

    #[test]
    fn on_added_shows_first_hidden() {
        let mut ds = DisplaySet::new(entries_two());
        let mut wss: Vec<Workspace> = Vec::new();
        ds.init_workspaces(&mut wss, &test_displays_one(), 1);
        assert_eq!(wss.len(), 2);

        let mut adapter = TestAdapter::new();
        adapter.displays = test_displays_two();
        // One display shows entry 0; entry 1 is hidden → new output shows it.
        let sync = ds.on_added(2, &mut wss, &adapter).unwrap();
        assert_eq!(sync.shown, vec![1]);
        assert_eq!(sync.relayout, vec![1]);
        assert!(sync.hidden.is_empty());
        assert_eq!(wss.len(), 2);
        assert_eq!(ds.active.get(&2), Some(&1));
        assert_eq!(wss[1].monitor_id, 2);
    }

    #[test]
    fn on_added_pulls_first_when_none_hidden() {
        let mut ds = DisplaySet::new(entries_two());
        let mut wss: Vec<Workspace> = Vec::new();
        ds.init_workspaces(&mut wss, &test_displays_two(), 1);
        let mut adapter = TestAdapter::new();
        adapter.displays = vec![
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
            DisplayInfo {
                id: 3,
                origin: (3840, 0),
                size: (1920, 1080),
            },
        ];
        let sync = ds.on_added(3, &mut wss, &adapter).unwrap();
        assert_eq!(sync.shown, vec![0]);
        assert_eq!(ds.active.get(&3), Some(&0));
    }

    #[test]
    fn on_removed_reassigns_orphans_to_primary() {
        let mut ds = DisplaySet::new(entries_two());
        let mut wss: Vec<Workspace> = Vec::new();
        ds.init_workspaces(&mut wss, &test_displays_two(), 1);
        // Display 2 showed entry 1; after removal it is hidden on primary.
        let mut adapter = TestAdapter::new();
        adapter.displays = test_displays_one();
        let sync = ds.on_removed(2, &mut wss, &adapter);
        assert!(wss.iter().all(|ws| ws.monitor_id == 1));
        assert_eq!(wss.len(), 2);
        assert!(!ds.active().contains_key(&2));
        // Entry 1 was visible on removed output 2 → now hidden.
        assert_eq!(sync.hidden, vec![1]);
        assert!(sync.relayout.contains(&0));
    }

    #[test]
    fn on_resized_updates_geometry_and_returns_affected() {
        let mut ds = DisplaySet::new(entries_two());
        let mut wss: Vec<Workspace> = Vec::new();
        ds.init_workspaces(&mut wss, &test_displays_two(), 1);
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
        assert_eq!(affected, vec![0]);
        assert_eq!(wss[0].monitor_size(), (2560, 1440));
        assert_eq!(wss[1].monitor_size(), (1920, 1080));
    }

    // -- routing helpers (global pool) --

    #[test]
    fn next_with_room_spills_globally() {
        let ds = DisplaySet::with_max_tiles(vec![], 2);
        let mut wss = vec![
            Workspace::new("a".into(), 1, (0, 0), (1920, 1080)),
            Workspace::new("b".into(), 1, (0, 0), (1920, 1080)),
            Workspace::new("c".into(), 2, (1920, 0), (1920, 1080)),
        ];
        wss[0].add_window(1, None);
        wss[0].add_window(2, None);
        assert_eq!(ds.next_with_room(&wss, 0), Some(1));
        // Full workspace spills across outputs now: from 1 → 2, not None.
        assert_eq!(ds.next_with_room(&wss, 1), Some(2));
    }

    #[test]
    fn next_with_room_none_when_pool_full() {
        let ds = DisplaySet::with_max_tiles(vec![], 1);
        let mut wss = vec![
            Workspace::new("a".into(), 1, (0, 0), (1920, 1080)),
            Workspace::new("b".into(), 2, (1920, 0), (1920, 1080)),
        ];
        wss[0].add_window(1, None);
        wss[1].add_window(2, None);
        assert_eq!(ds.next_with_room(&wss, 0), None);
    }

    #[test]
    fn resolve_workspace_is_global() {
        let ds = DisplaySet::new(entries_two());
        let wss = vec![
            Workspace::new("a".into(), 1, (0, 0), (1920, 1080)),
            Workspace::new("b".into(), 2, (1920, 0), (1920, 1080)),
        ];
        assert_eq!(ds.resolve_workspace(2, &wss), Some(1));
        assert_eq!(ds.resolve_workspace(1, &wss), Some(0));
        assert_eq!(ds.resolve_workspace(3, &wss), None);
        assert_eq!(ds.resolve_workspace(0, &wss), None);
    }

    #[test]
    fn workspaces_on_reflects_assignment() {
        let mut ds = DisplaySet::new(entries_two());
        let mut wss = Vec::new();
        ds.init_workspaces(&mut wss, &test_displays_two(), 1);
        assert_eq!(ds.workspaces_on(1, &wss), vec![0]);
        assert_eq!(ds.workspaces_on(2, &wss), vec![1]);
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
    fn routed_idx_matches_global_name() {
        let ds = DisplaySet::new(crate::config::default_workspaces());
        let adapter = TestAdapter::new();
        adapter.inject_bundle_id(10, "com.apple.Safari".into());
        let wss = vec![
            Workspace::new("Development".into(), 1, (0, 0), (1920, 1080)),
            Workspace::new("Browsing".into(), 2, (1920, 0), (1920, 1080)),
        ];
        // Global match: Browsing lives on output 2, still routed there.
        let idx = ds.routed_workspace_idx(10, &wss, &adapter);
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
            1,
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

    // -- switch decisions (global pool pull/swap) --

    #[test]
    fn plan_switch_pulls_hidden() {
        let mut ds = DisplaySet::new(entries_two());
        let mut wss = Vec::new();
        ds.init_workspaces(&mut wss, &test_displays_one(), 1);
        // One output shows 0; 1 is hidden → pull hides 0, shows 1.
        let mut adapter = TestAdapter::new();
        adapter.displays = test_displays_one();
        let dec = ds.plan_switch(1, &mut wss, &adapter).unwrap();
        assert_eq!(dec.show, 1);
        assert_eq!(dec.hide, vec![0]);
        assert_eq!(dec.relayout, vec![1]);
        assert_eq!(ds.active.get(&1), Some(&1));
    }

    #[test]
    fn plan_switch_swaps_when_visible_elsewhere() {
        let mut ds = DisplaySet::new(entries_two());
        let mut wss = Vec::new();
        ds.init_workspaces(&mut wss, &test_displays_two(), 1);
        // Output 1 shows 0, output 2 shows 1. Focused = 1, switch to 1 → swap.
        let mut adapter = TestAdapter::new();
        adapter.displays = test_displays_two();
        let dec = ds.plan_switch(1, &mut wss, &adapter).unwrap();
        assert_eq!(dec.show, 1);
        assert!(dec.hide.is_empty());
        assert_eq!(ds.active.get(&1), Some(&1));
        assert_eq!(ds.active.get(&2), Some(&0));
        assert_eq!(wss[1].monitor_id, 1);
        assert_eq!(wss[0].monitor_id, 2);
    }

    #[test]
    fn plan_switch_noop_when_already_shown() {
        let mut ds = DisplaySet::new(entries_two());
        let mut wss = Vec::new();
        ds.init_workspaces(&mut wss, &test_displays_one(), 1);
        let adapter = TestAdapter::new();
        assert!(ds.plan_switch(0, &mut wss, &adapter).is_none());
        assert!(ds.plan_switch(9, &mut wss, &adapter).is_none());
    }

    #[test]
    fn direction_target_resolves_visible_or_first() {
        let mut ds = DisplaySet::new(entries_two());
        let mut wss = Vec::new();
        ds.init_workspaces(&mut wss, &test_displays_two(), 1);
        let mut adapter = TestAdapter::new();
        adapter.displays = test_displays_two();
        use pengwm_core::tree::Direction;
        // Two workspaces, two outputs: mon 1 shows 0, mon 2 shows 1.
        // From 0 rightward → visible workspace on monitor 2 (idx 1).
        assert_eq!(
            ds.direction_target(&wss, 0, Direction::Right, &adapter),
            Some(1)
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
