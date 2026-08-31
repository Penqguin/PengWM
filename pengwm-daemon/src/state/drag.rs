use std::collections::HashMap;
use std::time::{Duration, Instant};

use pengwm_core::layout::Rect;
use pengwm_core::tree::WindowId;
use pengwm_core::workspace::Workspace;

const SWAP_HOLD_DURATION: Duration = Duration::from_secs(2);
const DRAG_IDLE_TIMEOUT: Duration = Duration::from_millis(500);

/// Outcome of `DragState::on_tick` — the caller (StateManager) owns
/// `apply_layout` / workspace mutation, the drag tracker only decides.
#[derive(Debug, PartialEq)]
pub enum DragTickAction {
    /// Swap `drag` and `target` in their workspace.
    Swap {
        workspace_idx: usize,
        drag: WindowId,
        target: WindowId,
    },
    /// No overlap target and idle timeout elapsed — snap back to tiled layout.
    SnapBack { workspace_idx: usize },
    None,
}

/// Drag-to-swap state: the window being dragged, the window currently under
/// the cursor, and the two timers that gate the swap vs snap-back. Owns only
/// timer/window ids; `last_layout_rects` is borrowed from StateManager on each
/// call so `window_at_point` hit-testing stays local to this module.
pub struct DragState {
    drag_window: Option<WindowId>,
    overlap_target: Option<WindowId>,
    overlap_start: Option<Instant>,
    last_move: Option<Instant>,
}

impl DragState {
    pub fn new() -> Self {
        Self {
            drag_window: None,
            overlap_target: None,
            overlap_start: None,
            last_move: None,
        }
    }

    pub fn drag_window(&self) -> Option<WindowId> {
        self.drag_window
    }

    pub fn overlap_target(&self) -> Option<WindowId> {
        self.overlap_target
    }

    pub fn is_idle(&self) -> bool {
        self.drag_window.is_none()
    }

    pub fn clear(&mut self) {
        self.drag_window = None;
        self.overlap_target = None;
        self.overlap_start = None;
        self.last_move = None;
    }

    /// Called on every `WindowMoved` AX notification. Updates timers and
    /// recomputes the overlap target via `layout::window_at_point`.
    pub fn on_moved(
        &mut self,
        window_id: WindowId,
        x: f64,
        y: f64,
        workspaces: &[Workspace],
        rects: &HashMap<WindowId, Rect>,
        now: Instant,
    ) {
        self.drag_window = Some(window_id);
        self.last_move = Some(now);

        if find_workspace_for_window(workspaces, window_id).is_none() {
            return;
        }

        let Some(r) = rects.get(&window_id) else {
            return;
        };
        let cx = x + r.width / 2.0;
        let cy = y + r.height / 2.0;
        let new_target = pengwm_core::layout::window_at_point(rects, cx, cy, window_id);

        match (self.overlap_target, new_target) {
            (Some(t), Some(nt)) if t == nt => {
                // same target — swap timing handled in on_tick
            }
            (_, Some(nt)) => {
                self.overlap_target = Some(nt);
                self.overlap_start = Some(now);
            }
            (_, None) => {
                self.overlap_target = None;
                self.overlap_start = None;
            }
        }
    }

    /// Called every ~50 ms from `StateManager::on_tick`. Returns the action the
    /// caller should execute (swap or snap-back); the tracker clears itself when
    /// it returns a non-None action.
    pub fn on_tick(
        &mut self,
        now: Instant,
        workspaces: &[Workspace],
        _rects: &HashMap<WindowId, Rect>,
        active_workspace_idx: usize,
    ) -> DragTickAction {
        // Check swap hold first — runs even when move notifications stop (user
        // holds window still over a target).
        if let (Some(drag), Some(target)) = (self.drag_window, self.overlap_target) {
            if let Some(start) = self.overlap_start {
                if now.duration_since(start) >= SWAP_HOLD_DURATION {
                    if let Some(ws_idx) = find_workspace_for_window(workspaces, drag) {
                        let action = DragTickAction::Swap {
                            workspace_idx: ws_idx,
                            drag,
                            target,
                        };
                        self.clear();
                        return action;
                    }
                    self.clear();
                    return DragTickAction::None;
                }
            }
        }

        // Only snap back when there's no active overlap target — otherwise the
        // 500 ms idle timeout would kill the swap before the 2 s hold.
        if let Some(last_move) = self.last_move {
            if self.overlap_target.is_none() && now.duration_since(last_move) >= DRAG_IDLE_TIMEOUT
            {
                if self.drag_window.is_some() {
                    let action = DragTickAction::SnapBack {
                        workspace_idx: active_workspace_idx,
                    };
                    self.clear();
                    return action;
                }
                self.clear();
            }
        }

        DragTickAction::None
    }
}

impl Default for DragState {
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
    use pengwm_core::layout::Rect;
    use pengwm_core::workspace::Workspace;

    fn rect(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect { x, y, width: w, height: h }
    }

    fn two_window_workspaces_and_rects() -> (Vec<Workspace>, HashMap<WindowId, Rect>) {
        let mut ws = Workspace::new("ws".into(), 1, (0, 0), (1000, 800));
        ws.add_window(1, None);
        ws.add_window(2, None);
        // Fake tiled rects (no gaps for simplicity).
        let mut rects = HashMap::new();
        rects.insert(1, rect(0.0, 0.0, 500.0, 800.0));
        rects.insert(2, rect(500.0, 0.0, 500.0, 800.0));
        (vec![ws], rects)
    }

    #[test]
    fn on_moved_sets_overlap_when_center_hits_other_window() {
        let (wss, rects) = two_window_workspaces_and_rects();
        let mut d = DragState::new();
        let now = Instant::now();
        // Drag window 1 so its center lands in window 2's rect.
        d.on_moved(1, 400.0, 0.0, &wss, &rects, now);
        // rect[1] is 500 wide, center = 400+250=650 -> inside rect[2] (500..1000)
        assert_eq!(d.overlap_target, Some(2));
        assert_eq!(d.drag_window, Some(1));
    }

    #[test]
    fn on_moved_clears_overlap_when_no_hit() {
        let (wss, rects) = two_window_workspaces_and_rects();
        let mut d = DragState::new();
        let t0 = Instant::now();
        d.on_moved(1, 400.0, 0.0, &wss, &rects, t0);
        assert_eq!(d.overlap_target, Some(2));
        // Move far away so center misses both (rects are 0..1000, so 2000 misses).
        d.on_moved(1, 2000.0, 0.0, &wss, &rects, t0 + Duration::from_millis(10));
        assert_eq!(d.overlap_target, None);
    }

    #[test]
    fn on_tick_swap_after_hold_duration() {
        let (mut wss, rects) = two_window_workspaces_and_rects();
        let mut d = DragState::new();
        let t0 = Instant::now();
        d.on_moved(1, 400.0, 0.0, &wss, &rects, t0);
        // Before hold — no swap.
        assert_eq!(d.on_tick(t0 + Duration::from_secs(1), &wss, &rects, 0), DragTickAction::None);
        // After hold — swap.
        let action = d.on_tick(t0 + Duration::from_secs(3), &wss, &rects, 0);
        assert_eq!(
            action,
            DragTickAction::Swap {
                workspace_idx: 0,
                drag: 1,
                target: 2
            }
        );
        assert!(d.is_idle(), "drag should be cleared after swap");
        // Workspaces not yet swapped (caller does it) — verify tracker didn't mutate.
        assert_eq!(wss[0].window_count(), 2);
        let _ = &mut wss; // silence unused_mut
    }

    #[test]
    fn on_tick_snap_back_after_idle_timeout_with_no_target() {
        let (wss, rects) = two_window_workspaces_and_rects();
        let mut d = DragState::new();
        let t0 = Instant::now();
        d.on_moved(1, 2000.0, 0.0, &wss, &rects, t0);
        assert_eq!(d.overlap_target, None);
        assert_eq!(d.on_tick(t0 + Duration::from_millis(600), &wss, &rects, 0), DragTickAction::SnapBack { workspace_idx: 0 });
        assert!(d.is_idle());
    }

    #[test]
    fn idle_timeout_does_not_fire_while_overlap_pending() {
        let (wss, rects) = two_window_workspaces_and_rects();
        let mut d = DragState::new();
        let t0 = Instant::now();
        d.on_moved(1, 400.0, 0.0, &wss, &rects, t0);
        assert_eq!(d.overlap_target, Some(2));
        // 600 ms > DRAG_IDLE_TIMEOUT but overlap is active -> no snap-back.
        assert_eq!(d.on_tick(t0 + Duration::from_millis(600), &wss, &rects, 0), DragTickAction::None);
        assert!(!d.is_idle());
    }

    #[test]
    fn clear_resets_all_fields() {
        let (wss, rects) = two_window_workspaces_and_rects();
        let mut d = DragState::new();
        d.on_moved(1, 400.0, 0.0, &wss, &rects, Instant::now());
        d.clear();
        assert!(d.is_idle());
        assert_eq!(d.overlap_target, None);
    }
}
