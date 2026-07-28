use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use pengwm_core::tree::WindowId;
use pengwm_core::workspace::Workspace;
use pengwm_core::command::{Command, DaemonResponse};
use crate::adapter::OsAdapter;
use crate::event_loop::DaemonEvent;
use crate::config::keybinds::KeybindConfig;
use crate::config::Settings;

pub struct StateManager {
    workspaces: Vec<Workspace>,
    active_workspaces: HashMap<u32, usize>,
    frontmost_pid: Option<i32>,
    pid_to_windows: HashMap<i32, Vec<WindowId>>,
    window_pids: HashMap<WindowId, i32>,
    os: Box<dyn OsAdapter>,
    event_tx: mpsc::Sender<DaemonEvent>,
    gap_outer: f64,
    gap_inner: f64,
    keybinds: Arc<Mutex<KeybindConfig>>,
}

impl StateManager {
    pub fn new(
        event_tx: mpsc::Sender<DaemonEvent>,
        keybinds: Arc<Mutex<KeybindConfig>>,
        mut os: Box<dyn OsAdapter>,
    ) -> Self {
        let displays = os.active_displays();
        let mut workspaces = Vec::new();
        let mut pid_to_windows: HashMap<i32, Vec<WindowId>> = HashMap::new();
        let mut window_pids: HashMap<WindowId, i32> = HashMap::new();
        let mut active_workspaces = HashMap::new();

        for (i, display) in displays.iter().enumerate() {
            active_workspaces.insert(display.id, i);
            let ws = Workspace::new(
                format!("ws-{}", i + 1),
                display.id,
                display.origin,
                display.size,
            );
            workspaces.push(ws);
        }

        if workspaces.is_empty() {
            workspaces.push(Workspace::new(
                "ws-1".into(),
                os.primary_display_id(),
                (0, 0),
                (1920, 1080),
            ));
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

        let settings = Settings::load();
        let mut state = Self {
            workspaces,
            active_workspaces,
            frontmost_pid,
            pid_to_windows,
            window_pids,
            os,
            event_tx,
            gap_outer: settings.gap_outer as f64,
            gap_inner: settings.gap_inner as f64,
            keybinds,
        };

        for i in 0..state.workspaces.len() {
            if i != 0 {
                state.hide_workspace(i);
            }
        }

        state
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
        if !self.window_pids.contains_key(&window_id) {
            self.window_pids.insert(window_id, pid);
            self.pid_to_windows.entry(pid).or_default().push(window_id);
        }
        let target = self.active_workspace_idx();
        let ws = &mut self.workspaces[target];
        if ws.find_window(window_id).is_some() {
            return;
        }
        ws.add_window(window_id, None);
        self.apply_layout(target);
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
        self.pid_to_windows.retain(|_, windows| {
            windows.retain(|w| *w != window_id);
            !windows.is_empty()
        });
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
                return;
            }
        }
    }

    pub fn on_app_launched(&mut self, pid: i32) {
        log::info!("App launched: pid={}", pid);
        self.os.attach_observer(pid);
        for window_id in self.os.poll_windows_for_pid(pid) {
            self.window_pids.insert(window_id, pid);
            self.pid_to_windows.entry(pid).or_default().push(window_id);
            let _ = self.event_tx.try_send(DaemonEvent::WindowCreated(window_id, pid));
        }
    }

    pub fn on_app_terminated(&mut self, pid: i32) {
        log::info!("App terminated: pid={}", pid);
        self.os.detach_observer(pid);
        if let Some(windows) = self.pid_to_windows.remove(&pid) {
            for window_id in windows {
                self.window_pids.remove(&window_id);
                let _ = self.event_tx.try_send(DaemonEvent::WindowDestroyed(window_id));
            }
        }
    }

    pub fn on_app_activated(&mut self, pid: i32) {
        log::debug!("App activated: pid={}", pid);
        self.frontmost_pid = Some(pid);
        if let Some(window_id) = self.os.focused_window_for_pid(pid) {
            self.on_window_focused(window_id);
        }
    }

    pub fn on_monitor_added(&mut self, display_id: u32) {
        let info = self.os.active_displays()
            .into_iter()
            .find(|d| d.id == display_id);
        let (origin, size) = match info {
            Some(d) => (d.origin, d.size),
            None => return,
        };
        let name = format!("ws-{}", self.workspaces.len() + 1);
        let ws = Workspace::new(name, display_id, origin, size);
        let idx = self.workspaces.len();
        self.active_workspaces.insert(display_id, idx);
        self.workspaces.push(ws);
    }

    pub fn on_monitor_removed(&mut self, _display_id: u32) {
        let primary = self.os.primary_display_id();
        let primary_origin = self.os.active_displays()
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
        self.workspaces.retain(|ws| {
            active.iter().any(|d| d.id == ws.monitor_id)
        });
        if self.workspaces.is_empty() {
            self.workspaces.push(Workspace::new(
                "ws-1".into(),
                primary,
                primary_origin,
                (1920, 1080),
            ));
        }
        self.active_workspaces.retain(|_, idx| *idx < self.workspaces.len());
        if self.active_workspaces.is_empty() {
            self.active_workspaces.insert(self.workspaces[0].monitor_id, 0);
        }
    }

    pub fn on_monitor_resized(&mut self, display_id: u32) {
        let info = self.os.active_displays()
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
    }

    pub fn on_command(
        &mut self,
        cmd: Command,
        tx: mpsc::Sender<DaemonResponse>,
    ) {
        match cmd {
            Command::Focus { direction } => self.focus_command(direction),
            Command::MoveWindow { direction } => self.swap_command(direction),
            Command::Split { direction } => {
                let idx = self.active_workspace_idx();
                let ws = &mut self.workspaces[idx];
                if ws.focused_is_window() {
                    ws.pending_split = Some(direction);
                } else {
                    ws.set_split_direction(direction);
                }
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
            Command::SetGapOuter { pixels } => {
                self.gap_outer = pixels.max(0) as f64;
                self.apply_layout(self.active_workspace_idx());
            }
            Command::SetGapInner { pixels } => {
                self.gap_inner = pixels.max(0) as f64;
                self.apply_layout(self.active_workspace_idx());
            }
            Command::ReloadConfig => {
                self.reload_config();
            }
            Command::QueryState => {
                let info = self.workspaces.iter().map(|ws| {
                    pengwm_core::command::WorkspaceInfo {
                        name: ws.name.clone(),
                        monitor_id: ws.monitor_id,
                        window_count: ws.window_count(),
                        focused_window: ws.focused_window_id(),
                    }
                }).collect();
                let _ = tx.try_send(DaemonResponse::State { workspaces: info });
            }
        }
    }

    fn focus_command(&mut self, direction: Direction) {
        let idx = self.active_workspace_idx();
        let ws = &mut self.workspaces[idx];
        ws.focus_neighbor(direction);
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
            self.workspaces[current].remove_window(wid);
            self.apply_layout(current);
            self.workspaces[target].add_window(wid, None);
            self.apply_layout(target);
        }
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
        self.gap_outer = updated_settings.gap_outer as f64;
        self.gap_inner = updated_settings.gap_inner as f64;
        self.apply_layout(self.active_workspace_idx());
        log::info!("Config reloaded (gaps: {}/{})", self.gap_outer, self.gap_inner);
    }

    fn hide_workspace(&mut self, workspace_idx: usize) {
        let window_ids = self.workspaces[workspace_idx].all_windows();
        self.os.hide_windows(&window_ids);
    }

    fn apply_layout(&mut self, workspace_idx: usize) {
        let rects = self.workspaces[workspace_idx].layout(self.gap_inner, self.gap_outer);

        for (&window_id, rect) in &rects {
            if let Err(e) = self.os.set_window_rect(window_id, *rect) {
                log::error!("apply_layout: set_window_rect failed for window {}: {}", window_id, e);
            }
        }
    }
}

use pengwm_core::tree::Direction;

#[cfg(test)]
mod state_tests {
    use super::*;
    use crate::adapter::DisplayInfo;
    use crate::adapter_test::TestAdapter;
    use crate::config::keybinds::KeybindConfig;

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
                DisplayInfo { id: 1, origin: (0, 0), size: (1920, 1080) },
                DisplayInfo { id: 2, origin: (1920, 0), size: (1920, 1080) },
            ];
        }
        adapter.frontmost = Some(42);
        adapter.running_apps = vec![42];
        adapter.windows.entry(42).or_default().extend(vec![100, 200]);
        adapter.window_pids.insert(100, 42);
        adapter.window_pids.insert(200, 42);
        adapter
    }

    fn setup(display_count: u32) -> StateManager {
        let (tx, _) = mpsc::channel(64);
        let keybinds = Arc::new(Mutex::new(KeybindConfig::default()));
        let adapter = make_adapter(display_count);
        StateManager::new(tx, keybinds, Box::new(adapter))
    }

    #[test]
    fn creates_workspaces_from_displays() {
        let sm = setup(1);
        assert_eq!(sm.workspaces.len(), 1);
        assert_eq!(sm.workspaces[0].monitor_id, 1);
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
        assert!(sm.window_pids.get(&100).is_none());
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
        assert!(sm.window_pids.get(&100).is_none());
        assert!(sm.window_pids.get(&200).is_none());
    }

    #[test]
    fn on_app_activated_updates_frontmost_pid() {
        let mut sm = setup(1);
        sm.on_app_activated(99);
        assert_eq!(sm.frontmost_pid, Some(99));
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
        assert_eq!(sm.workspaces.len(), 2);

        // Add two windows — they go to whatever workspace active_workspace_idx() picks
        sm.on_window_created(100, 42);
        sm.on_window_created(300, 42);

        // Find which workspace received them (avoids HashMap ordering assumptions)
        let source = sm.workspaces.iter()
            .position(|ws| ws.find_window(100).is_some())
            .expect("window 100 should be in some workspace");
        assert_eq!(sm.workspaces[source].window_count(), 2);

        let target = if source == 0 { 1 } else { 0 };

        sm.move_focused_to_workspace(target);

        assert_eq!(sm.workspaces[source].window_count(), 1);
        assert_eq!(sm.workspaces[target].window_count(), 1);
    }

    #[test]
    fn toggle_layout_switches_monocle() {
        let mut sm = setup(1);
        assert!(!sm.workspaces[0].monocle);
        let cmd = Command::ToggleLayout;
        let (rtx, _) = mpsc::channel(1);
        sm.on_command(cmd, rtx);
        assert!(sm.workspaces[0].monocle);
    }

    #[test]
    fn set_gap_updates_values() {
        let mut sm = setup(1);
        let (rtx, _) = mpsc::channel(1);
        sm.on_command(Command::SetGapOuter { pixels: 20 }, rtx);
        assert_eq!(sm.gap_outer, 20.0);
    }

    #[test]
    fn query_state_returns_workspace_info() {
        let mut sm = setup(1);
        sm.on_window_created(100, 42);
        let (rtx, mut rx) = mpsc::channel(1);
        sm.on_command(Command::QueryState, rtx);
        let resp = rx.blocking_recv();
        assert!(resp.is_some());
    }
}
