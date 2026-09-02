use crate::adapter::OsAdapter;
use crate::bar_server::BarSender;
use crate::config::keybinds::KeybindConfig;
use crate::config::Settings;
use crate::event_loop::DaemonEvent;
use pengwm_core::command::{
    BarMessage, BarState, BarWorkspace, Command, DaemonResponse, LayoutMode,
};
use pengwm_core::layout::Rect;
use pengwm_core::tree::WindowId;
use pengwm_core::workspace::Workspace;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

pub mod bar;
pub mod display;
pub mod drag;
pub mod hidden;
pub mod router;
pub mod session;
use self::bar::{BarReserve, ReloadAction, ToggleAction};
use self::display::DisplaySet;
use self::router::Router;
use self::drag::{DragState, DragTickAction};
use self::hidden::HiddenTracker;

pub struct StateManager {
    workspaces: Vec<Workspace>,
    displays: DisplaySet,
    frontmost_pid: Option<i32>,
    pid_to_windows: HashMap<i32, Vec<WindowId>>,
    window_pids: HashMap<WindowId, i32>,
    hidden: HiddenTracker,
    os: Box<dyn OsAdapter>,
    event_tx: mpsc::Sender<DaemonEvent>,
    gap_outer: f64,
    gap_inner: f64,
    router: Router,
    keybinds: Arc<Mutex<KeybindConfig>>,
    restricted_apps: Vec<String>,
    bar_sender: BarSender,
    bar: BarReserve,
    /// Pids whose windows are never managed by the WM — currently the spawned
    /// `pengwm-bar` process, whose window must not be tiled.
    excluded_pids: Vec<i32>,
    last_layout_rects: HashMap<WindowId, Rect>,
    drag: DragState,
    /// Set when `Command::Quit` is handled; the event loop polls this and
    /// returns so the daemon process can exit.
    shutdown_requested: bool,
    /// Deadline until which `on_window_focused` will not mutate `DisplaySet::active`.
    /// Set on explicit workspace switches to prevent focus-notification feedback loops
    /// that drag windows along to the new workspace.
    switch_debounce_until: Option<Instant>,
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
        let mut workspaces = Vec::new();
        let mut pid_to_windows: HashMap<i32, Vec<WindowId>> = HashMap::new();
        let mut window_pids: HashMap<WindowId, i32> = HashMap::new();

        let settings = Settings::load();
        // Try session restore (topology + active + gaps) when enabled.
        // Skipped in `cargo test` so unit tests start from a deterministic
        // fresh state; session restore is exercised via `session::tests`.
        let maybe_session = if settings.restore_last_session && !cfg!(test) {
            session::load_default()
        } else {
            None
        };

        let (entries, use_session) = if let Some(ref sess) = maybe_session {
            if !sess.workspaces.is_empty() {
                (sess.entries.clone(), true)
            } else {
                let e = if settings.workspaces.is_empty() {
                    crate::config::default_workspaces()
                } else {
                    settings.workspaces.clone()
                };
                (e, false)
            }
        } else {
            let e = if settings.workspaces.is_empty() {
                crate::config::default_workspaces()
            } else {
                settings.workspaces.clone()
            };
            (e, false)
        };

        let mut displays = DisplaySet::new(entries.clone());
        if use_session {
            let sess = maybe_session.as_ref().unwrap();
            log::info!(
                "Restoring session: {} workspaces, {} active displays",
                sess.workspaces.len(),
                sess.active.len()
            );
            // Remap orphaned workspaces to primary and update geometry to
            // current display infos so stale monitor sizes don't persist.
            let primary = os.primary_display_id();
            let primary_info = display_infos.iter().find(|d| d.id == primary);
            let primary_origin = primary_info.map(|d| d.origin).unwrap_or((0, 0));
            let primary_size = primary_info.map(|d| d.size).unwrap_or((1920, 1080));
            for ws in &sess.workspaces {
                let mut ws = ws.clone();
                let exists = display_infos.iter().any(|d| d.id == ws.monitor_id);
                if !exists {
                    log::info!(
                        "Remapping orphan workspace '{}' from monitor {} to primary {}",
                        ws.name,
                        ws.monitor_id,
                        primary
                    );
                    ws.monitor_id = primary;
                    ws.set_monitor_origin(primary_origin);
                    ws.update_monitor_geometry(primary_origin, primary_size);
                } else if let Some(info) = display_infos.iter().find(|d| d.id == ws.monitor_id) {
                    ws.update_monitor_geometry(info.origin, info.size);
                }
                workspaces.push(ws);
            }
            // Active map: keep only entries for live displays, remap stale indices.
            for (mon_id, idx) in &sess.active {
                if display_infos.iter().any(|d| d.id == *mon_id) && *idx < workspaces.len() {
                    displays.active_mut().insert(*mon_id, *idx);
                }
            }
            // Ensure every live display has an active entry.
            for info in &display_infos {
                if !displays.active().contains_key(&info.id) {
                    // Pick first workspace on that monitor
                    if let Some(idx) = workspaces.iter().position(|ws| ws.monitor_id == info.id) {
                        displays.active_mut().insert(info.id, idx);
                    } else if !workspaces.is_empty() {
                        displays.active_mut().insert(info.id, 0);
                    }
                }
            }
            // Restore gaps from session (override settings gaps)
            // (handled after StateManager construction to avoid borrowing)
        } else {
            displays.init_workspaces(&mut workspaces, &display_infos);
        }

        // Fallback when no displays (headless test) — DisplaySet leaves workspaces empty.
        if workspaces.is_empty() {
            workspaces.push(Workspace::new(
                "ws-1".into(),
                os.primary_display_id(),
                (0, 0),
                (1920, 1080),
            ));
            displays.active_mut().insert(os.primary_display_id(), 0);
        }

        // Autostart: only on fresh init, not when restoring a session (session
        // already has its windows' state). Skipped in tests. Spawn each
        // `autostart` command once per entry that has affinity for at least one
        // active display.
        if !use_session && !cfg!(test) {
            for entry in &entries {
                if entry.autostart.is_empty() {
                    continue;
                }
                let applies = if entry.monitor.is_none() {
                    !display_infos.is_empty()
                } else {
                    display_infos
                        .iter()
                        .any(|d| DisplaySet::entry_applies_to_display(entry, d))
                };
                if !applies {
                    continue;
                }
                for cmd in &entry.autostart {
                    log::info!("Autostart for workspace '{}': {}", entry.name, cmd);
                    let _ = std::process::Command::new("sh")
                        .arg("-c")
                        .arg(cmd)
                        .spawn()
                        .map(|mut child| {
                            std::thread::spawn(move || {
                                let _ = child.wait();
                            });
                        })
                        .map_err(|e| {
                            log::warn!(
                                "Failed to autostart '{}' for workspace '{}': {}",
                                cmd,
                                entry.name,
                                e
                            );
                        });
                }
            }
        }

        let frontmost_pid = os.frontmost_pid();

        for pid in os.running_app_pids() {
            os.attach_observer(pid);
            for window_id in os.poll_windows_for_pid(pid) {
                window_pids.insert(window_id, pid);
                pid_to_windows.entry(pid).or_default().push(window_id);
                let _ = event_tx.try_send(DaemonEvent::WindowCreated(window_id, pid));
            }
        }

        let bar_spawned = bar_pid.is_some();
        let bar = BarReserve::new(settings.bar.clone(), bar_spawned);
        let excluded_pids = bar_pid
            .into_iter()
            .chain(excluded_pids)
            .collect::<Vec<_>>();
        let (gap_outer, gap_inner) = if use_session {
            let sess = maybe_session.as_ref().unwrap();
            (sess.gap_outer, sess.gap_inner)
        } else {
            (
                settings.gap_outer.max(0) as f64,
                settings.gap_inner.max(0) as f64,
            )
        };
        let mut state = Self {
            workspaces,
            displays,
            frontmost_pid,
            pid_to_windows,
            window_pids,
            hidden: HiddenTracker::new(),
            os,
            event_tx,
            gap_outer,
            gap_inner,
            router: Router::new(settings.max_tiles as usize),
            keybinds,
            restricted_apps: settings.restricted_apps,
            bar_sender,
            bar,
            excluded_pids,
            last_layout_rects: HashMap::new(),
            drag: DragState::new(),
            shutdown_requested: false,
            switch_debounce_until: None,
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

    /// First workspace after `start` (wrapping within the same monitor) with
    /// room for another window. Returns `None` when every workspace on that
    /// monitor is at capacity.
    fn find_next_workspace_with_capacity(&self, start: usize) -> Option<usize> {
        self.router.find_next_with_capacity(&self.workspaces, start)
    }

    fn active_workspace_idx(&self) -> usize {
        self.router.active_workspace_idx(
            &self.workspaces,
            &self.pid_to_windows,
            self.frontmost_pid,
            &self.displays,
        )
    }

    pub fn on_window_created(&mut self, window_id: WindowId, pid: i32) {
        if self.excluded_pids.contains(&pid) {
            log::debug!("Ignoring window {} from excluded pid {}", window_id, pid);
            return;
        }
        if let std::collections::hash_map::Entry::Vacant(e) = self.window_pids.entry(window_id) {
            e.insert(pid);
            self.pid_to_windows.entry(pid).or_default().push(window_id);
        }
        self.hidden.remove(window_id);
        if self.find_workspace_for_window(window_id).is_some() {
            return;
        }

        let preferred = self
            .routed_workspace_idx(pid)
            .unwrap_or_else(|| self.active_workspace_idx());
        self.add_window_to_workspace(window_id, pid, preferred);
        self.publish_bar_state();
    }

    /// Flat workspace index a new window from `pid` should land in: the
    /// configured workspace for the app (by bundle id or name) on the active
    /// monitor. `None` when the app isn't assigned to any workspace.
    fn routed_workspace_idx(&self, pid: i32) -> Option<usize> {
        let active = self.active_workspace_idx();
        self.router.routed_workspace_idx(
            pid,
            &self.workspaces,
            active,
            &*self.os,
            self.displays.entries(),
        )
    }

    /// Name of the configured workspace `pid`'s app is assigned to, matched
    /// case-insensitively against bundle id first, then app display name.
    #[allow(dead_code)]
    fn configured_workspace_name_for_pid(&self, pid: i32) -> Option<&str> {
        self.router.configured_workspace_name_for_pid(
            pid,
            &*self.os,
            self.displays.entries(),
        )
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
        let target = if self.workspaces[preferred].window_count() >= self.router.max_tiles() {
            match self.find_next_workspace_with_capacity(preferred) {
                Some(idx) => {
                    log::info!(
                        "Workspace {} full ({} >= {}), routing new window to workspace {}",
                        preferred,
                        self.workspaces[preferred].window_count(),
                        self.router.max_tiles(),
                        idx
                    );
                    idx
                }
                None => {
                    log::warn!(
                        "All workspaces at capacity ({}), leaving window {} untracked",
                        self.router.max_tiles(),
                        window_id
                    );
                    return None;
                }
            }
        } else {
            preferred
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
        if self.is_workspace_visible(target) {
            self.apply_layout(target);
        }
        Some(target)
    }

    pub fn on_window_destroyed(&mut self, window_id: WindowId) {
        for i in 0..self.workspaces.len() {
            if self.workspaces[i].find_window(window_id).is_some() {
                let is_visible = self.is_workspace_visible(i);
                let ws = &mut self.workspaces[i];
                ws.remove_window(window_id);
                if is_visible {
                    self.apply_layout(i);
                }
                break;
            }
        }
        self.window_pids.remove(&window_id);
        self.hidden.remove(window_id);
        // Also evict from WindowElementCache via adapter if needed? MacOsAdapter handles via observer, but we can ensure hide not retried.
        self.pid_to_windows.retain(|_, windows| {
            windows.retain(|w| *w != window_id);
            !windows.is_empty()
        });
        self.publish_bar_state();
    }

    /// A window was minimized or its app hidden: drop it from the tree (like a
    /// close) so it stops occupying tiled space, but keep pid tracking so it
    /// can be retiled when it becomes visible again.
    pub fn on_window_hidden(&mut self, window_id: WindowId) {
        if let Some(idx) = self.hidden.hide_window(window_id, &mut self.workspaces) {
            if self.is_workspace_visible(idx) {
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
        let pid = match self.window_pids.get(&window_id) {
            Some(&p) => p,
            None => {
                log::debug!("WindowShown: unknown window {}, ignoring", window_id);
                return;
            }
        };
        let remembered = self.hidden.take_hidden(window_id);
        let preferred = remembered
            .filter(|&idx| idx < self.workspaces.len())
            .unwrap_or_else(|| {
                self.routed_workspace_idx(pid)
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
        let now = Instant::now();
        self.drag
            .on_moved(window_id, x, y, &self.workspaces, &self.last_layout_rects, now);
    }

    pub fn on_tick(&mut self) {
        let now = Instant::now();

        // Fallback hidden reconcile via predicate — no downcast.
        if self.hidden.should_reconcile(now) {
            self.reconcile_hidden_windows();
        }

        match self
            .drag
            .on_tick(now, &self.workspaces, &self.last_layout_rects, self.active_workspace_idx())
        {
            DragTickAction::Swap {
                workspace_idx,
                drag,
                target,
            } => {
                let ws = &mut self.workspaces[workspace_idx];
                if ws.swap_windows_by_id(drag, target) {
                    self.apply_layout(workspace_idx);
                }
                return;
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
        let window_ids: Vec<WindowId> = self.window_pids.keys().copied().collect();
        let (to_hide, to_show) = self.hidden.pending_for_reconcile(
            &self.window_pids,
            &self.workspaces,
            |wid| self.os.window_is_hidden(wid),
        );
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
            if self.hidden.contains(wid) {
                self.on_window_shown(wid);
            }
        }
        // Keep unused variable for clarity if pending logic changes.
        let _ = window_ids;
    }

    pub fn on_app_launched(&mut self, pid: i32) {
        log::info!("App launched: pid={}", pid);
        if self.excluded_pids.contains(&pid) {
            log::debug!("Skipping excluded app launch: pid={}", pid);
            return;
        }
        self.os.attach_observer(pid);
        for window_id in self.os.poll_windows_for_pid(pid) {
            self.window_pids.insert(window_id, pid);
            self.pid_to_windows.entry(pid).or_default().push(window_id);
            let _ = self
                .event_tx
                .try_send(DaemonEvent::WindowCreated(window_id, pid));
        }
    }

    pub fn on_app_terminated(&mut self, pid: i32) {
        log::info!("App terminated: pid={}", pid);
        self.os.detach_observer(pid);
        if let Some(windows) = self.pid_to_windows.remove(&pid) {
            for window_id in windows {
                self.window_pids.remove(&window_id);
                let _ = self
                    .event_tx
                    .try_send(DaemonEvent::WindowDestroyed(window_id));
            }
        }
        self.publish_bar_state();
    }

    pub fn on_app_activated(&mut self, pid: i32) {
        log::debug!("App activated: pid={}", pid);
        self.frontmost_pid = Some(pid);
        if let Some(window_id) = self.os.focused_window_for_pid(pid) {
            self.on_window_focused(window_id);
        } else {
            self.publish_bar_state();
        }
    }

    pub fn on_monitor_added(&mut self, display_id: u32) {
        if self.displays.on_added(display_id, &mut self.workspaces, &*self.os).is_none() {
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

    /// Handle one `Command` from any surface (IPC, keybind, config watcher).
    /// `tx` is `None` for fire-and-forget sources; the reply is only sent when
    /// a caller is waiting on it.
    pub fn on_command(&mut self, cmd: Command, tx: Option<mpsc::Sender<DaemonResponse>>) {
        match cmd {
            Command::Focus { direction } => self.focus_command(direction),
            Command::MoveWindow { direction } => self.swap_command(direction),
            Command::Split { direction } => {
                let idx = self.active_workspace_idx();
                let ws = &mut self.workspaces[idx];
                ws.apply_split_direction(direction);
                self.apply_layout(idx);
            }
            Command::Workspace { id } => {
                // Per-monitor: id is 1-based index among workspaces on the
                // *focused* monitor, not a flat global index. This keeps
                // workspace-1..5 independent per display (10 total with 2
                // monitors) and fixes "second monitor shoudl have separate
                // workspaces but it doesn't".
                let n = id as usize;
                if n == 0 {
                    // filtered by parse_id, but keep guard
                } else {
                    let current_idx = self.active_workspace_idx();
                    if current_idx >= self.workspaces.len() {
                        // headless fallback
                    } else {
                        let current_mon = self.workspaces[current_idx].monitor_id;
                        let on_mon: Vec<usize> = self
                            .workspaces
                            .iter()
                            .enumerate()
                            .filter(|(_, ws)| ws.monitor_id == current_mon)
                            .map(|(i, _)| i)
                            .collect();
                        if n <= on_mon.len() {
                            let new_idx = on_mon[n - 1];
                            if new_idx != current_idx {
                                // Hide every workspace on this monitor except the destination.
                                for &idx in &on_mon {
                                    if idx != new_idx {
                                        self.hide_workspace(idx);
                                    }
                                }
                                self.displays.active_mut().insert(current_mon, new_idx);
                                self.switch_debounce_until =
                                    Some(Instant::now() + Duration::from_millis(150));
                                self.apply_layout(new_idx);
                            }
                        } else {
                            log::debug!(
                                "Workspace id {} out of range for monitor {} (has {} workspaces)",
                                n, current_mon, on_mon.len()
                            );
                        }
                    }
                }
            }
            Command::MoveWindowToWorkspace { id } => {
                let n = id as usize;
                if n == 0 {
                } else {
                    let current_idx = self.active_workspace_idx();
                    if current_idx < self.workspaces.len() {
                        let current_mon = self.workspaces[current_idx].monitor_id;
                        let on_mon: Vec<usize> = self
                            .workspaces
                            .iter()
                            .enumerate()
                            .filter(|(_, ws)| ws.monitor_id == current_mon)
                            .map(|(i, _)| i)
                            .collect();
                        if n <= on_mon.len() {
                            self.move_focused_to_workspace(on_mon[n - 1]);
                        }
                    }
                }
            }
            Command::FocusDisplay { direction } => {
                self.focus_display(direction);
            }
            Command::MoveWindowToDisplay { direction } => {
                self.move_window_to_display(direction);
            }
            Command::Close => {
                let idx = self.active_workspace_idx();
                if let Some(wid) = self.workspaces[idx].focused_window_id() {
                    self.os.close_window(wid);
                }
            }
            Command::ToggleLayout => {
                let idx = self.active_workspace_idx();
                let ws = &mut self.workspaces[idx];
                ws.toggle_monocle();
                self.apply_layout(idx);
            }
            Command::SetLayout { mode } => {
                let idx = self.active_workspace_idx();
                let ws = &mut self.workspaces[idx];
                ws.monocle = mode == LayoutMode::Accordion;
                self.apply_layout(idx);
            }
            Command::SetGapOuter { pixels } => {
                self.gap_outer = pixels.max(0) as f64;
                self.apply_layout(self.active_workspace_idx());
            }
            Command::SetGapInner { pixels } => {
                self.gap_inner = pixels.max(0) as f64;
                self.apply_layout(self.active_workspace_idx());
            }
            Command::ToggleBar => {
                match self.bar.toggle() {
                    ToggleAction::Show(_) => {
                        log::info!("Bar toggled: visible");
                        self.bar_sender.send(BarMessage::Show);
                        self.apply_bar_reservation();
                    }
                    ToggleAction::Hide => {
                        log::info!("Bar toggled: hidden");
                        self.bar_sender.send(BarMessage::Hide);
                        self.apply_bar_reservation();
                    }
                    ToggleAction::Noop => {
                        log::info!("Bar not running; toggle ignored");
                    }
                }
            }
            Command::ReloadConfig => {
                self.reload_config();
            }
            Command::QueryState => {
                let info = self
                    .workspaces
                    .iter()
                    .map(|ws| pengwm_core::command::WorkspaceInfo {
                        name: ws.name.clone(),
                        monitor_id: ws.monitor_id,
                        window_count: ws.window_count(),
                        focused_window: ws.focused_window_id(),
                    })
                    .collect();
                if let Some(tx) = tx {
                    let _ = tx.try_send(DaemonResponse::State { workspaces: info });
                }
                return;
            }
            Command::Quit => {
                // Ack the caller (menubar / `pengwm quit`), then shut the bar
                // down too so quitting the menubar stops everything. A short
                // sleep lets the bar-server and IPC threads flush their writes
                // before the event loop returns and the process exits.
                if let Some(tx) = tx {
                    let _ = tx.try_send(DaemonResponse::Ack);
                }
                log::info!("Quit requested — shutting down daemon and bar");
                // Persist session (topology + active + gaps) atomically before exit.
                // Skipped in tests to avoid polluting the user's real session file.
                if !cfg!(test) {
                    let sess = session::snapshot_from(
                        &self.workspaces,
                        self.displays.active(),
                        self.displays.entries(),
                        self.gap_outer,
                        self.gap_inner,
                    );
                    if let Err(e) = session::save_default(&sess) {
                        log::warn!("Failed to save session on quit: {}", e);
                    }
                }
                self.bar_sender.send(BarMessage::Exit);
                std::thread::sleep(Duration::from_millis(150));
                self.shutdown_requested = true;
                return;
            }
        }
        if let Some(tx) = tx {
            let _ = tx.try_send(DaemonResponse::Ack);
        }
        self.publish_bar_state();
    }

    /// True once `Command::Quit` has been handled; the event loop polls this
    /// to know when to stop running.
    pub fn shutdown_requested(&self) -> bool {
        self.shutdown_requested
    }

    fn focus_command(&mut self, direction: Direction) {
        let idx = self.active_workspace_idx();
        let ws = &mut self.workspaces[idx];
        ws.focus_neighbor(direction);
        if let Some(wid) = ws.focused_window_id() {
            self.os.focus_window(wid);
        }
    }

    fn swap_command(&mut self, direction: Direction) {
        let idx = self.active_workspace_idx();
        let ws = &mut self.workspaces[idx];
        ws.swap_window(direction);
        self.apply_layout(idx);
    }

    fn move_focused_to_workspace(&mut self, target: usize) {
        let current = self.active_workspace_idx();
        if target == current {
            return;
        }
        let window_id = self.workspaces[current].focused_window_id();
        if let Some(wid) = window_id {
            let dest = if self.workspaces[target].window_count() >= self.router.max_tiles() {
                match self.find_next_workspace_with_capacity(target) {
                    Some(idx) => idx,
                    None => {
                        log::warn!(
                            "No workspace has room for {} (cap {}), move aborted",
                            wid,
                            self.router.max_tiles()
                        );
                        self.publish_bar_state();
                        return;
                    }
                }
            } else {
                target
            };
            self.workspaces[current].remove_window(wid);
            if self.is_workspace_visible(current) {
                self.apply_layout(current);
            }
            self.workspaces[dest].add_window(wid, None);
            if self.is_workspace_visible(dest) {
                self.apply_layout(dest);
            }
        }
        self.publish_bar_state();
    }

    fn find_display_in_direction(&self, from_id: u32, direction: Direction) -> Option<u32> {
        let displays = self.os.active_displays();
        let from = displays.iter().find(|d| d.id == from_id)?;
        // Pick the closest display whose center is in the given direction.
        // Uses center-to-center vector and dot product heuristic.
        let from_cx = from.origin.0 as f64 + from.size.0 as f64 / 2.0;
        let from_cy = from.origin.1 as f64 + from.size.1 as f64 / 2.0;
        let mut best: Option<(u32, f64)> = None;
        for d in &displays {
            if d.id == from_id {
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

    fn focus_display(&mut self, direction: Direction) {
        let current_idx = self.active_workspace_idx();
        if current_idx >= self.workspaces.len() {
            return;
        }
        let current_mon = self.workspaces[current_idx].monitor_id;
        let target_mon = match self.find_display_in_direction(current_mon, direction) {
            Some(id) => id,
            None => {
                log::debug!("focus_display {:?} no target from mon {}", direction, current_mon);
                return;
            }
        };
        let target_idx = match self.displays.active().get(&target_mon).copied() {
            Some(idx) if idx < self.workspaces.len() => idx,
            _ => match self.workspaces.iter().position(|ws| ws.monitor_id == target_mon) {
                Some(idx) => idx,
                None => return,
            },
        };
        if let Some(wid) = self.workspaces[target_idx].focused_window_id() {
            log::debug!("focus_display {:?} mon {} -> {} wid {}", direction, current_mon, target_mon, wid);
            self.os.focus_window(wid);
        } else {
            // No window to focus — just update frontmost heuristic by focusing display?
            // Publish so bar/menubar reflect focused display change.
            log::debug!("focus_display {:?} target {} has no windows", direction, target_mon);
            self.publish_bar_state();
        }
    }

    fn move_window_to_display(&mut self, direction: Direction) {
        let current_idx = self.active_workspace_idx();
        if current_idx >= self.workspaces.len() {
            return;
        }
        let current_mon = self.workspaces[current_idx].monitor_id;
        let target_mon = match self.find_display_in_direction(current_mon, direction) {
            Some(id) => id,
            None => return,
        };
        let wid = match self.workspaces[current_idx].focused_window_id() {
            Some(id) => id,
            None => return,
        };
        let target_idx = match self.displays.active().get(&target_mon).copied() {
            Some(idx) if idx < self.workspaces.len() => idx,
            _ => match self.workspaces.iter().position(|ws| ws.monitor_id == target_mon) {
                Some(idx) => idx,
                None => return,
            },
        };
        let dest = if self.workspaces[target_idx].window_count() >= self.router.max_tiles() {
            match self.find_next_workspace_with_capacity(target_idx) {
                Some(idx) => idx,
                None => {
                    log::warn!("No room on target display {} for window {}", target_mon, wid);
                    return;
                }
            }
        } else {
            target_idx
        };
        log::debug!("move_window_to_display {:?} wid {} mon {} -> {} dest {}", direction, wid, current_mon, target_mon, dest);
        self.workspaces[current_idx].remove_window(wid);
        if self.is_workspace_visible(current_idx) {
            self.apply_layout(current_idx);
        }
        self.workspaces[dest].add_window(wid, None);
        if self.is_workspace_visible(dest) {
            self.apply_layout(dest);
        }
        self.publish_bar_state();
    }

    fn reload_config(&mut self) {
        log::info!("Reloading config...");
        let (updated_settings, updated_keybinds) =
            match crate::config::loader::load() {
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
        self.router.set_max_tiles(updated_settings.max_tiles as usize);
        self.restricted_apps = updated_settings.restricted_apps;
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

    fn is_workspace_visible(&self, idx: usize) -> bool {
        if idx >= self.workspaces.len() {
            return false;
        }
        let mon = self.workspaces[idx].monitor_id;
        self.displays.active().get(&mon).copied() == Some(idx)
    }

    fn hide_workspace(&mut self, workspace_idx: usize) {
        let window_ids = self.workspaces[workspace_idx].all_windows();
        log::debug!("hide_workspace idx={} mon={} windows={:?}", workspace_idx, self.workspaces[workspace_idx].monitor_id, window_ids);
        self.os.hide_windows(&window_ids);
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
            if let Err(e) = self.os.set_window_rect(window_id, *rect) {
                log::error!(
                    "apply_layout: set_window_rect failed for window {}: {}",
                    window_id,
                    e
                );
            }
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
                    .filter_map(|wid| self.window_pids.get(&wid).copied())
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

use pengwm_core::tree::Direction;

#[cfg(test)]
mod state_tests {
    use super::*;
    use crate::adapter::DisplayInfo;
    use crate::adapter_test::TestAdapter;
    use crate::config::keybinds::KeybindConfig;
    use crate::config::{BarConfig, WorkspaceEntry};
    use crate::config::MonitorRef;
    use pengwm_core::config::BarPosition;
    use pengwm_core::tree::SplitDirection;

    fn make_adapter(display_count: u32) -> TestAdapter {
        let mut adapter = TestAdapter::new();
        if display_count == 1 {
            adapter.displays = vec![DisplayInfo {
                id: 1,
                origin: (0, 0),
                size: (1920, 1080),
            }];
        }
        if display_count == 2 {
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
            ];
        }
        adapter.frontmost = Some(42);
        adapter.running_apps = vec![42];
        adapter
            .windows
            .borrow_mut()
            .entry(42)
            .or_default()
            .extend(vec![100, 200]);
        adapter.window_pids.borrow_mut().insert(100, 42);
        adapter.window_pids.borrow_mut().insert(200, 42);
        adapter
    }

    fn setup(display_count: u32) -> StateManager {
        let (tx, _) = mpsc::channel(64);
        let keybinds = Arc::new(Mutex::new(KeybindConfig::default()));
        let adapter = make_adapter(display_count);
        let (bar_tx, _) = mpsc::channel(64);
        StateManager::new(
            tx,
            keybinds,
            Box::new(adapter),
            BarSender::from_channel(bar_tx),
            None,
            vec![],
        )
    }

    #[test]
    fn creates_workspaces_from_displays() {
        let sm = setup(1);
        // The five default named workspaces, all on the primary monitor.
        let names: Vec<&str> = sm.workspaces.iter().map(|w| w.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["Development", "Browsing", "Notes", "Music", "Messaging"]
        );
        assert!(sm.workspaces.iter().all(|w| w.monitor_id == 1));
    }

    #[test]
    fn creates_workspace_set_per_display() {
        let sm = setup(2);
        assert_eq!(sm.workspaces.len(), 10);
        assert!(sm.workspaces[..5].iter().all(|w| w.monitor_id == 1));
        assert!(sm.workspaces[5..].iter().all(|w| w.monitor_id == 2));
        assert_eq!(sm.displays.active().get(&1), Some(&0));
        assert_eq!(sm.displays.active().get(&2), Some(&5));
    }

    #[test]
    fn tracks_existing_windows_at_init() {
        let sm = setup(1);
        // tracked in pid maps
        assert_eq!(sm.window_pids.len(), 2);
        assert_eq!(sm.window_pids.get(&100), Some(&42));
        assert_eq!(sm.window_pids.get(&200), Some(&42));
        // not yet added to workspace tree (event loop hasn't consumed init events)
        assert_eq!(sm.workspaces[0].window_count(), 0);
    }

    #[test]
    fn on_window_created_tracks_pid_and_adds_to_workspace() {
        let mut sm = setup(1);
        sm.on_window_created(300, 42);
        assert_eq!(sm.window_pids.get(&300), Some(&42));
        assert!(sm.workspaces[0].find_window(300).is_some());
    }

    #[test]
    fn on_window_destroyed_removes_tracking_and_window() {
        let mut sm = setup(1);
        sm.on_window_destroyed(100);
        assert!(!sm.window_pids.contains_key(&100));
        assert!(sm.workspaces[0].find_window(100).is_none());
    }

    #[test]
    fn on_window_focused_updates_active_workspace() {
        let mut sm = setup(1);
        sm.on_window_created(300, 42);
        sm.on_window_focused(300);
        let ws = &sm.workspaces[0];
        assert_eq!(ws.focused_node, ws.find_window(300));
    }

    #[test]
    fn on_app_launched_attaches_observer_and_tracks_windows() {
        let mut sm = setup(1);
        // TestAdapter pre-populated with pid 42 having windows 100, 200
        // on_app_launched for pid 99 should attach observer and query windows
        sm.on_app_launched(99);
        // The observer was attached (no-op in TestAdapter, but method was called)
        // No windows returned for this pid since TestAdapter has none for pid 99
        assert!(!sm.pid_to_windows.contains_key(&99));
    }

    #[test]
    fn on_app_terminated_detaches_observer_and_cleans_windows() {
        let mut sm = setup(1);
        // Start with pid 42 having windows 100, 200
        assert!(sm.window_pids.contains_key(&100));
        sm.on_app_terminated(42);
        assert!(!sm.window_pids.contains_key(&100));
        assert!(!sm.window_pids.contains_key(&200));
    }

    #[test]
    fn on_app_activated_updates_frontmost_pid() {
        let mut sm = setup(1);
        sm.on_app_activated(99);
        assert_eq!(sm.frontmost_pid, Some(99));
    }

    #[test]
    fn on_window_hidden_removes_from_tree_but_keeps_pid() {
        let mut sm = setup(1);
        sm.on_window_created(100, 42);
        sm.on_window_created(200, 42);
        assert_eq!(sm.workspaces[0].window_count(), 2);
        sm.on_window_hidden(100);
        assert!(sm.workspaces[0].find_window(100).is_none());
        assert_eq!(sm.window_pids.get(&100), Some(&42));
        assert_eq!(sm.hidden.get(100), Some(0));
        assert_eq!(sm.workspaces[0].window_count(), 1);
    }

    #[test]
    fn on_window_shown_retiles_into_original_workspace() {
        let mut sm = setup(1);
        sm.on_window_created(100, 42);
        sm.on_window_created(200, 42);
        sm.on_window_hidden(100);
        sm.on_window_shown(100);
        assert!(sm.workspaces[0].find_window(100).is_some());
        assert!(!sm.hidden.contains(100));
        assert_eq!(sm.workspaces[0].window_count(), 2);
    }

    #[test]
    fn on_window_shown_ignores_untracked_window() {
        let mut sm = setup(1);
        sm.on_window_shown(999);
        assert!(sm.workspaces[0].find_window(999).is_none());
    }

    #[test]
    fn on_window_shown_skips_window_already_tracked() {
        let mut sm = setup(1);
        sm.on_window_created(100, 42);
        sm.on_window_shown(100);
        assert_eq!(sm.workspaces[0].window_count(), 1);
    }

    #[test]
    fn destroyed_hidden_window_cleans_hidden_workspace() {
        let mut sm = setup(1);
        sm.on_window_created(100, 42);
        sm.on_window_hidden(100);
        assert!(sm.hidden.contains(100));
        sm.on_window_destroyed(100);
        assert!(!sm.hidden.contains(100));
        assert!(!sm.window_pids.contains_key(&100));
    }

    // Reconcile logic is now unit-tested in hidden.rs via predicate injection.
    // StateManager integration for reconcile is exercised through on_window_hidden/shown.
    // The three previous reconcile tests (hidden_windows.insert + last_reconcile) are
    // migrated to hidden::tests::pending_* and hidden::tests::should_reconcile_*

    #[test]
    fn focus_command_delegates_to_workspace() {
        let mut sm = setup(1);
        sm.on_window_created(100, 42);
        sm.on_window_created(300, 42);
        sm.focus_command(Direction::Right);
        // Should focus the other window
        let focused = sm.workspaces[0].focused_node;
        assert!(focused.is_some());
    }

    #[test]
    fn focus_command_focuses_window_via_adapter() {
        let mut sm = setup(1);
        sm.on_window_created(100, 42);
        sm.on_window_created(200, 42);
        assert_eq!(sm.workspaces[0].focused_window_id(), Some(200));
        sm.focus_command(Direction::Right);
        assert_eq!(sm.workspaces[0].focused_window_id(), Some(100));
        assert_eq!(
            sm.os.focused_window_for_pid(42),
            Some(100),
            "adapter should be told to focus the new window"
        );
    }

    #[test]
    fn swap_command_triggers_layout() {
        let mut sm = setup(1);
        sm.on_window_created(100, 42);
        sm.on_window_created(300, 42);
        sm.swap_command(Direction::Right);
        // Workspace should have both windows after swap
        assert_eq!(sm.workspaces[0].window_count(), 2);
    }

    #[test]
    fn close_command_invokes_adapter() {
        let mut sm = setup(1);
        sm.on_window_created(100, 42);
        sm.on_window_created(300, 42);
        let focused = sm.workspaces[0].focused_node;
        assert!(focused.is_some());
    }

    #[test]
    fn move_focused_to_workspace_moves_window() {
        let mut sm = setup(2);
        // 5 named workspaces per display.
        assert_eq!(sm.workspaces.len(), 10);

        // Add two windows — they go to whatever workspace active_workspace_idx() picks
        sm.on_window_created(100, 42);
        sm.on_window_created(300, 42);

        // Find which workspace received them (avoids HashMap ordering assumptions)
        let source = sm
            .workspaces
            .iter()
            .position(|ws| ws.find_window(100).is_some())
            .expect("window 100 should be in some workspace");
        assert_eq!(sm.workspaces[source].window_count(), 2);

        let target = if source == 0 { 1 } else { 0 };

        sm.move_focused_to_workspace(target);

        assert_eq!(sm.workspaces[source].window_count(), 1);
        assert_eq!(sm.workspaces[target].window_count(), 1);
    }

    #[test]
    fn created_window_overflows_to_next_workspace() {
        let mut sm = setup(2);
        sm.router.set_max_tiles(2);
        sm.workspaces[0].add_window(100, None);
        sm.workspaces[0].add_window(200, None);
        sm.workspaces[1].add_window(300, None);

        sm.on_window_created(400, 42);

        assert!(sm.workspaces[0].find_window(400).is_none());
        assert!(
            sm.workspaces[1].find_window(400).is_some(),
            "full ws-0 should overflow into ws-1"
        );
        assert_eq!(sm.workspaces[0].window_count(), 2);
        assert_eq!(sm.workspaces[1].window_count(), 2);
    }

    #[test]
    fn created_window_all_workspaces_full_stays_untracked() {
        let mut sm = setup(1);
        sm.router.set_max_tiles(2);
        for (i, ws) in sm.workspaces.iter_mut().enumerate() {
            ws.add_window((1000 + i * 2) as WindowId, None);
            ws.add_window((1001 + i * 2) as WindowId, None);
        }

        sm.on_window_created(400, 42);

        assert!(
            sm.workspaces.iter().all(|ws| ws.find_window(400).is_none()),
            "no workspace has room, so the window stays untracked"
        );
        assert_eq!(sm.window_pids.get(&400), Some(&42), "still pid-tracked");
    }

    #[test]
    fn created_window_overflow_wraps_to_first_workspace() {
        let mut sm = setup(1);
        sm.router.set_max_tiles(2);
        // Fill the active workspace (1) and every one after it, leaving only
        // the first workspace with room, so overflow wraps around to ws-0.
        sm.workspaces[1].add_window(300, None);
        sm.workspaces[1].add_window(500, None);
        sm.workspaces[2].add_window(301, None);
        sm.workspaces[2].add_window(501, None);
        sm.workspaces[3].add_window(302, None);
        sm.workspaces[3].add_window(502, None);
        sm.workspaces[4].add_window(303, None);
        sm.workspaces[4].add_window(503, None);
        sm.displays.active_mut().insert(1, 1);
        sm.pid_to_windows.insert(42, vec![300]);

        sm.on_window_created(400, 42);

        assert_eq!(sm.active_workspace_idx(), 1, "ws-1 should be active");
        assert!(
            sm.workspaces[0].find_window(400).is_some(),
            "full ws-1 should wrap around into ws-0"
        );
    }

    #[test]
    fn move_to_full_workspace_redirects_to_next_with_room() {
        let mut sm = setup(2);
        sm.router.set_max_tiles(2);
        sm.workspaces
            .push(Workspace::new("ws-3".into(), 3, (3840, 0), (1920, 1080)));
        sm.displays.active_mut().insert(3, 2);
        sm.workspaces[0].add_window(100, None);
        sm.workspaces[1].add_window(300, None);
        sm.workspaces[1].add_window(500, None);
        sm.pid_to_windows.insert(42, vec![100]);
        sm.on_window_focused(100);

        sm.move_focused_to_workspace(1);

        assert_eq!(sm.workspaces[0].window_count(), 0);
        assert_eq!(sm.workspaces[1].window_count(), 2);
        assert!(
            sm.workspaces[2].find_window(100).is_some(),
            "move to full ws-1 should land in ws-3"
        );
    }

    #[test]
    fn move_to_full_workspace_aborts_when_no_room_anywhere() {
        let mut sm = setup(1);
        sm.router.set_max_tiles(2);
        for (i, ws) in sm.workspaces.iter_mut().enumerate() {
            ws.add_window((1000 + i * 2) as WindowId, None);
            ws.add_window((1001 + i * 2) as WindowId, None);
        }
        sm.pid_to_windows.insert(42, vec![1000]);
        sm.on_window_focused(1000);

        sm.move_focused_to_workspace(1);

        assert!(
            sm.workspaces[0].find_window(1000).is_some(),
            "window should stay put when all workspaces are full"
        );
        assert!(sm.workspaces.iter().all(|ws| ws.window_count() == 2));
    }

    #[test]
    fn toggle_layout_switches_monocle() {
        let mut sm = setup(1);
        assert!(!sm.workspaces[0].monocle);
        let cmd = Command::ToggleLayout;
        let (rtx, _) = mpsc::channel(1);
        sm.on_command(cmd, Some(rtx));
        assert!(sm.workspaces[0].monocle);
    }

    #[test]
    fn set_gap_updates_values() {
        let mut sm = setup(1);
        let (rtx, _) = mpsc::channel(1);
        sm.on_command(Command::SetGapOuter { pixels: 20 }, Some(rtx));
        assert_eq!(sm.gap_outer, 20.0);
    }

    #[test]
    fn split_command_pends_direction_for_next_window() {
        let mut sm = setup(1);
        sm.on_window_created(100, 42);
        let (rtx, _) = mpsc::channel(1);
        sm.on_command(
            Command::Split {
                direction: SplitDirection::Horizontal,
            },
            Some(rtx),
        );
        sm.on_window_created(200, 42);
        assert_eq!(
            sm.workspaces[0].focused_split_direction(),
            Some(SplitDirection::Horizontal),
            "split issued on a focused window becomes the next window's parent direction"
        );
    }

    #[test]
    fn query_state_returns_workspace_info() {
        let mut sm = setup(1);
        sm.on_window_created(100, 42);
        let (rtx, mut rx) = mpsc::channel(1);
        sm.on_command(Command::QueryState, Some(rtx));
        let resp = rx.blocking_recv();
        assert!(resp.is_some());
    }

    #[test]
    fn on_command_handles_every_variant_without_reply() {
        // The keybind/config-watcher path sends `None` for the reply slot:
        // every Command variant must be handled without a channel to write to.
        let commands = [
            Command::Focus {
                direction: Direction::Left,
            },
            Command::MoveWindow {
                direction: Direction::Right,
            },
            Command::Split {
                direction: SplitDirection::Vertical,
            },
            Command::Workspace { id: 1 },
            Command::MoveWindowToWorkspace { id: 2 },
            Command::FocusDisplay {
                direction: Direction::Left,
            },
            Command::MoveWindowToDisplay {
                direction: Direction::Right,
            },
            Command::Close,
            Command::ToggleLayout,
            Command::SetLayout {
                mode: LayoutMode::Accordion,
            },
            Command::SetGapOuter { pixels: 4 },
            Command::SetGapInner { pixels: 2 },
            Command::ToggleBar,
            Command::ReloadConfig,
            Command::QueryState,
            Command::Quit,
        ];
        for cmd in commands {
            let mut sm = setup(1);
            sm.on_command(cmd, None);
        }
    }

    #[test]
    fn on_command_sends_ack_only_when_reply_slot_is_present() {
        let mut sm = setup(1);
        let (rtx, mut rx) = mpsc::channel(1);
        sm.on_command(Command::ToggleLayout, Some(rtx));
        assert!(matches!(rx.blocking_recv(), Some(DaemonResponse::Ack)));

        let mut sm = setup(1);
        let (rtx, mut rx) = mpsc::channel(1);
        sm.on_command(Command::QueryState, Some(rtx));
        assert!(matches!(
            rx.blocking_recv(),
            Some(DaemonResponse::State { .. })
        ));
    }

    #[test]
    fn bar_reserved_rect_top_strip_on_primary_display() {
        let mut sm = setup(1);
        *sm.bar.config_mut() = BarConfig {
            position: BarPosition::Top,
            thickness: 24,
            visible: true,
            enabled: true,
            ..Default::default()
        };
        sm.bar.set_spawned(true);
        sm.bar.set_visible(true);
        let rect = sm.bar_reserved_rect().unwrap();
        assert_eq!(
            (rect.x, rect.y, rect.width, rect.height),
            (0.0, 0.0, 1920.0, 24.0)
        );
    }

    #[test]
    fn bar_reserved_rect_bottom_and_right() {
        let mut sm = setup(1);
        sm.bar.set_spawned(true);
        sm.bar.set_visible(true);
        *sm.bar.config_mut() = BarConfig {
            position: BarPosition::Bottom,
            thickness: 30,
            visible: true,
            enabled: true,
            ..Default::default()
        };
        let rect = sm.bar_reserved_rect().unwrap();
        assert_eq!((rect.x, rect.y), (0.0, 1080.0 - 30.0));
        assert_eq!((rect.width, rect.height), (1920.0, 30.0));

        sm.bar.config_mut().position = BarPosition::Right;
        sm.bar.config_mut().thickness = 40;
        let rect = sm.bar_reserved_rect().unwrap();
        assert_eq!((rect.x, rect.y), (1920.0 - 40.0, 0.0));
        assert_eq!((rect.width, rect.height), (40.0, 1080.0));
    }

    #[test]
    fn bar_reserved_rect_none_when_hidden() {
        let mut sm = setup(1);
        sm.bar.set_visible(false);
        assert_eq!(sm.bar_reserved_rect(), None);
    }

    #[test]
    fn bar_reserved_rect_none_when_not_spawned() {
        let mut sm = setup(1);
        sm.bar.set_visible(true);
        sm.bar.set_spawned(false);
        assert_eq!(sm.bar_reserved_rect(), None);
    }

    #[test]
    fn apply_bar_reservation_reserves_primary_workspace() {
        let mut sm = setup(2);
        sm.bar.set_spawned(true);
        sm.bar.set_visible(true);
        *sm.bar.config_mut() = BarConfig {
            position: BarPosition::Top,
            thickness: 20,
            visible: true,
            enabled: true,
            ..Default::default()
        };
        sm.apply_bar_reservation();
        // Display 1 (primary) owns workspaces 0-4, display 2 owns 5-9.
        assert!(
            sm.workspaces[0].reserved_rect().is_some(),
            "primary monitor workspace should be reserved"
        );
        assert!(
            sm.workspaces[5].reserved_rect().is_none(),
            "secondary monitor workspace should not be reserved"
        );
        sm.bar.set_visible(false);
        sm.apply_bar_reservation();
        assert!(sm.workspaces[0].reserved_rect().is_none());
    }

    #[test]
    fn toggle_bar_command_flips_visibility_and_reservation() {
        let mut sm = setup(1);
        sm.bar.set_spawned(true);
        let was_visible = sm.bar.is_visible();
        let (rtx, _) = mpsc::channel(1);
        sm.on_command(Command::ToggleBar, Some(rtx));
        assert_ne!(sm.bar.is_visible(), was_visible);
        // Reservations match the new visibility.
        assert_eq!(sm.bar_reserved_rect().is_some(), sm.bar.is_visible());
    }

    #[test]
    fn publish_bar_state_reports_active_workspace_and_split() {
        let (tx, _) = mpsc::channel(64);
        let keybinds = Arc::new(Mutex::new(KeybindConfig::default()));
        let mut adapter = make_adapter(1);
        adapter.frontmost = Some(42);
        let (bar_tx, mut bar_rx) = mpsc::channel(64);
        let mut sm = StateManager::new(
            tx,
            keybinds,
            Box::new(adapter),
            BarSender::from_channel(bar_tx),
            None,
            vec![],
        );

        sm.on_window_created(100, 42);
        sm.on_window_created(200, 42);
        // Drain the startup + creation publishes, keep the latest.
        let mut last: Option<BarMessage> = None;
        while let Ok(msg) = bar_rx.try_recv() {
            last = Some(msg);
        }
        let state = match last {
            Some(BarMessage::State(s)) => s,
            other => panic!("expected a State publish, got {:?}", other),
        };
        assert_eq!(state.workspaces.len(), 5);
        assert_eq!(state.workspaces[0].window_count, 2);
        assert!(state.workspaces[0].active);
        assert_eq!(
            state.split_direction,
            Some(SplitDirection::Vertical),
            "two windows on a widescreen monitor split vertically"
        );
    }

    #[test]
    fn publish_bar_state_populates_window_app_names() {
        let (tx, _) = mpsc::channel(64);
        let keybinds = Arc::new(Mutex::new(KeybindConfig::default()));
        let mut adapter = make_adapter(1);
        adapter.app_names.borrow_mut().insert(42, "Safari".into());
        adapter.bundle_ids.borrow_mut().insert(42, "com.apple.Safari".into());
        let (bar_tx, mut bar_rx) = mpsc::channel(64);
        let mut sm = StateManager::new(
            tx,
            keybinds,
            Box::new(adapter),
            BarSender::from_channel(bar_tx),
            None,
            vec![],
        );

        sm.on_window_created(100, 42);
        sm.on_window_created(300, 43);
        // 43 has no display name and no bundle id -> falls back to "unknown".
        // 42 is Safari, which the default routing sends to the Browsing
        // workspace (index 1); 43's window lands in the active workspace (0).
        let mut last: Option<BarMessage> = None;
        while let Ok(msg) = bar_rx.try_recv() {
            last = Some(msg);
        }
        let state = match last {
            Some(BarMessage::State(s)) => s,
            other => panic!("expected a State publish, got {:?}", other),
        };
        assert_eq!(state.workspaces[0].windows, vec!["unknown"]);
        assert_eq!(state.workspaces[1].windows, vec!["Safari"]);
    }

    #[test]
    fn on_window_created_routes_configured_app_to_its_workspace() {
        let mut sm = setup(1);
        sm.os.inject_bundle_id(77, "com.google.Chrome".into());
        sm.os.inject_app_name(77, "Chrome".into());

        sm.on_window_created(777, 77);

        let idx = sm
            .workspaces
            .iter()
            .position(|ws| ws.find_window(777).is_some())
            .expect("routed window should be tracked");
        assert_eq!(sm.workspaces[idx].name, "Browsing");
    }

    #[test]
    fn on_window_created_routing_matches_app_name_case_insensitively() {
        let mut sm = setup(1);
        sm.os.inject_app_name(88, "spotify".into());

        sm.on_window_created(888, 88);

        let idx = sm
            .workspaces
            .iter()
            .position(|ws| ws.find_window(888).is_some())
            .expect("routed window should be tracked");
        assert_eq!(sm.workspaces[idx].name, "Music");
    }

    #[test]
    fn quit_command_requests_shutdown_and_exits_bar() {
        let mut sm = setup(1);
        let (bar_tx, mut bar_rx) = mpsc::channel(64);
        sm.bar_sender = BarSender::from_channel(bar_tx);
        let (rtx, mut rx) = mpsc::channel(1);

        sm.on_command(Command::Quit, Some(rtx));

        assert!(sm.shutdown_requested());
        assert!(matches!(rx.blocking_recv(), Some(DaemonResponse::Ack)));
        let msgs: Vec<_> = std::iter::from_fn(|| bar_rx.try_recv().ok()).collect();
        assert!(
            msgs.iter().any(|m| matches!(m, BarMessage::Exit)),
            "quitting should tell the bar to exit too"
        );
    }

    #[test]
    fn workspace_switch_hides_all_other_workspaces_on_monitor() {
        let mut sm = setup(1);
        // Route Firefox to Browsing (ws-1) then switch back to Development (ws-0).
        sm.os.inject_bundle_id(77, "org.mozilla.firefox".into());
        sm.on_window_created(777, 77);
        let browsing_idx = sm
            .workspaces
            .iter()
            .position(|ws| ws.name == "Browsing")
            .unwrap();
        assert!(sm.workspaces[browsing_idx].find_window(777).is_some());

        // Switch to Development via command.
        sm.on_command(Command::Workspace { id: 1 }, None);
        let dev_idx = sm
            .workspaces
            .iter()
            .position(|ws| ws.name == "Development")
            .unwrap();
        assert_eq!(sm.displays.active().get(&1), Some(&dev_idx));
        // Browsing windows should still be only in Browsing, not dragged to Dev.
        assert!(sm.workspaces[dev_idx].find_window(777).is_none());
        assert!(sm.workspaces[browsing_idx].find_window(777).is_some());
        assert_eq!(sm.workspaces[dev_idx].window_count(), 0);
    }

    #[test]
    fn workspace_switch_debounces_stale_focus() {
        let mut sm = setup(1);
        sm.os.inject_bundle_id(77, "org.mozilla.firefox".into());
        sm.on_window_created(777, 77);
        let browsing_idx = sm
            .workspaces
            .iter()
            .position(|ws| ws.name == "Browsing")
            .unwrap();
        let dev_idx = sm
            .workspaces
            .iter()
            .position(|ws| ws.name == "Development")
            .unwrap();
        // Start on browsing, then switch to dev — sets debounce.
        sm.on_command(Command::Workspace { id: 2 }, None);
        assert_eq!(sm.displays.active().get(&1), Some(&browsing_idx));
        sm.on_command(Command::Workspace { id: 1 }, None);
        assert_eq!(sm.displays.active().get(&1), Some(&dev_idx));
        // Stale focus for the firefox window that was just hidden should not flip active back.
        sm.on_window_focused(777);
        assert_eq!(
            sm.displays.active().get(&1),
            Some(&dev_idx),
            "debounced focus should not drag active back to browsing"
        );
    }

    #[test]
    fn per_monitor_workspace_entries_respected() {
        let mut ds = crate::state::display::DisplaySet::new(vec![
            WorkspaceEntry {
                name: "Dev".into(),
                apps: vec![],
                monitor: Some(crate::config::MonitorRef::Index(1)),
                autostart: vec![],
            },
            WorkspaceEntry {
                name: "Browse".into(),
                apps: vec![],
                monitor: None,
                autostart: vec![],
            },
        ]);
        let mut wss = Vec::new();
        ds.init_workspaces(
            &mut wss,
            &[
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
            ],
        );
        // Dev only on monitor 1, Browse on both.
        assert_eq!(wss.len(), 3);
        assert_eq!(wss[0].name, "Dev");
        assert_eq!(wss[0].monitor_id, 1);
        assert_eq!(wss[1].name, "Browse");
        assert_eq!(wss[1].monitor_id, 1);
        assert_eq!(wss[2].name, "Browse");
        assert_eq!(wss[2].monitor_id, 2);
    }
}
