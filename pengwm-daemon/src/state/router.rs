use std::collections::HashMap;

use crate::config::WorkspaceEntry;
use pengwm_core::tree::WindowId;
use pengwm_core::workspace::Workspace;

use crate::adapter::OsAdapter;
use crate::state::display::DisplaySet;

/// Owns the routing policy: `max_tiles` capacity and the `active` workspace
/// heuristic. `entries` (the named workspace set) is borrowed from
/// `DisplaySet` per call — so a `ReloadConfig` that updates entries is
/// immediately visible without cloning. Returns indices/actions so
/// `StateManager` retains `Vec<Workspace>` mutation and `apply_layout`.
pub struct Router {
    max_tiles: usize,
}

impl Router {
    pub fn new(max_tiles: usize) -> Self {
        Self {
            max_tiles: max_tiles.max(1),
        }
    }

    pub fn max_tiles(&self) -> usize {
        self.max_tiles
    }

    pub fn set_max_tiles(&mut self, n: usize) {
        self.max_tiles = n.max(1);
    }

    /// First workspace after `start` (wrapping within the same monitor) with
    /// room for another window. `None` when every workspace on that monitor is
    /// at capacity.
    pub fn find_next_with_capacity(
        &self,
        workspaces: &[Workspace],
        start: usize,
    ) -> Option<usize> {
        let n = workspaces.len();
        if n == 0 {
            return None;
        }
        let monitor = workspaces[start].monitor_id;
        for offset in 1..=n {
            let idx = (start + offset) % n;
            if workspaces[idx].monitor_id != monitor {
                continue;
            }
            if workspaces[idx].window_count() < self.max_tiles {
                return Some(idx);
            }
        }
        None
    }

    /// Heuristic for "which workspace is active": the workspace that contains a
    /// window belonging to `frontmost_pid`, mapped through `active` per
    /// monitor. Falls back to arbitrary `active` entry. This is the fragile
    /// piece — now isolated here for testability.
    pub fn active_workspace_idx(
        &self,
        workspaces: &[Workspace],
        pid_to_windows: &HashMap<i32, Vec<WindowId>>,
        frontmost_pid: Option<i32>,
        displays: &DisplaySet,
    ) -> usize {
        if let Some(pid) = frontmost_pid {
            if let Some(windows) = pid_to_windows.get(&pid) {
                for &window_id in windows {
                    for ws in workspaces {
                        if ws.find_window(window_id).is_some() {
                            if let Some(&idx) = displays.active().get(&ws.monitor_id) {
                                if idx < workspaces.len() {
                                    return idx;
                                }
                            }
                        }
                    }
                }
            }
        }
        displays.active().values().next().copied().unwrap_or(0)
    }

    /// Name of the configured workspace `pid`'s app is assigned to, matched
    /// case-insensitively against bundle id first, then app display name.
    pub fn configured_workspace_name_for_pid<'a>(
        &self,
        pid: i32,
        os: &dyn OsAdapter,
        entries: &'a [WorkspaceEntry],
    ) -> Option<&'a str> {
        if entries.is_empty() {
            return None;
        }
        let bundle = os.app_bundle_id(pid);
        let app_name = os.app_name(pid);
        entries
            .iter()
            .find(|entry| {
                entry.apps.iter().any(|app| {
                    bundle.as_deref().is_some_and(|b| b.eq_ignore_ascii_case(app))
                        || app_name
                            .as_deref()
                            .is_some_and(|n| n.eq_ignore_ascii_case(app))
                })
            })
            .map(|e| e.name.as_str())
    }

    /// Flat workspace index a new window from `pid` should land in: the
    /// configured workspace for the app on the active monitor. `None` when the
    /// app isn't assigned.
    pub fn routed_workspace_idx(
        &self,
        pid: i32,
        workspaces: &[Workspace],
        active_idx: usize,
        os: &dyn OsAdapter,
        entries: &[WorkspaceEntry],
    ) -> Option<usize> {
        if active_idx >= workspaces.len() {
            return None;
        }
        let monitor = workspaces[active_idx].monitor_id;
        let name = self.configured_workspace_name_for_pid(pid, os, entries)?;
        workspaces
            .iter()
            .position(|ws| ws.name == name && ws.monitor_id == monitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::{DisplayInfo, OsAdapter};
    use crate::adapter_test::TestAdapter;
    use crate::config::WorkspaceEntry;
    use pengwm_core::workspace::Workspace;

    fn entries_default() -> Vec<WorkspaceEntry> {
        crate::config::default_workspaces()
    }

    fn displays_two() -> DisplayInfo {
        DisplayInfo {
            id: 1,
            origin: (0, 0),
            size: (1920, 1080),
        }
    }

    #[test]
    fn find_next_wraps_within_monitor() {
        let r = Router::new(2);
        let mut wss = vec![
            Workspace::new("a".into(), 1, (0, 0), (1920, 1080)),
            Workspace::new("b".into(), 1, (0, 0), (1920, 1080)),
            Workspace::new("c".into(), 2, (1920, 0), (1920, 1080)),
        ];
        wss[0].add_window(1, None);
        wss[0].add_window(2, None);
        // wss[0] full, next on same monitor is 1
        assert_eq!(r.find_next_with_capacity(&wss, 0), Some(1));
        // wss[1] not full -> returns None? Actually it has room, but find_next looks *after* start, so from 1 it wraps to 0 which is full, then to 1? Wait offset 1..=n includes start+1..wrap, so from 1, next is 0 (full) then 1 is start? No, (1+1)%3=2 is other monitor skip, (1+2)%3=0 full, (1+3)%3=1 is start itself? But original find_next uses offset 1..=n, so it will eventually wrap to start itself if start is the only one with room? But start is assumed full when we call it, so it should find the other. For this test, 1 is the target, not the start, so fine.
    }

    #[test]
    fn configured_name_matches_bundle_case_insensitive() {
        let r = Router::new(4);
        let mut adapter = TestAdapter::new();
        adapter.bundle_ids.insert(10, "com.google.Chrome".into());
        let entries = entries_default();
        let name = r
            .configured_workspace_name_for_pid(10, &adapter, &entries)
            .unwrap();
        assert_eq!(name, "Browsing");
    }

    #[test]
    fn configured_name_matches_app_name() {
        let r = Router::new(4);
        let mut adapter = TestAdapter::new();
        adapter.app_names.insert(10, "spotify".into());
        let entries = entries_default();
        let name = r
            .configured_workspace_name_for_pid(10, &adapter, &entries)
            .unwrap();
        assert_eq!(name, "Music");
    }

    #[test]
    fn routed_idx_on_active_monitor() {
        let r = Router::new(4);
        let mut adapter = TestAdapter::new();
        adapter.bundle_ids.insert(10, "com.apple.Safari".into());
        let entries = entries_default();
        let wss = vec![
            Workspace::new("Development".into(), 1, (0, 0), (1920, 1080)),
            Workspace::new("Browsing".into(), 1, (0, 0), (1920, 1080)),
        ];
        // active is 0 (Development), but Safari routes to Browsing (index 1)
        let idx = r.routed_workspace_idx(10, &wss, 0, &adapter, &entries);
        assert_eq!(idx, Some(1));
    }

    #[test]
    fn active_idx_falls_back_when_frontmost_has_no_window() {
        let r = Router::new(4);
        let mut ds = DisplaySet::new(entries_default());
        let mut wss = Vec::new();
        ds.init_workspaces(&mut wss, &[displays_two()]);
        let pid_to_windows: HashMap<i32, Vec<WindowId>> = HashMap::new();
        let idx = r.active_workspace_idx(&wss, &pid_to_windows, Some(99), &ds);
        assert_eq!(idx, 0);
    }
}
