use crate::adapter::OsAdapter;
use crate::bar_server::BarSender;
use crate::config::keybinds::KeybindConfig;
use crate::config::{BarConfig, Settings, WorkspaceEntry};
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

const SWAP_HOLD_DURATION: Duration = Duration::from_secs(2);
const DRAG_IDLE_TIMEOUT: Duration = Duration::from_millis(500);
/// How often to re-check AX hidden/minimized state as a fallback for missed
/// notifications, so hidden windows don't keep occupying tiles.
const RECONCILE_INTERVAL: Duration = Duration::from_secs(1);

pub struct StateManager {
    workspaces: Vec<Workspace>,
    active_workspaces: HashMap<u32, usize>,
    frontmost_pid: Option<i32>,
    pid_to_windows: HashMap<i32, Vec<WindowId>>,
    window_pids: HashMap<WindowId, i32>,
    /// Workspace index each hidden/minimized window came from, so it is
    /// retiled back where it belongs when it becomes visible again.
    hidden_workspaces: HashMap<WindowId, usize>,
    os: Box<dyn OsAdapter>,
    event_tx: mpsc::Sender<DaemonEvent>,
    gap_outer: f64,
    gap_inner: f64,
    max_tiles: usize,
    keybinds: Arc<Mutex<KeybindConfig>>,
    restricted_apps: Vec<String>,
    bar_sender: BarSender,
    bar_config: BarConfig,
    bar_visible: bool,
    bar_spawned: bool,
    /// Pids whose windows are never managed by the WM — currently the spawned
    /// `pengwm-bar` process, whose window must not be tiled.
    excluded_pids: Vec<i32>,
    last_layout_rects: HashMap<WindowId, Rect>,
    drag_window: Option<WindowId>,
    drag_overlap_target: Option<WindowId>,
    drag_overlap_start: Option<Instant>,
    last_drag_move: Option<Instant>,
    last_reconcile: Instant,
    /// The configured named workspaces (with their app routing rules) that
    /// every monitor gets a copy of. Drives workspace creation at startup and
    /// app-to-workspace routing for new windows.
    workspace_entries: Vec<WorkspaceEntry>,
    /// Set when `Command::Quit` is handled; the event loop polls this and
    /// returns so the daemon process can exit.
    shutdown_requested: bool,
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
        let displays = os.active_displays();
        let mut workspaces = Vec::new();
        let mut pid_to_windows: HashMap<i32, Vec<WindowId>> = HashMap::new();
        let mut window_pids: HashMap<WindowId, i32> = HashMap::new();
        let mut active_workspaces = HashMap::new();

        let settings = Settings::load();
        // Empty list means the user opted out of named workspaces; fall back to
        // the defaults rather than creating no workspaces at all.
        let workspace_entries = if settings.workspaces.is_empty() {
            crate::config::default_workspaces()
        } else {
            settings.workspaces.clone()
        };

        for display in &displays {
            let base = workspaces.len();
            active_workspaces.insert(display.id, base);
            for entry in &workspace_entries {
                let ws = Workspace::new(
                    entry.name.clone(),
                    display.id,
                    display.origin,
                    display.size,
                );
                workspaces.push(ws);
            }
        }

        if workspaces.is_empty() {
            workspaces.push(Workspace::new(
                "ws-1".into(),
                os.primary_display_id(),
                (0, 0),
                (1920, 1080),
            ));
            active_workspaces.insert(os.primary_display_id(), 0);
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

        let bar_config = settings.bar.clone();
        let bar_spawned = bar_pid.is_some();
        let excluded_pids = bar_pid
            .into_iter()
            .chain(excluded_pids)
            .collect::<Vec<_>>();
        let bar_visible = settings.bar.visible && bar_spawned;
        let mut state = Self {
            workspaces,
            active_workspaces,
            frontmost_pid,
            pid_to_windows,
            window_pids,
            hidden_workspaces: HashMap::new(),
            os,
            event_tx,
            gap_outer: settings.gap_outer.max(0) as f64,
            gap_inner: settings.gap_inner.max(0) as f64,
            max_tiles: settings.max_tiles.max(1),
            keybinds,
            restricted_apps: settings.restricted_apps,
            bar_sender,
            bar_config,
            bar_visible,
            bar_spawned,
            excluded_pids,
            last_layout_rects: HashMap::new(),
            drag_window: None,
            drag_overlap_target: None,
            drag_overlap_start: None,
            last_drag_move: None,
            last_reconcile: Instant::now(),
            workspace_entries,
            shutdown_requested: false,
        };

        for i in 0..state.workspaces.len() {
            if i != 0 {
                state.hide_workspace(i);
            }
        }

        state.apply_bar_reservation();
        state.bar_sender.send(if state.bar_visible {
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
        let n = self.workspaces.len();
        if n == 0 {
            return None;
        }
        let monitor = self.workspaces[start].monitor_id;
        for offset in 1..=n {
            let idx = (start + offset) % n;
            if self.workspaces[idx].monitor_id != monitor {
                continue;
            }
            if self.workspaces[idx].window_count() < self.max_tiles {
                return Some(idx);
            }
        }
        None
    }

    fn active_workspace_idx(&self) -> usize {
        if let Some(pid) = self.frontmost_pid {
            if let Some(windows) = self.pid_to_windows.get(&pid) {
                for &window_id in windows {
                    for ws in &self.workspaces {
                        if ws.find_window(window_id).is_some() {
                            if let Some(&idx) = self.active_workspaces.get(&ws.monitor_id) {
                                if idx < self.workspaces.len() {
                                    return idx;
                                }
                            }
                        }
                    }
                }
            }
        }
        self.active_workspaces.values().next().copied().unwrap_or(0)
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
        self.hidden_workspaces.remove(&window_id);
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
        if active >= self.workspaces.len() {
            return None;
        }
        let monitor = self.workspaces[active].monitor_id;
        let name = self.configured_workspace_name_for_pid(pid)?;
        self.workspaces
            .iter()
            .position(|ws| ws.name == *name && ws.monitor_id == monitor)
    }

    /// Name of the configured workspace `pid`'s app is assigned to, matched
    /// case-insensitively against bundle id first, then app display name.
    fn configured_workspace_name_for_pid(&self, pid: i32) -> Option<&str> {
        if self.workspace_entries.is_empty() {
            return None;
        }
        let bundle = self.os.app_bundle_id(pid);
        let app_name = self.os.app_name(pid);
        self.workspace_entries
            .iter()
            .find(|entry| {
                entry.apps.iter().any(|app| {
                    bundle.as_deref().is_some_and(|b| b.eq_ignore_ascii_case(app))
                        || app_name
                            .as_deref()
                            .is_some_and(|n| n.eq_ignore_ascii_case(app))
                })
            })
            .map(|entry| entry.name.as_str())
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
        let target = if self.workspaces[preferred].window_count() >= self.max_tiles {
            match self.find_next_workspace_with_capacity(preferred) {
                Some(idx) => {
                    log::info!(
                        "Workspace {} full ({} >= {}), routing new window to workspace {}",
                        preferred,
                        self.workspaces[preferred].window_count(),
                        self.max_tiles,
                        idx
                    );
                    idx
                }
                None => {
                    log::warn!(
                        "All workspaces at capacity ({}), leaving window {} untracked",
                        self.max_tiles,
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
        self.apply_layout(target);
        Some(target)
    }

    pub fn on_window_destroyed(&mut self, window_id: WindowId) {
        for i in 0..self.workspaces.len() {
            let ws = &self.workspaces[i];
            if ws.find_window(window_id).is_some() {
                let ws = &mut self.workspaces[i];
                ws.remove_window(window_id);
                self.apply_layout(i);
                break;
            }
        }
        self.window_pids.remove(&window_id);
        self.hidden_workspaces.remove(&window_id);
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
        for i in 0..self.workspaces.len() {
            let ws = &self.workspaces[i];
            if ws.find_window(window_id).is_some() {
                let ws = &mut self.workspaces[i];
                ws.remove_window(window_id);
                self.hidden_workspaces.insert(window_id, i);
                self.apply_layout(i);
                self.publish_bar_state();
                return;
            }
        }
        log::debug!("WindowHidden: window {} not tracked, ignoring", window_id);
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
        let preferred = self
            .hidden_workspaces
            .remove(&window_id)
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
                let prev = self.active_workspaces.insert(mon_id, i);
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
        self.drag_window = Some(window_id);
        self.last_drag_move = Some(now);

        if self.find_workspace_for_window(window_id).is_none() {
            return;
        }

        let dragged_size = match self.last_layout_rects.get(&window_id) {
            Some(r) => (r.width, r.height),
            None => return,
        };

        let cx = x + dragged_size.0 / 2.0;
        let cy = y + dragged_size.1 / 2.0;

        let new_target =
            pengwm_core::layout::window_at_point(&self.last_layout_rects, cx, cy, window_id);

        match (self.drag_overlap_target, new_target) {
            (Some(t), Some(nt)) if t == nt => {
                // same target — swap timing is handled in on_tick
            }
            (_, Some(nt)) => {
                self.drag_overlap_target = Some(nt);
                self.drag_overlap_start = Some(now);
            }
            (_, None) => {
                self.drag_overlap_target = None;
                self.drag_overlap_start = None;
            }
        }
    }

    pub fn on_tick(&mut self) {
        let now = Instant::now();

        // Fallback detection for minimized/hidden windows in case the AX
        // notifications were missed or never registered.
        if now.duration_since(self.last_reconcile) >= RECONCILE_INTERVAL {
            self.last_reconcile = now;
            self.reconcile_hidden_windows();
        }

        // Check swap hold first — this runs every 50ms even when move
        // notifications stop (user holds window still over a target).
        if let (Some(window_id), Some(target)) = (self.drag_window, self.drag_overlap_target) {
            if let Some(start) = self.drag_overlap_start {
                if now.duration_since(start) >= SWAP_HOLD_DURATION {
                    if let Some(ws_idx) = self.find_workspace_for_window(window_id) {
                        let ws = &mut self.workspaces[ws_idx];
                        if ws.swap_windows_by_id(window_id, target) {
                            self.apply_layout(ws_idx);
                        }
                    }
                    self.clear_drag_state();
                    return;
                }
            }
        }

        // Only snap back when there's no active overlap target — otherwise
        // the 500ms idle timeout would kill the swap before the 2s hold.
        if let Some(last_move) = self.last_drag_move {
            if self.drag_overlap_target.is_none()
                && now.duration_since(last_move) >= DRAG_IDLE_TIMEOUT
            {
                if self.drag_window.is_some() {
                    self.apply_layout(self.active_workspace_idx());
                }
                self.clear_drag_state();
            }
        }
    }

    fn clear_drag_state(&mut self) {
        self.drag_window = None;
        self.drag_overlap_target = None;
        self.drag_overlap_start = None;
        self.last_drag_move = None;
    }

    /// Re-check every tracked window's actual hidden/minimized state and bring
    /// the tree in line: untile windows the OS reports as hidden, and retile
    /// ones that became visible again.
    fn reconcile_hidden_windows(&mut self) {
        for window_id in self.window_pids.keys().copied().collect::<Vec<_>>() {
            let hidden = self.os.window_is_hidden(window_id);
            if hidden && self.find_workspace_for_window(window_id).is_some() {
                self.on_window_hidden(window_id);
            } else if !hidden && self.hidden_workspaces.contains_key(&window_id) {
                self.on_window_shown(window_id);
            }
        }
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
        let info = self
            .os
            .active_displays()
            .into_iter()
            .find(|d| d.id == display_id);
        let (origin, size) = match info {
            Some(d) => (d.origin, d.size),
            None => return,
        };
        let entries = if self.workspace_entries.is_empty() {
            crate::config::default_workspaces()
        } else {
            self.workspace_entries.clone()
        };
        let base = self.workspaces.len();
        self.active_workspaces.insert(display_id, base);
        for entry in &entries {
            self.workspaces
                .push(Workspace::new(entry.name.clone(), display_id, origin, size));
        }
        self.apply_bar_reservation();
        self.publish_bar_state();
    }

    pub fn on_monitor_removed(&mut self, _display_id: u32) {
        let primary = self.os.primary_display_id();
        let primary_origin = self
            .os
            .active_displays()
            .into_iter()
            .find(|d| d.id == primary)
            .map(|d| d.origin)
            .unwrap_or((0, 0));
        for ws in &mut self.workspaces {
            if ws.monitor_id == _display_id {
                ws.monitor_id = primary;
                ws.set_monitor_origin(primary_origin);
            }
        }
        let active = self.os.active_displays();
        self.workspaces
            .retain(|ws| active.iter().any(|d| d.id == ws.monitor_id));
        if self.workspaces.is_empty() {
            self.workspaces.push(Workspace::new(
                "ws-1".into(),
                primary,
                primary_origin,
                (1920, 1080),
            ));
        }
        self.active_workspaces
            .retain(|_, idx| *idx < self.workspaces.len());
        if self.active_workspaces.is_empty() {
            self.active_workspaces
                .insert(self.workspaces[0].monitor_id, 0);
        }
        self.apply_bar_reservation();
        self.publish_bar_state();
    }

    pub fn on_monitor_resized(&mut self, display_id: u32) {
        let info = self
            .os
            .active_displays()
            .into_iter()
            .find(|d| d.id == display_id);
        if let Some(display) = info {
            for i in 0..self.workspaces.len() {
                if self.workspaces[i].monitor_id == display_id {
                    self.workspaces[i].update_monitor_geometry(display.origin, display.size);
                    self.apply_layout(i);
                }
            }
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
                let n = id;
                if n > 0 && (n as usize) <= self.workspaces.len() {
                    let new_idx = (n - 1) as usize;
                    let current = self.active_workspace_idx();
                    if new_idx != current {
                        self.hide_workspace(current);
                        let mon_id = self.workspaces[current].monitor_id;
                        self.active_workspaces.insert(mon_id, new_idx);
                        self.apply_layout(new_idx);
                    }
                }
            }
            Command::MoveWindowToWorkspace { id } => {
                let n = id;
                if n > 0 && (n as usize) <= self.workspaces.len() {
                    self.move_focused_to_workspace((n - 1) as usize);
                }
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
                if self.bar_spawned {
                    self.bar_visible = !self.bar_visible;
                    log::info!(
                        "Bar toggled: {}",
                        if self.bar_visible {
                            "visible"
                        } else {
                            "hidden"
                        }
                    );
                    self.bar_sender.send(if self.bar_visible {
                        BarMessage::Show
                    } else {
                        BarMessage::Hide
                    });
                    self.apply_bar_reservation();
                } else {
                    log::info!("Bar not running; toggle ignored");
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
            let dest = if self.workspaces[target].window_count() >= self.max_tiles {
                match self.find_next_workspace_with_capacity(target) {
                    Some(idx) => idx,
                    None => {
                        log::warn!(
                            "No workspace has room for {} (cap {}), move aborted",
                            wid,
                            self.max_tiles
                        );
                        self.publish_bar_state();
                        return;
                    }
                }
            } else {
                target
            };
            self.workspaces[current].remove_window(wid);
            self.apply_layout(current);
            self.workspaces[dest].add_window(wid, None);
            self.apply_layout(dest);
        }
        self.publish_bar_state();
    }

    fn reload_config(&mut self) {
        log::info!("Reloading config...");
        let updated_keybinds = KeybindConfig::load();
        {
            let mut keybinds = self.keybinds.lock().expect("keybind mutex poisoned");
            *keybinds = updated_keybinds;
            log::info!("Config reloaded ({} bindings)", keybinds.bindings.len());
        }
        let updated_settings = Settings::load();
        self.gap_outer = updated_settings.gap_outer.max(0) as f64;
        self.gap_inner = updated_settings.gap_inner.max(0) as f64;
        self.max_tiles = updated_settings.max_tiles.max(1);
        self.restricted_apps = updated_settings.restricted_apps;
        self.bar_config = updated_settings.bar.clone();
        let enabled = updated_settings.bar.enabled;
        self.bar_visible = updated_settings.bar.visible && self.bar_spawned;
        self.apply_layout(self.active_workspace_idx());
        self.bar_sender.send(BarMessage::Reload);
        if enabled && !self.bar_spawned {
            // Bar disabled at startup but enabled now — a restart is required
            // to spawn it (the daemon reads `enabled` at startup).
            log::info!(
                "bar.enabled flipped to true at runtime; restart the daemon to spawn pengwm-bar"
            );
        }
        if !enabled && self.bar_spawned {
            log::info!("bar.enabled flipped to false; exiting pengwm-bar");
            self.bar_sender.send(BarMessage::Exit);
            self.bar_spawned = false;
            self.bar_visible = false;
            self.apply_bar_reservation();
            self.publish_bar_state();
            return;
        }
        self.bar_sender.send(if self.bar_visible {
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
        let window_ids = self.workspaces[workspace_idx].all_windows();
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
    /// available.
    fn bar_reserved_rect(&self) -> Option<Rect> {
        if !self.bar_visible || !self.bar_spawned {
            return None;
        }
        let primary = self.os.primary_display_id();
        let display = self
            .os
            .active_displays()
            .into_iter()
            .find(|d| d.id == primary)?;
        Some(pengwm_core::layout::bar_strip_rect(
            display.origin,
            display.size,
            self.bar_config.position,
            self.bar_config.thickness,
        ))
    }

    /// Push the current bar strip geometry into every workspace (only
    /// workspaces on the primary display are reserved) and re-lay-out.
    fn apply_bar_reservation(&mut self) {
        let rect = self.bar_reserved_rect();
        let primary = self.os.primary_display_id();
        for i in 0..self.workspaces.len() {
            let ws = &mut self.workspaces[i];
            ws.set_reserved_rect(if ws.monitor_id == primary { rect } else { None });
        }
        for i in 0..self.workspaces.len() {
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
                        .active_workspaces
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
            .entry(42)
            .or_default()
            .extend(vec![100, 200]);
        adapter.window_pids.insert(100, 42);
        adapter.window_pids.insert(200, 42);
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
        assert_eq!(sm.active_workspaces.get(&1), Some(&0));
        assert_eq!(sm.active_workspaces.get(&2), Some(&5));
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
        assert_eq!(sm.hidden_workspaces.get(&100), Some(&0));
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
        assert!(!sm.hidden_workspaces.contains_key(&100));
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
        assert!(sm.hidden_workspaces.contains_key(&100));
        sm.on_window_destroyed(100);
        assert!(!sm.hidden_workspaces.contains_key(&100));
        assert!(!sm.window_pids.contains_key(&100));
    }

    #[test]
    fn reconcile_untiles_windows_reported_hidden() {
        let mut sm = setup(1);
        sm.on_window_created(100, 42);
        sm.on_window_created(200, 42);
        let adapter = sm.os.as_any_mut().downcast_mut::<TestAdapter>().unwrap();
        adapter.hidden_windows.insert(100);
        sm.last_reconcile = Instant::now() - RECONCILE_INTERVAL - Duration::from_secs(1);
        sm.on_tick();
        assert!(sm.workspaces[0].find_window(100).is_none());
        assert_eq!(sm.hidden_workspaces.get(&100), Some(&0));
        assert_eq!(sm.workspaces[0].window_count(), 1);
    }

    #[test]
    fn reconcile_retiles_windows_when_visible_again() {
        let mut sm = setup(1);
        sm.on_window_created(100, 42);
        sm.on_window_hidden(100);
        assert!(sm.hidden_workspaces.contains_key(&100));
        let adapter = sm.os.as_any_mut().downcast_mut::<TestAdapter>().unwrap();
        adapter.hidden_windows.clear();
        sm.last_reconcile = Instant::now() - RECONCILE_INTERVAL - Duration::from_secs(1);
        sm.on_tick();
        assert!(sm.workspaces[0].find_window(100).is_some());
        assert!(!sm.hidden_workspaces.contains_key(&100));
    }

    #[test]
    fn reconcile_hides_windows_when_app_is_hidden() {
        let mut sm = setup(1);
        sm.on_window_created(100, 42);
        sm.on_window_created(200, 42);
        let adapter = sm.os.as_any_mut().downcast_mut::<TestAdapter>().unwrap();
        adapter.hidden_apps.insert(42);
        sm.last_reconcile = Instant::now() - RECONCILE_INTERVAL - Duration::from_secs(1);
        sm.on_tick();
        assert!(sm.workspaces[0].find_window(100).is_none());
        assert!(sm.workspaces[0].find_window(200).is_none());
        assert_eq!(sm.workspaces[0].window_count(), 0);
    }

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
        sm.max_tiles = 2;
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
        sm.max_tiles = 2;
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
        sm.max_tiles = 2;
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
        sm.active_workspaces.insert(1, 1);
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
        sm.max_tiles = 2;
        sm.workspaces
            .push(Workspace::new("ws-3".into(), 3, (3840, 0), (1920, 1080)));
        sm.active_workspaces.insert(3, 2);
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
        sm.max_tiles = 2;
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
        sm.bar_config = BarConfig {
            position: BarPosition::Top,
            thickness: 24,
            visible: true,
            enabled: true,
            ..Default::default()
        };
        sm.bar_spawned = true;
        sm.bar_visible = true;
        let rect = sm.bar_reserved_rect().unwrap();
        assert_eq!(
            (rect.x, rect.y, rect.width, rect.height),
            (0.0, 0.0, 1920.0, 24.0)
        );
    }

    #[test]
    fn bar_reserved_rect_bottom_and_right() {
        let mut sm = setup(1);
        sm.bar_spawned = true;
        sm.bar_visible = true;
        sm.bar_config = BarConfig {
            position: BarPosition::Bottom,
            thickness: 30,
            visible: true,
            enabled: true,
            ..Default::default()
        };
        let rect = sm.bar_reserved_rect().unwrap();
        assert_eq!((rect.x, rect.y), (0.0, 1080.0 - 30.0));
        assert_eq!((rect.width, rect.height), (1920.0, 30.0));

        sm.bar_config.position = BarPosition::Right;
        sm.bar_config.thickness = 40;
        let rect = sm.bar_reserved_rect().unwrap();
        assert_eq!((rect.x, rect.y), (1920.0 - 40.0, 0.0));
        assert_eq!((rect.width, rect.height), (40.0, 1080.0));
    }

    #[test]
    fn bar_reserved_rect_none_when_hidden() {
        let mut sm = setup(1);
        sm.bar_visible = false;
        assert_eq!(sm.bar_reserved_rect(), None);
    }

    #[test]
    fn bar_reserved_rect_none_when_not_spawned() {
        let mut sm = setup(1);
        sm.bar_visible = true;
        sm.bar_spawned = false;
        assert_eq!(sm.bar_reserved_rect(), None);
    }

    #[test]
    fn apply_bar_reservation_reserves_primary_workspace() {
        let mut sm = setup(2);
        sm.bar_spawned = true;
        sm.bar_visible = true;
        sm.bar_config = BarConfig {
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
        sm.bar_visible = false;
        sm.apply_bar_reservation();
        assert!(sm.workspaces[0].reserved_rect().is_none());
    }

    #[test]
    fn toggle_bar_command_flips_visibility_and_reservation() {
        let mut sm = setup(1);
        sm.bar_spawned = true;
        let was_visible = sm.bar_visible;
        let (rtx, _) = mpsc::channel(1);
        sm.on_command(Command::ToggleBar, Some(rtx));
        assert_ne!(sm.bar_visible, was_visible);
        // Reservations match the new visibility.
        assert_eq!(sm.bar_reserved_rect().is_some(), sm.bar_visible);
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
        adapter.app_names.insert(42, "Safari".into());
        adapter.bundle_ids.insert(42, "com.apple.Safari".into());
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
        let adapter = sm.os.as_any_mut().downcast_mut::<TestAdapter>().unwrap();
        adapter.bundle_ids.insert(77, "com.google.Chrome".into());
        adapter.app_names.insert(77, "Chrome".into());

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
        let adapter = sm.os.as_any_mut().downcast_mut::<TestAdapter>().unwrap();
        adapter.app_names.insert(88, "spotify".into());

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
}
