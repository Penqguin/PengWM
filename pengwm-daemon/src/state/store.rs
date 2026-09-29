use std::collections::HashMap;
use std::time::Instant;

use pengwm_core::tree::WindowId;
use pengwm_core::workspace::Workspace;

use crate::state::hidden::HiddenTracker;

/// Single owner of wid↔pid↔hidden state. `HiddenTracker` is a private
/// implementation detail — its predicate seam (`Fn(WindowId)->bool`) stays
/// inside the store. `StateManager` retains `Vec<Workspace>` ownership and
/// borrows it per call. This is the deep module that concentrates the
/// "hidden means untiled but still tracked" invariant.
pub struct WindowStore {
    window_pids: HashMap<WindowId, i32>,
    pid_to_windows: HashMap<i32, Vec<WindowId>>,
    hidden: HiddenTracker,
}

impl WindowStore {
    pub fn new() -> Self {
        Self {
            window_pids: HashMap::new(),
            pid_to_windows: HashMap::new(),
            hidden: HiddenTracker::new(),
        }
    }

    #[cfg(test)]
    pub fn with_last_reconcile(last: Instant) -> Self {
        Self {
            window_pids: HashMap::new(),
            pid_to_windows: HashMap::new(),
            hidden: HiddenTracker::with_last_reconcile(last),
        }
    }

    // -- pid maps -----------------------------------------------------------

    pub fn register(&mut self, window_id: WindowId, pid: i32) {
        if let std::collections::hash_map::Entry::Vacant(e) = self.window_pids.entry(window_id) {
            e.insert(pid);
            self.pid_to_windows.entry(pid).or_default().push(window_id);
        }
        self.hidden.remove(window_id);
    }

    pub fn unregister(&mut self, window_id: WindowId) -> Option<i32> {
        let pid = self.window_pids.remove(&window_id)?;
        self.hidden.remove(window_id);
        if let Some(v) = self.pid_to_windows.get_mut(&pid) {
            v.retain(|&w| w != window_id);
            if v.is_empty() {
                self.pid_to_windows.remove(&pid);
            }
        }
        Some(pid)
    }

    pub fn remove_pid_tracking(&mut self, window_id: WindowId) {
        if let Some(pid) = self.window_pids.remove(&window_id) {
            if let Some(v) = self.pid_to_windows.get_mut(&pid) {
                v.retain(|&w| w != window_id);
                if v.is_empty() {
                    self.pid_to_windows.remove(&pid);
                }
            }
        }
        self.hidden.remove(window_id);
    }

    pub fn remove_pid(&mut self, pid: i32) -> Vec<WindowId> {
        let windows = self.pid_to_windows.remove(&pid).unwrap_or_default();
        for &wid in &windows {
            self.window_pids.remove(&wid);
            self.hidden.remove(wid);
        }
        windows
    }

    pub fn contains(&self, window_id: WindowId) -> bool {
        self.window_pids.contains_key(&window_id)
    }

    pub fn pid_for(&self, window_id: WindowId) -> Option<i32> {
        self.window_pids.get(&window_id).copied()
    }

    pub fn windows_for(&self, pid: i32) -> Option<&[WindowId]> {
        self.pid_to_windows.get(&pid).map(|v| v.as_slice())
    }

    pub fn all_pids(&self) -> &HashMap<i32, Vec<WindowId>> {
        &self.pid_to_windows
    }

    pub fn all_window_pids(&self) -> &HashMap<WindowId, i32> {
        &self.window_pids
    }

    pub fn len(&self) -> usize {
        self.window_pids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.window_pids.is_empty()
    }

    // -- hidden state (HiddenTracker is private detail) ----------------------

    /// True when the window is hidden-tracked (untiled but still owned).
    pub fn is_hidden(&self, window_id: WindowId) -> bool {
        self.hidden.contains(window_id)
    }

    /// Remember a tiled window's workspace. Returns the index for the caller
    /// to remove from the tree and re-layout — the store never mutates trees.
    pub fn hide(&mut self, window_id: WindowId, workspaces: &[Workspace]) -> Option<usize> {
        self.hidden.hide_window(window_id, workspaces)
    }

    /// Forget a hidden entry and return its remembered workspace index, if any.
    pub fn reveal(&mut self, window_id: WindowId) -> Option<usize> {
        self.hidden.take_hidden(window_id)
    }

    #[cfg(test)]
    pub fn hidden_insert(&mut self, window_id: WindowId, idx: usize) {
        self.hidden.insert(window_id, idx);
    }

    /// Drain every hidden entry with its remembered workspace index. The
    /// caller retiles each (remembered, routed, or active workspace).
    pub fn reveal_all(&mut self) -> Vec<(WindowId, usize)> {
        self.hidden.drain()
    }

    pub fn should_reconcile(&mut self, now: Instant) -> bool {
        self.hidden.should_reconcile(now)
    }

    pub fn pending_for_reconcile<F>(
        &self,
        workspaces: &[Workspace],
        is_hidden: F,
    ) -> (Vec<WindowId>, Vec<WindowId>)
    where
        F: Fn(WindowId) -> bool,
    {
        self.hidden
            .pending_for_reconcile(&self.window_pids, workspaces, is_hidden)
    }

    #[cfg(test)]
    pub fn inject(&mut self, window_id: WindowId, pid: i32) {
        self.window_pids.insert(window_id, pid);
        self.pid_to_windows.entry(pid).or_default().push(window_id);
    }

    /// Test helper: replace the entire window list for `pid` (used to fake
    /// `active_workspace_idx` routing in overflow tests).
    #[cfg(test)]
    pub fn set_windows_for_pid(&mut self, pid: i32, windows: Vec<WindowId>) {
        // Remove old entries for this pid
        if let Some(old) = self.pid_to_windows.remove(&pid) {
            for wid in old {
                self.window_pids.remove(&wid);
            }
        }
        for &wid in &windows {
            self.window_pids.insert(wid, pid);
        }
        if !windows.is_empty() {
            self.pid_to_windows.insert(pid, windows);
        }
    }
}

impl Default for WindowStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pengwm_core::workspace::Workspace;
    use std::time::Duration;

    fn ws_with(ids: &[WindowId]) -> Vec<Workspace> {
        let mut ws = Workspace::new("ws".into(), 1, (0, 0), (1920, 1080));
        for &id in ids {
            ws.add_window(id, None);
        }
        vec![ws]
    }

    fn ws_with_windows(ids: &[WindowId]) -> Vec<Workspace> {
        ws_with(ids)
    }

    #[test]
    fn register_and_lookup() {
        let mut r = WindowStore::new();
        r.register(10, 42);
        assert_eq!(r.pid_for(10), Some(42));
        assert_eq!(r.windows_for(42), Some(&[10][..]));
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn register_idempotent() {
        let mut r = WindowStore::new();
        r.register(10, 42);
        r.register(10, 42);
        assert_eq!(r.windows_for(42).unwrap().len(), 1);
    }

    #[test]
    fn unregister_removes_both_maps() {
        let mut r = WindowStore::new();
        r.register(10, 42);
        r.register(20, 42);
        r.unregister(10);
        assert!(!r.contains(10));
        assert_eq!(r.windows_for(42), Some(&[20][..]));
    }

    #[test]
    fn remove_pid_clears_all() {
        let mut r = WindowStore::new();
        r.register(10, 42);
        r.register(20, 42);
        let removed = r.remove_pid(42);
        assert_eq!(removed.len(), 2);
        assert!(r.is_empty());
    }

    #[test]
    fn hide_and_reveal_roundtrip() {
        let mut wss = ws_with(&[10, 20]);
        let mut r = WindowStore::new();
        r.register(10, 42);
        r.register(20, 42);
        let idx = r.hide(10, &wss).unwrap();
        assert_eq!(idx, 0);
        assert!(r.is_hidden(10));
        // Tree removal is the caller's job.
        assert!(wss[0].find_window(10).is_some());
        wss[0].remove_window(10);
        assert!(wss[0].find_window(10).is_none());
        let remembered = r.reveal(10).unwrap();
        assert_eq!(remembered, 0);
        assert!(!r.is_hidden(10));
    }

    #[test]
    fn pending_delegates_to_hidden() {
        let wss = ws_with(&[1, 2]);
        let mut r = WindowStore::new();
        r.register(1, 42);
        r.register(2, 42);
        let (to_hide, to_show) = r.pending_for_reconcile(&wss, |wid| wid == 1);
        assert_eq!(to_hide, vec![1]);
        assert!(to_show.is_empty());
    }

    #[test]
    fn hide_window_remembers_idx_without_mutating_tree() {
        let workspaces = ws_with_windows(&[10, 20]);
        let mut t = WindowStore::new();
        t.register(10, 42);
        t.register(20, 42);
        let idx = t.hide(10, &workspaces).unwrap();
        assert_eq!(idx, 0);
        assert!(workspaces[0].find_window(10).is_some());
        assert!(t.is_hidden(10));
    }

    #[test]
    fn hide_window_returns_none_when_not_tiled() {
        let workspaces = ws_with_windows(&[10]);
        let mut t = WindowStore::new();
        assert!(t.hide(99, &workspaces).is_none());
        assert!(!t.is_hidden(99));
    }

    #[test]
    fn take_hidden_returns_remembered_and_clears() {
        let mut t = WindowStore::new();
        t.hidden_insert(10, 2);
        assert_eq!(t.reveal(10), Some(2));
        assert!(!t.is_hidden(10));
    }

    #[test]
    fn pending_to_hide_when_predicate_says_hidden_and_tiled() {
        let workspaces = ws_with_windows(&[1, 2]);
        let mut pids = WindowStore::new();
        pids.register(1, 42);
        pids.register(2, 42);
        let is_hidden = |wid| wid == 1;
        let (to_hide, to_show) = pids.pending_for_reconcile(&workspaces, is_hidden);
        assert_eq!(to_hide, vec![1]);
        assert!(to_show.is_empty());
    }

    #[test]
    fn pending_to_show_when_hidden_map_has_entry_but_predicate_says_visible() {
        let workspaces = ws_with_windows(&[2]);
        let mut pids = WindowStore::new();
        pids.register(1, 42);
        pids.register(2, 42);
        pids.hidden_insert(1, 0);
        let is_hidden = |_| false;
        let (to_hide, to_show) = pids.pending_for_reconcile(&workspaces, is_hidden);
        assert!(to_hide.is_empty());
        assert_eq!(to_show, vec![1]);
    }

    #[test]
    fn pending_hides_all_windows_when_app_hidden() {
        let workspaces = ws_with_windows(&[10, 20]);
        let mut pids = WindowStore::new();
        pids.register(10, 42);
        pids.register(20, 42);
        let is_hidden = |_| true;
        let (to_hide, _) = pids.pending_for_reconcile(&workspaces, is_hidden);
        assert_eq!(to_hide.len(), 2);
    }

    #[test]
    fn should_reconcile_debounces_by_interval() {
        let now = Instant::now();
        let mut t = WindowStore::with_last_reconcile(now);
        assert!(!t.should_reconcile(now));
        assert!(t.should_reconcile(now + Duration::from_secs(2)));
        assert!(!t.should_reconcile(now + Duration::from_secs(2) + Duration::from_millis(100)));
    }
}
