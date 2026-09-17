use crate::adapter::OsAdapter;
use crate::bar_server::BarSender;
use crate::config::keybinds::KeybindConfig;
use crate::config::Settings;
use crate::event_loop::DaemonEvent;
use pengwm_core::command::{BarMessage, BarState, BarWorkspace};
use pengwm_core::layout::Rect;
use pengwm_core::tree::WindowId;
use pengwm_core::workspace::Workspace;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

pub mod bar;
pub mod bootstrap;
pub mod commands;
pub mod display;
pub mod drag;
pub mod hidden;
pub mod router;
pub mod session;
pub mod store;
#[cfg(test)]
mod tests;
use self::bar::{BarReserve, ReloadAction};
use self::display::DisplaySet;
use self::drag::{DragState, DragTickAction};
use self::store::WindowStore;

pub struct StateManager {
    workspaces: Vec<Workspace>,
    displays: DisplaySet,
    frontmost_pid: Option<i32>,
    store: WindowStore,
    os: Box<dyn OsAdapter>,
    event_tx: mpsc::Sender<DaemonEvent>,
    gap_outer: f64,
    gap_inner: f64,
    keybinds: Arc<Mutex<KeybindConfig>>,
    restricted_apps: Vec<String>,
    bar_sender: BarSender,
    bar: BarReserve,
    /// Pids whose windows are never managed by the WM — currently the spawned
    /// `pengwm-bar` process, whose window must not be tiled.
    excluded_pids: Vec<i32>,
    last_layout_rects: HashMap<WindowId, Rect>,
    /// Last rect successfully pushed to the OS per window (tiles and hides),
    /// with the write time. `apply_layout` skips windows already at their
    /// target so redundant layouts don't hammer the AX API — Firefox reflows
    /// on every write and visibly crawls under the repeat storm, while native
    /// apps shrug it off. Only updated on success so failed writes retry.
    /// Entries are invalidated by `on_window_moved` when the window is
    /// genuinely displaced (user drag, app move) so the next layout
    /// re-asserts — without this, drag snap-back would compare equal and
    /// wrongly skip.
    applied_rects: HashMap<WindowId, (Rect, Instant)>,
    drag: DragState,
    /// Set when `Command::Quit` is handled; the event loop polls this and
    /// returns so the daemon process can exit.
    shutdown_requested: bool,
    /// Deadline until which `on_window_focused` will not mutate `DisplaySet::active`.
    /// Set on explicit workspace switches to prevent focus-notification feedback loops
    /// that drag windows along to the new workspace.
    switch_debounce_until: Option<Instant>,
    cached_hidden_strategy: Option<crate::config::HiddenStrategy>,
    focus_first_on_switch: bool,
    /// Last time `on_tick` swept `poll_windows_for_pid` for windows whose
    /// `WindowCreated` notification was missed (Firefox tear-offs / incognito:
    /// no AX notification, or a transient non-manageable subrole at creation).
    /// Throttled so the AX query storm doesn't run on every tick.
    last_window_sweep: Instant,
}

impl StateManager {
    pub fn new(
        event_tx: mpsc::Sender<DaemonEvent>,
        keybinds: Arc<Mutex<KeybindConfig>>,
        mut os: Box<dyn OsAdapter>,
        bar_sender: BarSender,
        bar_pid: Option<i32>,
        excluded_pids: Vec<i32>,
    ) -> Self {
        let display_infos = os.active_displays();
        let mut store = WindowStore::new();

        let settings = Settings::load();
        // Try session restore (topology + active + gaps) when enabled.
        // Skipped in `cargo test` so unit tests start from a deterministic
        // fresh state; session restore is exercised via `session::tests`.
        let maybe_session = if settings.restore_last_session && !cfg!(test) {
            session::load_default()
        } else {
            None
        };

        let primary_id = os.primary_display_id();
        let assembled = crate::state::bootstrap::assemble(
            &display_infos,
            primary_id,
            &settings,
            maybe_session.as_ref(),
        );
        let workspaces = assembled.workspaces;
        let displays = assembled.displays;
        let use_session = assembled.use_session;
        let entries = assembled.entries;
        let (gap_outer, gap_inner) = (assembled.gap_outer, assembled.gap_inner);

        if !use_session && !cfg!(test) {
            crate::state::bootstrap::maybe_autostart(&entries, &display_infos, use_session);
        }

        let frontmost_pid = os.frontmost_pid();

        for pid in os.running_app_pids() {
            os.attach_observer(pid);
            for window_id in os.poll_windows_for_pid(pid) {
                store.register(window_id, pid);
                let _ = event_tx.try_send(DaemonEvent::WindowCreated(window_id, pid));
            }
        }

        let bar_spawned = bar_pid.is_some();
        let bar = BarReserve::new(settings.bar.clone(), bar_spawned);
        let excluded_pids = bar_pid.into_iter().chain(excluded_pids).collect::<Vec<_>>();
        let mut state = Self {
            workspaces,
            displays,
            frontmost_pid,
            store,
            os,
            event_tx,
            gap_outer,
            gap_inner,
            keybinds,
            restricted_apps: settings.restricted_apps,
            bar_sender,
            bar,
            excluded_pids,
            last_layout_rects: HashMap::new(),
            applied_rects: HashMap::new(),
            drag: DragState::new(),
            shutdown_requested: false,
            switch_debounce_until: None,
            cached_hidden_strategy: Some(settings.windows.hidden_strategy),
            focus_first_on_switch: settings.focus_first_on_switch,
            last_window_sweep: Instant::now(),
        };

        // Hide every workspace that isn't the active one for its monitor.
        // On fresh init that's all i != 0; on session restore it's the saved active set.
        let active_set: std::collections::HashSet<usize> =
            state.displays.active().values().copied().collect();
        for i in 0..state.workspaces.len() {
            if !active_set.contains(&i) {
                state.hide_workspace(i);
            }
        }

        state.apply_bar_reservation();
        state.bar_sender.send(if state.bar.is_visible() {
            BarMessage::Show
        } else {
            BarMessage::Hide
        });
        state.publish_bar_state();
        state
    }

    fn active_workspace_idx(&self) -> usize {
        self.displays.active_workspace_idx(
            &self.workspaces,
            self.store.all_pids(),
            self.frontmost_pid,
        )
    }

    pub fn on_window_created(&mut self, window_id: WindowId, pid: i32) {
        if self.excluded_pids.contains(&pid) {
            log::debug!("Ignoring window {} from excluded pid {}", window_id, pid);
            return;
        }
        self.store.register(window_id, pid);
        if self.find_workspace_for_window(window_id).is_some() {
            return;
        }

        let active = self.active_workspace_idx();
        let preferred = self
            .displays
            .routed_workspace_idx(pid, &self.workspaces, active, &*self.os)
            .unwrap_or(active);
        self.add_window_to_workspace(window_id, pid, preferred);
        self.publish_bar_state();
    }

    /// Route `window_id` into a workspace and retile. Prefers `preferred`,
    /// overflowing to the next workspace with capacity. Returns the target
    /// workspace index, or `None` when every workspace is full.
    fn add_window_to_workspace(
        &mut self,
        window_id: WindowId,
        pid: i32,
        preferred: usize,
    ) -> Option<usize> {
        if self.workspaces[preferred].find_window(window_id).is_some() {
            return Some(preferred);
        }
        let target = match self.displays.target_with_room(&self.workspaces, preferred) {
            Some(idx) => {
                if idx != preferred {
                    log::info!(
                        "Workspace {} full ({} >= {}), routing new window to workspace {}",
                        preferred,
                        self.workspaces[preferred].window_count(),
                        self.displays.max_tiles(),
                        idx
                    );
                }
                idx
            }
            None => {
                log::warn!(
                    "All workspaces at capacity ({}), leaving window {} untracked",
                    self.displays.max_tiles(),
                    window_id
                );
                return None;
            }
        };

        let ws = &mut self.workspaces[target];
        if !self.restricted_apps.is_empty() {
            if let Some(bundle_id) = self.os.app_bundle_id(pid) {
                if self.restricted_apps.contains(&bundle_id) {
                    log::info!("App {} is restricted — enabling monocle", bundle_id);
                    ws.monocle = true;
                }
            }
        }

        ws.add_window(window_id, None);
        if self.displays.is_visible(target, &self.workspaces) {
            self.apply_layout(target);
        }
        Some(target)
    }

    pub fn on_window_destroyed(&mut self, window_id: WindowId) {
        for i in 0..self.workspaces.len() {
            if self.workspaces[i].find_window(window_id).is_some() {
                let is_visible = self.displays.is_visible(i, &self.workspaces);
                let ws = &mut self.workspaces[i];
                ws.remove_window(window_id);
                if is_visible {
                    self.apply_layout(i);
                }
                break;
            }
        }
        self.store.unregister(window_id);
        self.applied_rects.remove(&window_id);
        self.publish_bar_state();
    }

    /// A window was minimized or its app hidden: drop it from the tree (like a
    /// close) so it stops occupying tiled space, but keep pid tracking so it
    /// can be retiled when it becomes visible again.
    pub fn on_window_hidden(&mut self, window_id: WindowId) {
        if let Some(idx) = self.store.hide(window_id, &mut self.workspaces) {
            if self.displays.is_visible(idx, &self.workspaces) {
                self.apply_layout(idx);
            }
            self.publish_bar_state();
        } else {
            log::debug!("WindowHidden: window {} not tracked, ignoring", window_id);
        }
    }

    /// A window was deminiaturized or its app unhidden: retile it back into
    /// the workspace it came from (or the active one).
    pub fn on_window_shown(&mut self, window_id: WindowId) {
        if self.find_workspace_for_window(window_id).is_some() {
            return;
        }
        let pid = match self.store.pid_for(window_id) {
            Some(p) => p,
            None => {
                log::debug!("WindowShown: unknown window {}, ignoring", window_id);
                return;
            }
        };
        let remembered = self.store.reveal(window_id);
        let preferred = remembered
            .filter(|&idx| idx < self.workspaces.len())
            .unwrap_or_else(|| {
                self.displays
                    .routed_workspace_idx(
                        pid,
                        &self.workspaces,
                        self.active_workspace_idx(),
                        &*self.os,
                    )
                    .unwrap_or_else(|| self.active_workspace_idx())
            });
        if self
            .add_window_to_workspace(window_id, pid, preferred)
            .is_some()
        {
            self.publish_bar_state();
        }
    }

    pub fn on_window_focused(&mut self, window_id: WindowId) {
        // Ignore focus from windows parked at the bottom-edge hidden rect —
        // clamped title bars retain hit-testing and would otherwise steal focus
        // or flip DisplaySet::active.
        if self.store.is_hidden(window_id) {
            log::debug!("on_window_focused: ignoring hidden window {}", window_id);
            return;
        }
        for i in 0..self.workspaces.len() {
            if self.workspaces[i].find_window(window_id).is_some() {
                self.workspaces[i].focus_window(window_id);
                let mon_id = self.workspaces[i].monitor_id;
                // Debounce: don't mutate DisplaySet::active on focus notifications
                // that arrive immediately after an explicit workspace switch — they
                // are stale observer events for the window that was just hidden
                // and would drag the old workspace back into view.
                if let Some(until) = self.switch_debounce_until {
                    if Instant::now() < until {
                        self.publish_bar_state();
                        return;
                    } else {
                        self.switch_debounce_until = None;
                    }
                }
                let prev = self.displays.active_mut().insert(mon_id, i);
                if let Some(prev_idx) = prev {
                    if prev_idx != i {
                        self.hide_workspace(prev_idx);
                    }
                }
                self.publish_bar_state();
                return;
            }
        }
    }

    fn find_workspace_for_window(&self, window_id: WindowId) -> Option<usize> {
        self.workspaces
            .iter()
            .position(|ws| ws.find_window(window_id).is_some())
    }

    pub fn on_window_moved(&mut self, window_id: WindowId, x: f64, y: f64) {
        if self.store.is_hidden(window_id) {
            log::debug!("on_window_moved: ignoring hidden window {}", window_id);
            return;
        }
        let now = Instant::now();
        // If the window is genuinely displaced from where we put it, forget
        // the applied entry so the next layout re-asserts (snap-back, app
        // moves). Two guards keep our own writes from tripping this:
        // moves within the post-write grace window are our animation
        // settling, and moves within a few px are jitter, not a drag.
        const MOVE_GRACE: Duration = Duration::from_millis(500);
        const MOVE_EPSILON: f64 = 8.0;
        if let Some((target, written_at)) = self.applied_rects.get(&window_id) {
            let displaced =
                (x - target.x).abs() > MOVE_EPSILON || (y - target.y).abs() > MOVE_EPSILON;
            if displaced && now.duration_since(*written_at) > MOVE_GRACE {
                log::debug!(
                    "on_window_moved: window {} displaced to ({:.0},{:.0}), invalidating applied rect",
                    window_id,
                    x,
                    y
                );
                self.applied_rects.remove(&window_id);
            }
        }
        self.drag.on_moved(
            window_id,
            x,
            y,
            &self.workspaces,
            &self.last_layout_rects,
            now,
        );
    }

    pub fn on_tick(&mut self) {
        let now = Instant::now();

        // Fallback hidden reconcile via predicate — no downcast.
        if self.store.should_reconcile(now) {
            self.reconcile_hidden_windows();
        }

        // Safety net for windows whose WindowCreated notification never
        // arrived (Firefox tear-offs / incognito often don't fire it, or
        // fire while transiently non-manageable so the observer drops it).
        // `poll_windows_for_pid` already filters to manageable windows, so
        // PiP / dialog windows stay excluded here just as on the event path.
        const WINDOW_SWEEP_INTERVAL: Duration = Duration::from_secs(2);
        if now.duration_since(self.last_window_sweep) >= WINDOW_SWEEP_INTERVAL {
            self.last_window_sweep = now;
            self.reconcile_new_windows();
        }

        match self.drag.on_tick(
            now,
            &self.workspaces,
            &self.last_layout_rects,
            self.active_workspace_idx(),
        ) {
            DragTickAction::Swap {
                workspace_idx,
                drag,
                target,
            } => {
                let ws = &mut self.workspaces[workspace_idx];
                if ws.swap_windows_by_id(drag, target) {
                    self.apply_layout(workspace_idx);
                }
            }
            DragTickAction::SnapBack { workspace_idx } => {
                self.apply_layout(workspace_idx);
            }
            DragTickAction::None => {}
        }
    }

    #[allow(dead_code)]
    fn clear_drag_state(&mut self) {
        self.drag.clear();
    }

    /// Re-check every tracked window's actual hidden/minimized state and bring
    /// the tree in line: untile windows the OS reports as hidden, and retile
    /// ones that became visible again. Uses predicate injection so tests don't
    /// need `as_any_mut` downcast.
    fn reconcile_hidden_windows(&mut self) {
        // Snapshot to avoid borrow conflicts with &mut self in the loop.
        let window_ids: Vec<WindowId> = self.store.all_window_pids().keys().copied().collect();
        let (to_hide, to_show) = self.store.pending_for_reconcile(&self.workspaces, |wid| {
            self.os.window_is_hidden(wid)
        });
        // Hide first, then show — order doesn't matter but hide frees capacity.
        for wid in to_hide {
            // Only hide if still tiled (pending set already checked, but window
            // may have been destroyed between pending calc and now).
            if self.find_workspace_for_window(wid).is_some() {
                self.on_window_hidden(wid);
            }
        }
        for wid in to_show {
            // Only show if hidden-tracked; pending already checked.
            if self.store.is_hidden(wid) {
                self.on_window_shown(wid);
            }
        }
        // Keep unused variable for clarity if pending logic changes.
        let _ = window_ids;
    }

    /// Poll every running app for windows the store has never seen and tile
    /// them via the normal `on_window_created` path (routing, capacity,
    /// monocle, layout all shared — no duplicate logic). Only brand-new
    /// window ids are touched: tracked-but-untiled windows are either hidden
    /// (owned by `HiddenTracker`) or intentionally overflowed at capacity.
    fn reconcile_new_windows(&mut self) {
        let pids: Vec<i32> = self
            .os
            .running_app_pids()
            .into_iter()
            .filter(|pid| !self.excluded_pids.contains(pid))
            .collect();
        for pid in pids {
            for window_id in self.os.poll_windows_for_pid(pid) {
                if !self.store.contains(window_id) {
                    log::info!(
                        "reconcile_new_windows: discovered untracked window {} pid {}",
                        window_id,
                        pid
                    );
                    self.on_window_created(window_id, pid);
                }
            }
        }
    }

    #[cfg(test)]
    fn force_window_sweep_for_test(&mut self) {
        self.last_window_sweep = Instant::now() - Duration::from_secs(5);
    }

    pub fn on_app_launched(&mut self, pid: i32) {
        log::info!("App launched: pid={}", pid);
        if self.excluded_pids.contains(&pid) {
            log::debug!("Skipping excluded app launch: pid={}", pid);
            return;
        }
        self.os.attach_observer(pid);
        for window_id in self.os.poll_windows_for_pid(pid) {
            self.store.register(window_id, pid);
            let _ = self
                .event_tx
                .try_send(DaemonEvent::WindowCreated(window_id, pid));
        }
    }

    pub fn on_app_terminated(&mut self, pid: i32) {
        log::info!("App terminated: pid={}", pid);
        self.os.detach_observer(pid);
        let windows = self.store.remove_pid(pid);
        for window_id in windows {
            self.applied_rects.remove(&window_id);
            let _ = self
                .event_tx
                .try_send(DaemonEvent::WindowDestroyed(window_id));
        }
        self.publish_bar_state();
    }

    pub fn on_app_activated(&mut self, pid: i32) {
        log::debug!("App activated: pid={}", pid);
        self.frontmost_pid = Some(pid);
        // Fast path for tear-offs / incognito: the new window usually takes
        // focus, which fires AppActivated even when WindowCreated was missed.
        // A single-pid poll here tiles it immediately instead of waiting for
        // the 2s background sweep. Manageable-filtering still excludes PiP.
        if !self.excluded_pids.contains(&pid) {
            for window_id in self.os.poll_windows_for_pid(pid) {
                if !self.store.contains(window_id) {
                    log::info!(
                        "on_app_activated: discovered untracked window {} pid {}",
                        window_id,
                        pid
                    );
                    self.on_window_created(window_id, pid);
                }
            }
        }
        if let Some(window_id) = self.os.focused_window_for_pid(pid) {
            self.on_window_focused(window_id);
        } else {
            self.publish_bar_state();
        }
    }

    pub fn on_monitor_added(&mut self, display_id: u32) {
        if self
            .displays
            .on_added(display_id, &mut self.workspaces, &*self.os)
            .is_none()
        {
            return;
        }
        self.apply_bar_reservation();
        self.publish_bar_state();
    }

    pub fn on_monitor_removed(&mut self, _display_id: u32) {
        self.displays
            .on_removed(_display_id, &mut self.workspaces, &*self.os);
        self.apply_bar_reservation();
        self.publish_bar_state();
    }

    pub fn on_monitor_resized(&mut self, display_id: u32) {
        let affected = self
            .displays
            .on_resized(display_id, &mut self.workspaces, &*self.os);
        for idx in affected {
            self.apply_layout(idx);
        }
        self.apply_bar_reservation();
        self.publish_bar_state();
    }

    /// True once `Command::Quit` has been handled; the event loop polls this
    /// to know when to stop running.
    pub fn shutdown_requested(&self) -> bool {
        self.shutdown_requested
    }

    fn reload_config(&mut self) {
        log::info!("Reloading config...");
        let (updated_settings, updated_keybinds) = match crate::config::loader::load() {
            Ok(v) => v,
            Err(e) => {
                log::warn!("Failed to reload config: {}. Keeping previous config.", e);
                return;
            }
        };
        {
            let mut keybinds = self.keybinds.lock().expect("keybind mutex poisoned");
            *keybinds = updated_keybinds;
            log::info!("Config reloaded ({} bindings)", keybinds.bindings.len());
        }
        self.gap_outer = updated_settings.gap_outer.max(0) as f64;
        self.gap_inner = updated_settings.gap_inner.max(0) as f64;
        self.displays.set_max_tiles(updated_settings.max_tiles);
        self.restricted_apps = updated_settings.restricted_apps;
        self.cached_hidden_strategy = Some(updated_settings.windows.hidden_strategy);
        self.focus_first_on_switch = updated_settings.focus_first_on_switch;
        self.displays
            .set_entries(if updated_settings.workspaces.is_empty() {
                crate::config::default_workspaces()
            } else {
                updated_settings.workspaces.clone()
            });
        let reload_action = self.bar.on_reload(updated_settings.bar.clone());
        self.apply_layout(self.active_workspace_idx());
        self.bar_sender.send(BarMessage::Reload);
        match reload_action {
            ReloadAction::NeedsRestart => {
                log::info!(
                    "bar.enabled flipped to true at runtime; restart the daemon to spawn pengwm-bar"
                );
            }
            ReloadAction::ShouldExit => {
                log::info!("bar.enabled flipped to false; exiting pengwm-bar");
                self.bar_sender.send(BarMessage::Exit);
                self.apply_bar_reservation();
                self.publish_bar_state();
                return;
            }
            ReloadAction::Reapply => {}
        }
        self.bar_sender.send(if self.bar.is_visible() {
            BarMessage::Show
        } else {
            BarMessage::Hide
        });
        self.apply_bar_reservation();
        self.publish_bar_state();
        log::info!(
            "Config reloaded (gaps: {}/{})",
            self.gap_outer,
            self.gap_inner
        );
    }

    fn hide_workspace(&mut self, workspace_idx: usize) {
        let ws = &self.workspaces[workspace_idx];
        let window_ids = ws.all_windows();
        if window_ids.is_empty() {
            return;
        }
        let placement = match self.windows_hidden_strategy() {
            crate::config::HiddenStrategy::BottomEdge => {
                pengwm_core::layout::HidePlacement::BottomEdge(pengwm_core::layout::hidden_rect(
                    ws.monitor_origin(),
                    ws.monitor_size(),
                ))
            }
            crate::config::HiddenStrategy::FarOffscreen => {
                pengwm_core::layout::HidePlacement::FarOffscreen
            }
        };
        let placements: HashMap<WindowId, pengwm_core::layout::HidePlacement> =
            window_ids.into_iter().map(|wid| (wid, placement)).collect();
        log::debug!(
            "hide_workspace idx={} mon={} strategy={:?} placement={:?} windows={:?}",
            workspace_idx,
            ws.monitor_id,
            self.windows_hidden_strategy(),
            placement,
            placements.keys()
        );
        self.os.hide_windows(&placements);
        // Record where we put them: a hidden rect never equals a future tile
        // target, so this can't cause a wrongful skip — worst case one extra
        // write. Without it, a window hidden after being tiled would compare
        // equal to its stale tiled entry and never come back on switch-back.
        let hidden_rect = placement.rect();
        let written_at = Instant::now();
        for wid in placements.keys() {
            self.applied_rects.insert(*wid, (hidden_rect, written_at));
        }
    }

    fn windows_hidden_strategy(&self) -> crate::config::HiddenStrategy {
        self.windows_config_hidden_strategy()
    }

    fn windows_config_hidden_strategy(&self) -> crate::config::HiddenStrategy {
        self.cached_hidden_strategy.unwrap_or_default()
    }

    /// Re-tile every window currently tracked as hidden back into its
    /// remembered (or routed) workspace and clear `HiddenTracker`. Must clear
    /// the tracker so future focus/move events are not ignored for now-visible
    /// windows.
    fn reveal_all(&mut self) {
        if self.store.hidden_is_empty() {
            return;
        }
        // Snapshot keys to avoid borrow conflict with &mut self in loop.
        let hidden_ids = self.store.keys();
        for wid in hidden_ids {
            // Skip if already re-tiled via earlier iteration
            if self.find_workspace_for_window(wid).is_some() {
                self.store.hidden_remove(wid);
                continue;
            }
            // Reuse on_window_shown which does take_hidden + add_window_to_workspace
            self.on_window_shown(wid);
        }
        // Ensure any remaining entries (e.g. unknown pid) are cleared
        if !self.store.hidden_is_empty() {
            for (_, _) in self.store.drain() {}
        }
        // Re-layout visible workspaces to ensure tiling clean
        let visible: Vec<usize> = self.displays.active().values().copied().collect();
        for idx in visible {
            if idx < self.workspaces.len() {
                self.apply_layout(idx);
            }
        }
    }

    #[cfg(test)]
    fn set_hidden_strategy_for_test(&mut self, s: crate::config::HiddenStrategy) {
        self.cached_hidden_strategy = Some(s);
    }

    #[cfg(test)]
    #[allow(dead_code)]
    fn set_focus_first_for_test(&mut self, v: bool) {
        self.focus_first_on_switch = v;
    }

    fn apply_layout(&mut self, workspace_idx: usize) {
        let rects = self.workspaces[workspace_idx].layout(self.gap_inner, self.gap_outer);
        self.last_layout_rects = rects.clone();

        log::debug!(
            "apply_layout ws={} gaps_in={} out={}:",
            workspace_idx,
            self.gap_inner,
            self.gap_outer
        );
        for (&window_id, rect) in &rects {
            log::debug!(
                "  win={} -> ({:.0},{:.0}) {}x{}",
                window_id,
                rect.x,
                rect.y,
                rect.width,
                rect.height
            );
        }

        for (&window_id, rect) in &rects {
            // Skip windows already at their target — redundant AX writes are
            // what makes Firefox crawl (reflow per write).
            if self.applied_rects.get(&window_id).map(|(r, _)| r) == Some(rect) {
                continue;
            }
            match self.os.set_window_rect(window_id, *rect) {
                Ok(()) => {
                    self.applied_rects
                        .insert(window_id, (*rect, Instant::now()));
                }
                Err(e) => {
                    log::error!(
                        "apply_layout: set_window_rect failed for window {}: {}",
                        window_id,
                        e
                    );
                }
            }
        }
    }

    #[cfg(test)]
    fn age_applied_for_test(&mut self, window_id: WindowId, age: Duration) {
        if let Some((rect, _)) = self.applied_rects.get(&window_id).copied() {
            self.applied_rects
                .insert(window_id, (rect, Instant::now() - age));
        }
    }

    /// Global-coordinate rect of the bar strip on the primary display, or
    /// `None` when the bar is hidden, not spawned, or no display geometry is
    /// available. Delegates to `BarReserve` — the one place that knows the
    /// spawn gate (CONTEXT.md).
    fn bar_reserved_rect(&self) -> Option<Rect> {
        self.bar.reserved_rect(&*self.os)
    }

    /// Push the current bar strip geometry into every workspace (only
    /// workspaces on the primary display are reserved) and re-lay-out.
    fn apply_bar_reservation(&mut self) {
        let affected = self.bar.apply_reservation(&mut self.workspaces, &*self.os);
        for i in affected {
            self.apply_layout(i);
        }
    }

    /// Build a fresh `BarState` snapshot and broadcast it to the bar.
    fn publish_bar_state(&mut self) {
        let active_idx = self.active_workspace_idx();
        let active_monitor = self.workspaces[active_idx].monitor_id;
        let split_direction = self.workspaces[active_idx].focused_split_direction();

        let workspaces: Vec<BarWorkspace> = self
            .workspaces
            .iter()
            .enumerate()
            .map(|(i, ws)| {
                let is_active = ws.monitor_id == active_monitor
                    && self
                        .displays
                        .active()
                        .get(&ws.monitor_id)
                        .map(|&idx| idx == i)
                        .unwrap_or(false);
                let windows = ws
                    .all_windows()
                    .into_iter()
                    .filter_map(|wid| self.store.pid_for(wid))
                    .map(|pid| {
                        self.os
                            .app_name(pid)
                            .or_else(|| self.os.app_bundle_id(pid))
                            .unwrap_or_else(|| "unknown".into())
                    })
                    .collect();
                BarWorkspace {
                    name: ws.name.clone(),
                    monitor_id: ws.monitor_id,
                    window_count: ws.window_count(),
                    active: is_active,
                    windows,
                }
            })
            .collect();

        self.bar_sender.send(BarMessage::State(BarState {
            workspaces,
            active_workspace: active_idx,
            split_direction,
            rect: self.bar_reserved_rect(),
        }));
    }
}
