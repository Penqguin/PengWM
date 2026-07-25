use std::collections::HashMap;
use tokio::sync::mpsc;
use pengwm_core::tree::WindowId;
use pengwm_core::workspace::Workspace;
use pengwm_core::layout::{self, Rect};
use pengwm_core::command::{DaemonCommand, DaemonResponse};
use crate::event_loop::DaemonEvent;
use crate::macos::ax_element;
use crate::macos::ax_observer::ObserverRegistry;
use crate::macos::cg_display;

pub struct StateManager {
    workspaces: Vec<Workspace>,
    active_workspace: usize,
    frontmost_pid: Option<i32>,
    pid_to_windows: HashMap<i32, Vec<WindowId>>,
    window_pids: HashMap<WindowId, i32>,
    observer_registry: ObserverRegistry,
    event_tx: mpsc::Sender<DaemonEvent>,
    gap_outer: f64,
    gap_inner: f64,
}

impl StateManager {
    pub fn new(event_tx: mpsc::Sender<DaemonEvent>) -> Self {
        let mut observer_registry = ObserverRegistry::new(event_tx.clone());
        let displays = cg_display::active_displays();
        let mut workspaces = Vec::new();
        let mut pid_to_windows: HashMap<i32, Vec<WindowId>> = HashMap::new();
        let mut window_pids: HashMap<WindowId, i32> = HashMap::new();

        for (i, display) in displays.iter().enumerate() {
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
                cg_display::primary_display_id(),
                (0, 0),
                (1920, 1080),
            ));
        }

        let frontmost_pid = ax_element::frontmost_pid();

        #[cfg(target_os = "macos")]
        for pid in crate::macos::ns_workspace::running_app_pids() {
            observer_registry.attach(pid);
            let windows = unsafe { ax_element::windows_for_pid(pid) };
            for (_element, window_id) in windows {
                window_pids.insert(window_id, pid);
                pid_to_windows.entry(pid).or_default().push(window_id);
                let _ = event_tx.try_send(DaemonEvent::WindowCreated(window_id));
            }
        }

        Self {
            workspaces,
            active_workspace: 0,
            frontmost_pid,
            pid_to_windows,
            window_pids,
            observer_registry,
            event_tx,
            gap_outer: 10.0,
            gap_inner: 5.0,
        }
    }

    pub fn on_window_created(&mut self, window_id: WindowId) {
        let target = self.active_workspace;
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
                self.active_workspace = i;
                return;
            }
        }
    }

    pub fn on_app_launched(&mut self, pid: i32) {
        log::info!("App launched: pid={}", pid);
        self.observer_registry.attach(pid);
        let windows = unsafe { ax_element::windows_for_pid(pid) };
        for (_element, window_id) in windows {
            self.window_pids.insert(window_id, pid);
            self.pid_to_windows.entry(pid).or_default().push(window_id);
            let _ = self.event_tx.try_send(DaemonEvent::WindowCreated(window_id));
        }
    }

    pub fn on_app_terminated(&mut self, pid: i32) {
        log::info!("App terminated: pid={}", pid);
        self.observer_registry.detach(pid);
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
        if let Some(window_id) = unsafe { ax_element::focused_window_for_pid(pid) } {
            self.on_window_focused(window_id);
        }
    }

    pub fn on_monitor_added(&mut self, display_id: u32) {
        let info = cg_display::active_displays()
            .into_iter()
            .find(|d| d.id == display_id);
        let (origin, size) = match info {
            Some(d) => (d.origin, d.size),
            None => return,
        };
        let name = format!("ws-{}", self.workspaces.len() + 1);
        let ws = Workspace::new(name, display_id, origin, size);
        self.workspaces.push(ws);
    }

    pub fn on_monitor_removed(&mut self, _display_id: u32) {
        let primary = cg_display::primary_display_id();
        let primary_origin = cg_display::active_displays()
            .into_iter()
            .find(|d| d.id == primary)
            .map(|d| d.origin)
            .unwrap_or((0, 0));
        for ws in &mut self.workspaces {
            if ws.monitor_id == _display_id {
                ws.monitor_id = primary;
                ws.monitor_origin = primary_origin;
            }
        }
        self.workspaces.retain(|ws| {
            cg_display::active_displays().iter().any(|d| d.id == ws.monitor_id)
        });
        if self.workspaces.is_empty() {
            self.workspaces.push(Workspace::new(
                "ws-1".into(),
                primary,
                primary_origin,
                (1920, 1080),
            ));
        }
        if self.active_workspace >= self.workspaces.len() {
            self.active_workspace = 0;
        }
    }

    pub fn on_monitor_resized(&mut self, display_id: u32) {
        let info = cg_display::active_displays()
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
        cmd: DaemonCommand,
        tx: mpsc::Sender<DaemonResponse>,
    ) {
        match cmd {
            DaemonCommand::FocusLeft => self.focus_command(Direction::Left),
            DaemonCommand::FocusRight => self.focus_command(Direction::Right),
            DaemonCommand::FocusUp => self.focus_command(Direction::Up),
            DaemonCommand::FocusDown => self.focus_command(Direction::Down),
            DaemonCommand::SwapLeft => self.swap_command(Direction::Left),
            DaemonCommand::SwapRight => self.swap_command(Direction::Right),
            DaemonCommand::SwapUp => self.swap_command(Direction::Up),
            DaemonCommand::SwapDown => self.swap_command(Direction::Down),
            DaemonCommand::SwitchWorkspace(n) => {
                if n > 0 && (n as usize) <= self.workspaces.len() {
                    self.active_workspace = (n - 1) as usize;
                    self.apply_layout(self.active_workspace);
                }
            }
            DaemonCommand::MoveWindowToWorkspace(n) => {
                if n > 0 && (n as usize) <= self.workspaces.len() {
                    self.move_focused_to_workspace((n - 1) as usize);
                }
            }
            DaemonCommand::ToggleLayout => {
                log::info!("ToggleLayout not yet implemented");
            }
            DaemonCommand::SetGapOuter(val) => {
                self.gap_outer = val.max(0) as f64;
                self.apply_layout(self.active_workspace);
            }
            DaemonCommand::SetGapInner(val) => {
                self.gap_inner = val.max(0) as f64;
                self.apply_layout(self.active_workspace);
            }
            DaemonCommand::ReloadConfig => {
                log::info!("ReloadConfig not yet implemented");
            }
            DaemonCommand::QueryState => {
                let info = self.workspaces.iter().map(|ws| {
                    pengwm_core::command::WorkspaceInfo {
                        name: ws.name.clone(),
                        monitor_id: ws.monitor_id,
                        window_count: ws.window_count(),
                        focused_window: ws.focused_node.and_then(|nid| {
                            if let pengwm_core::tree::NodeData::Window { window_id, .. } =
                                &ws.arena.get(nid)?.data
                            {
                                Some(*window_id)
                            } else {
                                None
                            }
                        }),
                    }
                }).collect();
                let _ = tx.try_send(DaemonResponse::State { workspaces: info });
            }
        }
    }

    fn focus_command(&mut self, direction: Direction) {
        let ws = &mut self.workspaces[self.active_workspace];
        ws.focus_neighbor(direction);
    }

    fn swap_command(&mut self, direction: Direction) {
        let ws = &mut self.workspaces[self.active_workspace];
        ws.swap_window(direction);
        self.apply_layout(self.active_workspace);
    }

    fn move_focused_to_workspace(&mut self, target: usize) {
        if target == self.active_workspace {
            return;
        }
        let window_id = {
            let ws = &self.workspaces[self.active_workspace];
            ws.focused_node.and_then(|nid| {
                if let pengwm_core::tree::NodeData::Window { window_id, .. } =
                    &ws.arena.get(nid)?.data
                {
                    Some(*window_id)
                } else {
                    None
                }
            })
        };
        if let Some(wid) = window_id {
            self.workspaces[self.active_workspace].remove_window(wid);
            self.apply_layout(self.active_workspace);
            self.workspaces[target].add_window(wid, None);
            self.apply_layout(target);
        }
    }

    fn apply_layout(&self, workspace_idx: usize) {
        let ws = &self.workspaces[workspace_idx];
        let Some(root) = ws.root else { return };

        let monitor_rect = Rect::new(
            0.0, 0.0,
            ws.monitor_size.0 as f64,
            ws.monitor_size.1 as f64,
        );
        let inset = layout::inset_rect(monitor_rect, self.gap_outer);
        let mut output = HashMap::new();
        layout::calculate_layout(root, inset, &ws.arena, &mut output, self.gap_inner);

        for (&window_id, rect) in &output {
            let global = layout::screen_local_to_global(*rect, ws.monitor_origin);
            let pid = match self.window_pids.get(&window_id) {
                Some(&pid) => pid,
                None => continue,
            };
            unsafe {
                if let Some(element) = ax_element::find_element(pid, window_id) {
                    let _ = ax_element::set_window_rect(element, global);
                }
            }
        }
    }
}

use pengwm_core::tree::Direction;
