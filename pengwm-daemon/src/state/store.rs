use std::collections::HashMap;
use std::time::{Duration, Instant};

use pengwm_core::tree::WindowId;
use pengwm_core::workspace::Workspace;

const RECONCILE_INTERVAL: Duration = Duration::from_secs(1);

/// Why a window left the tiling tree while staying tracked. `Hide` is the
/// minimize/app-hide path (cmd-H, dock) — it reveals through the normal
/// shown flow; `Fullscreen` is the native-fullscreen probe — it reveals as
/// a plain tileable window the moment the `AXFullScreen` probe says so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParkReason {
    /// Minimized to the Dock or hidden in place (`AXMinimized`/`AXHidden`).
    Hide,
    /// Native fullscreen (`AXFullScreen`) — macOS owns the geometry now.
    Fullscreen,
}

/// What a parked window remembers: the workspace index it came from,
/// whether it parks as a popup (dialogs / floating / restricted apps) or as
/// a tree window, and the reason it left the tree. One parking map, one
/// reveal funnel for both kinds — the kind bit changes which membership the
/// caller re-adds to; the reason bit only changes *when* the caller is
/// allowed to fire the reveal (hidden windows wait for `window_is_hidden`,
/// fullscreen windows for its probe to falsify).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParkedEntry {
    pub idx: usize,
    pub popup: bool,
    pub reason: ParkReason,
}

/// Single owner of wid↔pid↔parked state. Parked windows leave the tiling
/// tree but stay in `window_pids` so they can be retiled by
/// `on_window_shown` (hide) or `on_fullscreen_exit` (fullscreen); the
/// `parked` map remembers the workspace index they came from (and kind +
/// reason), so the reveal funnel can route them back. Finding, remembering,
/// and removal split at the seam: the store finds + remembers and returns
/// the index; the caller (`StateManager`) mutates the workspace and
/// re-layouts. Reconcile stays testable via a `Fn(WindowId)->bool`
/// predicate (no `as_any_mut` downcast). This is the deep module that
/// concentrates the "parked means untiled but still tracked" invariant.
pub struct WindowStore {
    window_pids: HashMap<WindowId, i32>,
    pid_to_windows: HashMap<i32, Vec<WindowId>>,
    parked: HashMap<WindowId, ParkedEntry>,
    last_reconcile: Instant,
}

impl WindowStore {
    pub fn new() -> Self {
        Self {
            window_pids: HashMap::new(),
            pid_to_windows: HashMap::new(),
            parked: HashMap::new(),
            last_reconcile: Instant::now(),
        }
    }

    #[cfg(test)]
    pub fn with_last_reconcile(last: Instant) -> Self {
        Self {
            window_pids: HashMap::new(),
            pid_to_windows: HashMap::new(),
            parked: HashMap::new(),
            last_reconcile: last,
        }
    }

    // -- pid maps -----------------------------------------------------------

    pub fn register(&mut self, window_id: WindowId, pid: i32) {
        if let std::collections::hash_map::Entry::Vacant(e) = self.window_pids.entry(window_id) {
            e.insert(pid);
            self.pid_to_windows.entry(pid).or_default().push(window_id);
        }
        self.parked.remove(&window_id);
    }

    pub fn unregister(&mut self, window_id: WindowId) -> Option<i32> {
        let pid = self.window_pids.remove(&window_id)?;
        self.parked.remove(&window_id);
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
        self.parked.remove(&window_id);
    }

    pub fn remove_pid(&mut self, pid: i32) -> Vec<WindowId> {
        let windows = self.pid_to_windows.remove(&pid).unwrap_or_default();
        for &wid in &windows {
            self.window_pids.remove(&wid);
            self.parked.remove(&wid);
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

    // -- parked state ----------------------------------------------------------

    /// True when the window is parked by a hide (minimize / app-hidden):
    /// untiled but still tracked, waiting for the shown flow.
    pub fn is_hidden(&self, window_id: WindowId) -> bool {
        self.parked_reason(window_id) == Some(ParkReason::Hide)
    }

    /// True when the window is parked native-fullscreen: untiled but still
    /// tracked, waiting for the `window_is_fullscreen` probe to falsify.
    pub fn is_fullscreen_parked(&self, window_id: WindowId) -> bool {
        self.parked_reason(window_id) == Some(ParkReason::Fullscreen)
    }

    /// True whether the window is parked for any reason. Callers that must
    /// ignore a window regardless of *why* it left the tree (drag/move
    /// handling, focus tracking) use this; callers that care which reveal
    /// flow applies use `is_hidden` / `is_fullscreen_parked`.
    pub fn is_parked(&self, window_id: WindowId) -> bool {
        self.parked.contains_key(&window_id)
    }

    fn parked_reason(&self, window_id: WindowId) -> Option<ParkReason> {
        self.parked.get(&window_id).map(|e| e.reason)
    }

    /// Remember a leave-the-tree transition and report the index. The caller
    /// removes the window from the tree and re-layouts — tree mutation stays
    /// with the tree owner; the store only owns parked state. `None` when the
    /// window is not a tree member (popups park via `park_popup`).
    pub fn park(
        &mut self,
        window_id: WindowId,
        workspaces: &[Workspace],
        reason: ParkReason,
    ) -> Option<usize> {
        let idx = find_workspace_for_window(workspaces, window_id)?;
        self.parked.insert(
            window_id,
            ParkedEntry {
                idx,
                popup: false,
                reason,
            },
        );
        Some(idx)
    }

    /// Remember a parked popup's workspace. Popups need no capacity or
    /// routing: they re-attach to the same workspace on reveal. Fullscreen
    /// parking never applies — popups sit outside the tree by definition.
    pub fn park_popup(&mut self, window_id: WindowId, idx: usize) {
        self.parked.insert(
            window_id,
            ParkedEntry {
                idx,
                popup: true,
                reason: ParkReason::Hide,
            },
        );
    }

    /// A parked-fullscreen window was reported hidden (minimized with its
    /// Space active / app Cmd-H): upgrade the reason, keep the remembered
    /// idx. Fullscreen-exit no longer reveals this window — the normal
    /// hidden flow owns it. Never promotes Hide → Fullscreen.
    pub fn upgrade_park_to_hide(&mut self, window_id: WindowId) {
        if let Some(entry) = self.parked.get_mut(&window_id) {
            if entry.reason == ParkReason::Fullscreen {
                entry.reason = ParkReason::Hide;
            }
        }
    }

    /// Forget a parked entry and return what it remembered (workspace index,
    /// popup kind, reason), if any. The caller decides the final placement:
    /// popups re-attach to the remembered workspace; tree windows go through
    /// `add_window_to_workspace` (the remembered idx is the preferred slot —
    /// overflow beyond it reuses the normal capacity routing).
    pub fn reveal(&mut self, window_id: WindowId) -> Option<ParkedEntry> {
        self.parked.remove(&window_id)
    }

    #[cfg(test)]
    pub fn park_insert(&mut self, window_id: WindowId, idx: usize) {
        self.parked.insert(
            window_id,
            ParkedEntry {
                idx,
                popup: false,
                reason: ParkReason::Hide,
            },
        );
    }

    /// Test helper: reset the reconcile debounce so the next tick runs the
    /// hidden-state reconcile immediately.
    #[cfg(test)]
    pub fn force_reconcile_for_test(&mut self) {
        self.last_reconcile = Instant::now() - RECONCILE_INTERVAL;
    }

    /// Test helper: seed a parked popup entry.
    #[cfg(test)]
    pub fn park_insert_popup(&mut self, window_id: WindowId, idx: usize) {
        self.parked.insert(
            window_id,
            ParkedEntry {
                idx,
                popup: true,
                reason: ParkReason::Hide,
            },
        );
    }

    #[cfg(test)]
    pub fn fullscreen_park_insert(&mut self, window_id: WindowId, idx: usize) {
        self.parked.insert(
            window_id,
            ParkedEntry {
                idx,
                popup: false,
                reason: ParkReason::Fullscreen,
            },
        );
    }

    /// Drain every parked entry with what it remembered. The caller retiles
    /// each (remembered, routed, or active workspace; popups re-attach).
    pub fn reveal_all(&mut self) -> Vec<(WindowId, ParkedEntry)> {
        self.parked.drain().collect()
    }

    /// Snapshot of every fullscreen-parked window id, for the per-tick exit
    /// probe. The parked set is tiny by construction, so the probe is cheap.
    pub fn fullscreen_parked(&self) -> Vec<WindowId> {
        self.parked
            .iter()
            .filter(|(_, e)| e.reason == ParkReason::Fullscreen)
            .map(|(&wid, _)| wid)
            .collect()
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
    /// responsible for iterating the returned vectors and calling `hide` /
    /// `add_window_to_workspace` (tree) or `hide_popup` / `add_popup`.
    ///
    /// Membership counts a window that is tiled *or* a workspace-bound popup,
    /// so missed app-hide events on popups reconcile like tree windows.
    /// `is_hidden` is the `OsAdapter::window_is_hidden` predicate in prod
    /// (`|wid| os.window_is_hidden(wid)`); tests pass a closure over a hash set.
    pub fn pending_for_reconcile<F>(
        &self,
        workspaces: &[Workspace],
        is_hidden: F,
    ) -> (Vec<WindowId>, Vec<WindowId>)
    where
        F: Fn(WindowId) -> bool,
    {
        let mut to_hide = Vec::new();
        let mut to_show = Vec::new();
        for &wid in self.window_pids.keys() {
            let hidden = is_hidden(wid);
            let member = find_workspace_for_window(workspaces, wid).is_some()
                || workspaces.iter().any(|ws| ws.is_popup(wid));
            if hidden && member {
                to_hide.push(wid);
            } else if !hidden && self.is_hidden(wid) {
                // Hide-parked only: fullscreen-parked windows leave this
                // reconcile — their reveal is owned by the fullscreen exit
                // probe (`reconcile_fullscreen_windows`), which must not be
                // shortcut by the `window_is_hidden` predicate saying "not
                // hidden".
                to_show.push(wid);
            }
        }
        (to_hide, to_show)
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

fn find_workspace_for_window(workspaces: &[Workspace], window_id: WindowId) -> Option<usize> {
    workspaces
        .iter()
        .position(|ws| ws.find_window(window_id).is_some())
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
    fn park_and_reveal_roundtrip() {
        let mut wss = ws_with(&[10, 20]);
        let mut r = WindowStore::new();
        r.register(10, 42);
        r.register(20, 42);
        let idx = r.park(10, &wss, ParkReason::Hide).unwrap();
        assert_eq!(idx, 0);
        assert!(r.is_hidden(10));
        // Tree removal is the caller's job.
        assert!(wss[0].find_window(10).is_some());
        wss[0].remove_window(10);
        assert!(wss[0].find_window(10).is_none());
        let remembered = r.reveal(10).unwrap();
        assert_eq!(remembered.idx, 0);
        assert!(!remembered.popup);
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
    fn park_window_remembers_idx_without_mutating_tree() {
        let workspaces = ws_with_windows(&[10, 20]);
        let mut t = WindowStore::new();
        t.register(10, 42);
        t.register(20, 42);
        let idx = t.park(10, &workspaces, ParkReason::Hide).unwrap();
        assert_eq!(idx, 0);
        assert!(workspaces[0].find_window(10).is_some());
        assert!(t.is_hidden(10));
    }

    #[test]
    fn park_window_returns_none_when_not_tiled() {
        let workspaces = ws_with_windows(&[10]);
        let mut t = WindowStore::new();
        assert!(t.park(99, &workspaces, ParkReason::Hide).is_none());
        assert!(!t.is_hidden(99));
    }

    #[test]
    fn take_hidden_returns_remembered_and_clears() {
        let mut t = WindowStore::new();
        t.park_insert(10, 2);
        assert_eq!(t.reveal(10).map(|e| e.idx), Some(2));
        assert!(!t.is_hidden(10));
    }

    #[test]
    fn popup_hidden_entries_roundtrip_with_kind() {
        let mut t = WindowStore::new();
        t.park_insert_popup(10, 1);
        let entry = t.reveal(10).unwrap();
        assert_eq!(entry.idx, 1);
        assert!(entry.popup, "popup kind survives the parked map");
    }

    #[test]
    fn fullscreen_parking_roundtrips_with_reason() {
        let mut wss = ws_with(&[10]);
        let mut t = WindowStore::new();
        t.register(10, 42);
        let idx = t.park(10, &wss, ParkReason::Fullscreen).unwrap();
        assert_eq!(idx, 0);
        assert!(t.is_fullscreen_parked(10));
        assert!(!t.is_hidden(10));
        assert!(t.is_parked(10));
        wss[0].remove_window(10);
        let entry = t.reveal(10).unwrap();
        assert_eq!(entry.idx, 0);
        assert!(!entry.popup);
        assert_eq!(entry.reason, ParkReason::Fullscreen);
        assert!(t.fullscreen_parked().is_empty());
    }

    #[test]
    fn upgrade_to_hide_moves_the_park_reason() {
        let mut t = WindowStore::new();
        t.fullscreen_park_insert(10, 3);
        t.upgrade_park_to_hide(10);
        assert!(!t.is_fullscreen_parked(10));
        assert!(t.is_hidden(10));
        assert!(
            t.reveal(10).is_some_and(|e| e.idx == 3 && e.reason == ParkReason::Hide),
            "idx survives the upgrade"
        );
    }

    #[test]
    fn upgrade_to_hide_never_promotes_a_hidden_window() {
        let mut t = WindowStore::new();
        t.park_insert(10, 1);
        t.upgrade_park_to_hide(10);
        let entry = t.reveal(10).unwrap();
        assert_eq!(entry.reason, ParkReason::Hide);
    }

    #[test]
    fn fullscreen_parked_snapshot_is_scoped_to_the_reason() {
        let mut t = WindowStore::new();
        t.fullscreen_park_insert(10, 0);
        t.park_insert(20, 0);
        let snapshot = t.fullscreen_parked();
        assert_eq!(snapshot, vec![10]);
        assert!(t.is_parked(20));
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
        pids.park_insert(1, 0);
        let is_hidden = |_| false;
        let (to_hide, to_show) = pids.pending_for_reconcile(&workspaces, is_hidden);
        assert!(to_hide.is_empty());
        assert_eq!(to_show, vec![1]);
    }

    #[test]
    fn fullscreen_parked_windows_never_enter_the_hidden_reveal_set() {
        let workspaces = ws_with_windows(&[]);
        let mut pids = WindowStore::new();
        pids.register(1, 42);
        pids.fullscreen_park_insert(1, 0);
        let is_hidden = |_| false;
        let (to_hide, to_show) = pids.pending_for_reconcile(&workspaces, is_hidden);
        assert!(to_hide.is_empty());
        assert!(to_show.is_empty(), "fullscreen exit is the probe's job, not the hidden reconcile");
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
