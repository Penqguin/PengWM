use std::collections::HashMap;
use std::time::{Duration, Instant};

use pengwm_core::tree::WindowId;
use pengwm_core::workspace::Workspace;

const RECONCILE_INTERVAL: Duration = Duration::from_secs(1);

/// Tracks windows that were minimized / app-hidden: they are removed from the
/// tiling tree (via `on_window_hidden`) but kept in `window_pids` so they can
/// be retiled by `on_window_shown`. The map remembers the workspace index they
/// came from, so `on_window_shown` can route them back.
///
/// Owns only the `HiddenTracker` state — `hidden` map + `last_reconcile` — and
/// borrows `&mut [Workspace]` / `&HashMap<WindowId,i32>` per call. Callers
/// (StateManager) handle `apply_layout` / `publish_bar_state` after the tracker
/// returns the affected workspace index. This keeps `OsAdapter` out of the
/// sub-module and makes reconcile testable via a `Fn(WindowId)->bool`
/// predicate (no `as_any_mut` downcast).
pub struct HiddenTracker {
    hidden: HashMap<WindowId, usize>,
    last_reconcile: Instant,
}

impl HiddenTracker {
    pub fn new() -> Self {
        Self {
            hidden: HashMap::new(),
            last_reconcile: Instant::now(),
        }
    }

    /// For tests that need a deterministic clock.
    #[cfg(test)]
    pub fn with_last_reconcile(last: Instant) -> Self {
        Self {
            hidden: HashMap::new(),
            last_reconcile: last,
        }
    }

    #[cfg(test)]
    pub fn set_last_reconcile(&mut self, t: Instant) {
        self.last_reconcile = t;
    }

    #[cfg(test)]
    pub fn force_due(&mut self) {
        self.last_reconcile = Instant::now() - Duration::from_secs(2) - RECONCILE_INTERVAL;
    }

    pub fn contains(&self, window_id: WindowId) -> bool {
        self.hidden.contains_key(&window_id)
    }

    pub fn get(&self, window_id: WindowId) -> Option<usize> {
        self.hidden.get(&window_id).copied()
    }

    pub fn len(&self) -> usize {
        self.hidden.len()
    }

    pub fn is_empty(&self) -> bool {
        self.hidden.is_empty()
    }

    /// Remove a tiled window from its workspace and remember where it came
    /// from. Returns the workspace index for the caller to `apply_layout`.
    pub fn hide_window(
        &mut self,
        window_id: WindowId,
        workspaces: &mut [Workspace],
    ) -> Option<usize> {
        let idx = find_workspace_for_window(workspaces, window_id)?;
        workspaces[idx].remove_window(window_id);
        self.hidden.insert(window_id, idx);
        Some(idx)
    }

    /// Forget a hidden entry and return its remembered workspace index, if any.
    /// The caller decides the final `preferred` index (falls back to
    /// `routed_workspace_idx` / `active_workspace_idx`) and calls
    /// `add_window_to_workspace`.
    pub fn take_hidden(&mut self, window_id: WindowId) -> Option<usize> {
        self.hidden.remove(&window_id)
    }

    pub fn remove(&mut self, window_id: WindowId) -> Option<usize> {
        self.hidden.remove(&window_id)
    }

    pub fn insert(&mut self, window_id: WindowId, idx: usize) -> Option<usize> {
        self.hidden.insert(window_id, idx)
    }

    /// True when `RECONCILE_INTERVAL` has elapsed since the last reconcile.
    /// When true, also advances the timestamp so the next call is debounced.
    pub fn should_reconcile(&mut self, now: Instant) -> bool {
        if now.duration_since(self.last_reconcile) >= RECONCILE_INTERVAL {
            self.last_reconcile = now;
            true
        } else {
            false
        }
    }

    /// Compute the two reconcile sets without mutating anything. The caller is
    /// responsible for iterating the returned vectors and calling `hide_window`
    /// / `add_window_to_workspace`.
    ///
    /// `is_hidden` is the `OsAdapter::window_is_hidden` predicate in prod
    /// (`|wid| os.window_is_hidden(wid)`); tests pass a closure over a hash set.
    pub fn pending_for_reconcile<F>(
        &self,
        window_pids: &HashMap<WindowId, i32>,
        workspaces: &[Workspace],
        is_hidden: F,
    ) -> (Vec<WindowId>, Vec<WindowId>)
    where
        F: Fn(WindowId) -> bool,
    {
        let mut to_hide = Vec::new();
        let mut to_show = Vec::new();
        for &wid in window_pids.keys() {
            let hidden = is_hidden(wid);
            if hidden && find_workspace_for_window(workspaces, wid).is_some() {
                to_hide.push(wid);
            } else if !hidden && self.hidden.contains_key(&wid) {
                to_show.push(wid);
            }
        }
        (to_hide, to_show)
    }
}

impl Default for HiddenTracker {
    fn default() -> Self {
        Self::new()
    }
}

fn find_workspace_for_window(workspaces: &[Workspace], window_id: WindowId) -> Option<usize> {
    workspaces
        .iter()
        .position(|ws| ws.find_window(window_id).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pengwm_core::workspace::Workspace;

    fn ws_with_windows(ids: &[WindowId]) -> Vec<Workspace> {
        let mut ws = Workspace::new("ws".into(), 1, (0, 0), (1920, 1080));
        for &id in ids {
            ws.add_window(id, None);
        }
        vec![ws]
    }

    #[test]
    fn hide_window_removes_from_workspace_and_remembers_idx() {
        let mut workspaces = ws_with_windows(&[10, 20]);
        let mut t = HiddenTracker::new();
        let idx = t.hide_window(10, &mut workspaces).unwrap();
        assert_eq!(idx, 0);
        assert!(workspaces[0].find_window(10).is_none());
        assert_eq!(t.get(10), Some(0));
    }

    #[test]
    fn hide_window_returns_none_when_not_tiled() {
        let mut workspaces = ws_with_windows(&[10]);
        let mut t = HiddenTracker::new();
        assert!(t.hide_window(99, &mut workspaces).is_none());
        assert!(t.is_empty());
    }

    #[test]
    fn take_hidden_returns_remembered_and_clears() {
        let mut t = HiddenTracker::new();
        t.insert(10, 2);
        assert_eq!(t.take_hidden(10), Some(2));
        assert!(!t.contains(10));
    }

    #[test]
    fn pending_to_hide_when_predicate_says_hidden_and_tiled() {
        let workspaces = ws_with_windows(&[1, 2]);
        let mut pids = HashMap::new();
        pids.insert(1, 42);
        pids.insert(2, 42);
        let t = HiddenTracker::new();
        let is_hidden = |wid| wid == 1;
        let (to_hide, to_show) = t.pending_for_reconcile(&pids, &workspaces, is_hidden);
        assert_eq!(to_hide, vec![1]);
        assert!(to_show.is_empty());
    }

    #[test]
    fn pending_to_show_when_hidden_map_has_entry_but_predicate_says_visible() {
        let workspaces = ws_with_windows(&[2]);
        let mut pids = HashMap::new();
        pids.insert(1, 42);
        pids.insert(2, 42);
        let mut t = HiddenTracker::new();
        // 1 is hidden-tracked but not tiled; predicate says visible -> should retile
        t.insert(1, 0);
        let is_hidden = |_| false;
        let (to_hide, to_show) = t.pending_for_reconcile(&pids, &workspaces, is_hidden);
        assert!(to_hide.is_empty());
        assert_eq!(to_show, vec![1]);
    }

    #[test]
    fn pending_hides_all_windows_when_app_hidden() {
        let workspaces = ws_with_windows(&[10, 20]);
        let mut pids = HashMap::new();
        pids.insert(10, 42);
        pids.insert(20, 42);
        let t = HiddenTracker::new();
        let is_hidden = |_| true;
        let (to_hide, _) = t.pending_for_reconcile(&pids, &workspaces, is_hidden);
        assert_eq!(to_hide.len(), 2);
    }

    #[test]
    fn should_reconcile_debounces_by_interval() {
        let now = Instant::now();
        let mut t = HiddenTracker::with_last_reconcile(now);
        assert!(!t.should_reconcile(now));
        assert!(t.should_reconcile(now + Duration::from_secs(2)));
        assert!(!t.should_reconcile(now + Duration::from_secs(2) + Duration::from_millis(100)));
    }
}
