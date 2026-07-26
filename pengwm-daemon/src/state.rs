use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use pengwm_core::tree::{WindowId, NodeData};
use pengwm_core::workspace::Workspace;
use pengwm_core::layout::{self, Rect};
use pengwm_core::command::{Command, DaemonResponse};
use crate::event_loop::DaemonEvent;
use crate::macos::ax_element;
use crate::macos::ax_observer::ObserverRegistry;
use crate::macos::cg_display;
use crate::config::keybinds::KeybindConfig;

pub struct StateManager {
    workspaces: Vec<Workspace>,
    active_workspaces: HashMap<u32, usize>,
    frontmost_pid: Option<i32>,
    pid_to_windows: HashMap<i32, Vec<WindowId>>,
    window_pids: HashMap<WindowId, i32>,
    observer_registry: ObserverRegistry,
    event_tx: mpsc::Sender<DaemonEvent>,
    gap_outer: f64,
    gap_inner: f64,
    keybinds: Arc<Mutex<KeybindConfig>>,
}

const OFFSCREEN: Rect = Rect { x: -9999.0, y: 0.0, width: 1.0, height: 1.0 };

impl StateManager {
    pub fn new(event_tx: mpsc::Sender<DaemonEvent>, keybinds: Arc<Mutex<KeybindConfig>>) -> Self {
        let mut observer_registry = ObserverRegistry::new(event_tx.clone());
        let displays = cg_display::active_displays();
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
                let _ = event_tx.try_send(DaemonEvent::WindowCreated(window_id, pid));
            }
        }

        let state = Self {
            workspaces,
            active_workspaces,
            frontmost_pid,
            pid_to_windows,
            window_pids,
            observer_registry,
            event_tx,
            gap_outer: 10.0,
            gap_inner: 5.0,
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
        self.observer_registry.attach(pid);
        let windows = unsafe { ax_element::windows_for_pid(pid) };
        for (_element, window_id) in windows {
            self.window_pids.insert(window_id, pid);
            self.pid_to_windows.entry(pid).or_default().push(window_id);
            let _ = self.event_tx.try_send(DaemonEvent::WindowCreated(window_id, pid));
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
        let idx = self.workspaces.len();
        self.active_workspaces.insert(display_id, idx);
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
        self.active_workspaces.retain(|_, idx| *idx < self.workspaces.len());
        if self.active_workspaces.is_empty() {
            self.active_workspaces.insert(self.workspaces[0].monitor_id, 0);
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
        cmd: Command,
        tx: mpsc::Sender<DaemonResponse>,
    ) {
        match cmd {
            Command::Focus { direction } => self.focus_command(direction),
            Command::MoveWindow { direction } => self.swap_command(direction),
            Command::Split { direction } => {
                let idx = self.active_workspace_idx();
                let ws = &mut self.workspaces[idx];
                if let Some(node_id) = ws.focused_node {
                    if ws.arena.get(node_id).is_some_and(|n| matches!(n.data, NodeData::Window { .. })) {
                        ws.pending_split = Some(direction);
                    } else if let NodeData::Split { direction: ref mut dir, .. } = &mut ws.arena.get_mut(node_id).unwrap().data {
                        *dir = direction;
                        ws.flatten_split_if_redundant(node_id);
                    }
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
                let window_id = {
                    let ws = &self.workspaces[idx];
                    ws.focused_node.and_then(|nid| {
                        if let NodeData::Window { window_id, .. } = &ws.arena.get(nid)?.data {
                            Some(*window_id)
                        } else {
                            None
                        }
                    })
                };
                if let Some(wid) = window_id {
                    if let Some(&pid) = self.window_pids.get(&wid) {
                        unsafe {
                            if let Some(element) = ax_element::find_element(pid, wid) {
                                ax_element::close_window(element);
                            }
                        }
                    }
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
        let window_id = {
            let ws = &self.workspaces[current];
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
            self.workspaces[current].remove_window(wid);
            self.apply_layout(current);
            self.workspaces[target].add_window(wid, None);
            self.apply_layout(target);
        }
    }

    fn reload_config(&self) {
        log::info!("Reloading config...");
        let updated = KeybindConfig::load();
        let mut keybinds = self.keybinds.lock().expect("keybind mutex poisoned");
        *keybinds = updated;
        log::info!("Config reloaded successfully ({} bindings)", keybinds.bindings.len());
    }

    fn hide_workspace(&self, workspace_idx: usize) {
        let ws = &self.workspaces[workspace_idx];
        for window_id in ws.all_windows() {
            if let Some(&pid) = self.window_pids.get(&window_id) {
                unsafe {
                    if let Some(element) = ax_element::find_element(pid, window_id) {
                        let _ = ax_element::set_window_rect(element, OFFSCREEN);
                    }
                }
            }
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

        if ws.monocle {
            if let Some(focused) = ws.focused_node {
                if let Some(node) = ws.arena.get(focused) {
                    if let pengwm_core::tree::NodeData::Window { window_id, .. } = &node.data {
                        output.insert(*window_id, inset);
                    }
                }
            }
        } else {
            layout::calculate_layout(root, inset, &ws.arena, &mut output, self.gap_inner);
        }

        for (&window_id, rect) in &output {
            let global = layout::screen_local_to_global(*rect, ws.monitor_origin);
            let pid = match self.window_pids.get(&window_id) {
                Some(&pid) => pid,
                None => {
                    log::warn!("apply_layout: skipping window {} (no PID mapped)", window_id);
                    continue;
                }
            };
            unsafe {
                match ax_element::find_element(pid, window_id) {
                    Some(element) => {
                        if let Err(e) = ax_element::set_window_rect(element, global) {
                            log::error!("apply_layout: set_window_rect failed for window {} pid {}: {}", window_id, pid, e);
                        }
                    }
                    None => {
                        log::warn!("apply_layout: find_element returned None for window {} pid {}", window_id, pid);
                    }
                }
            }
        }

        if ws.monocle {
            for window_id in ws.all_windows() {
                if !output.contains_key(&window_id) {
                    if let Some(&pid) = self.window_pids.get(&window_id) {
                        unsafe {
                            if let Some(element) = ax_element::find_element(pid, window_id) {
                                let _ = ax_element::set_window_rect(element, OFFSCREEN);
                            }
                        }
                    }
                }
            }
        }
    }
}

use pengwm_core::tree::Direction;
