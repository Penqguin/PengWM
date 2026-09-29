use std::time::{Duration, Instant};

use pengwm_core::command::{BarMessage, Command, DaemonResponse, LayoutMode};
use pengwm_core::tree::{Direction, WindowId};
use tokio::sync::mpsc;

use super::bar::ToggleAction;
use super::session;
use super::StateManager;

/// Owns the `Command` dispatch: the single vocabulary every surface (IPC,
/// keybinds, config watcher) feeds into `StateManager::on_command`.
/// Per-monitor switch, overflow redirect, cross-display moves and the
/// switch debounce live here; `StateManager` retains the workspace tree,
/// `DisplaySet`, layout application and `BarSender` — this module only
/// orchestrates them. The three helpers exercised directly by
/// `state::tests` are `pub(super)`; the rest is private to the module.
impl StateManager {
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
                // *focused* monitor, not a flat global index.
                let n = id as usize;
                if n != 0 {
                    let current_idx = self.active_workspace_idx();
                    if current_idx < self.workspaces.len() {
                        let current_mon = self.workspaces[current_idx].monitor_id;
                        let on_mon = self.displays.workspaces_on(current_mon, &self.workspaces);
                        if let Some(new_idx) =
                            self.displays
                                .resolve_workspace(current_idx, n, &self.workspaces)
                        {
                            if new_idx != current_idx {
                                self.displays.active_mut().insert(current_mon, new_idx);
                                for &idx in &on_mon {
                                    if idx != new_idx {
                                        self.hide_workspace(idx);
                                    }
                                }
                                let maybe_focus = if self.focus_first_on_switch
                                    && self.workspaces[new_idx].window_count() > 0
                                {
                                    self.workspaces[new_idx].focus_first()
                                } else {
                                    None
                                };
                                self.apply_layout(new_idx);
                                if let Some(wid) = maybe_focus {
                                    self.os.focus_window(wid);
                                }
                                self.switch_debounce_until =
                                    Some(Instant::now() + Duration::from_millis(150));
                            }
                        } else {
                            log::debug!(
                                "Workspace id {} out of range for monitor {} (has {} workspaces)",
                                n,
                                current_mon,
                                on_mon.len()
                            );
                        }
                    }
                }
            }
            Command::MoveWindowToWorkspace { id } => {
                let n = id as usize;
                if n != 0 {
                    let current_idx = self.active_workspace_idx();
                    if let Some(target) =
                        self.displays
                            .resolve_workspace(current_idx, n, &self.workspaces)
                    {
                        self.move_focused_to_workspace(target);
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
            Command::SelectLayout { preset } => {
                let idx = self.active_workspace_idx();
                let ratio = self.main_ratio;
                self.workspaces[idx].apply_preset(preset, ratio);
                self.apply_layout(idx);
            }
            Command::ResizePane { direction } => {
                let idx = self.active_workspace_idx();
                self.workspaces[idx].resize_focused(direction);
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
            Command::ToggleBar => match self.bar.toggle() {
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
            },
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
            Command::RevealAll => {
                log::info!("RevealAll requested — retiling hidden windows");
                self.reveal_all();
            }
        }
        if let Some(tx) = tx {
            let _ = tx.try_send(DaemonResponse::Ack);
        }
        self.publish_bar_state();
    }

    pub(super) fn focus_command(&mut self, direction: Direction) {
        let idx = self.active_workspace_idx();
        let ws = &mut self.workspaces[idx];
        ws.focus_neighbor(direction);
        if let Some(wid) = ws.focused_window_id() {
            self.os.focus_window(wid);
        }
    }

    pub(super) fn swap_command(&mut self, direction: Direction) {
        let idx = self.active_workspace_idx();
        let ws = &mut self.workspaces[idx];
        ws.swap_window(direction);
        self.apply_layout(idx);
    }

    pub(super) fn move_focused_to_workspace(&mut self, target: usize) {
        let current = self.active_workspace_idx();
        if target == current {
            return;
        }
        let window_id = self.workspaces[current].focused_window_id();
        if let Some(wid) = window_id {
            let dest = match self.displays.plan_move(&self.workspaces, current, target) {
                Some(dec) => dec.to,
                None => {
                    log::warn!(
                        "No workspace has room for {} (cap {}), move aborted",
                        wid,
                        self.displays.max_tiles()
                    );
                    self.publish_bar_state();
                    return;
                }
            };
            self.move_window_between(current, dest, wid);
        }
        self.publish_bar_state();
    }

    /// Move `wid` from workspace `from` to `to`, re-laying-out visible ends.
    /// Shared by workspace moves and cross-display moves.
    fn move_window_between(&mut self, from: usize, to: usize, wid: WindowId) {
        self.workspaces[from].remove_window(wid);
        if self.displays.is_visible(from, &self.workspaces) {
            self.apply_layout(from);
        }
        self.workspaces[to].add_window(wid, None);
        if self.displays.is_visible(to, &self.workspaces) {
            self.apply_layout(to);
        }
    }

    fn focus_display(&mut self, direction: Direction) {
        let current_idx = self.active_workspace_idx();
        if current_idx >= self.workspaces.len() {
            return;
        }
        let target_idx = match self.displays.direction_target(
            &self.workspaces,
            current_idx,
            direction,
            &*self.os,
        ) {
            Some(idx) => idx,
            None => {
                log::debug!(
                    "focus_display {:?} no target from workspace {}",
                    direction,
                    current_idx
                );
                return;
            }
        };
        if let Some(wid) = self.workspaces[target_idx].focused_window_id() {
            log::debug!(
                "focus_display {:?} workspace {} -> {} wid {}",
                direction,
                current_idx,
                target_idx,
                wid
            );
            self.os.focus_window(wid);
        } else {
            // No window to focus — just update frontmost heuristic by focusing display?
            // Publish so bar/menubar reflect focused display change.
            log::debug!(
                "focus_display {:?} target {} has no windows",
                direction,
                target_idx
            );
            self.publish_bar_state();
        }
    }

    fn move_window_to_display(&mut self, direction: Direction) {
        let current_idx = self.active_workspace_idx();
        if current_idx >= self.workspaces.len() {
            return;
        }
        let target_idx = match self.displays.direction_target(
            &self.workspaces,
            current_idx,
            direction,
            &*self.os,
        ) {
            Some(idx) => idx,
            None => return,
        };
        let wid = match self.workspaces[current_idx].focused_window_id() {
            Some(id) => id,
            None => return,
        };
        let dest = match self
            .displays
            .plan_move(&self.workspaces, current_idx, target_idx)
        {
            Some(dec) => dec.to,
            None => {
                log::warn!(
                    "No room on target display {} for window {}",
                    self.workspaces[target_idx].monitor_id,
                    wid
                );
                return;
            }
        };
        log::debug!(
            "move_window_to_display {:?} wid {} workspace {} -> {} dest {}",
            direction,
            wid,
            current_idx,
            target_idx,
            dest
        );
        self.move_window_between(current_idx, dest, wid);
        self.publish_bar_state();
    }
}
