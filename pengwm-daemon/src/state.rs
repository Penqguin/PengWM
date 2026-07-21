//! StateManager — the brain of the window manager.
//!
//! Owns:
//!   - Vec<Workspace> — one per virtual desktop
//!   - PID→window tracking     (to clean up orphaned windows on app exit)
//!   - the active workspace index
//!   - current config (gaps, max_tiles, keybinds)
//!
//! Each on_* method mutates the tree, calls calculate_layout, then applies
//! the new positions via the macOS AX bindings.

use std::collections::HashMap;
use tokio::sync::mpsc;
use pengwm_core::tree::WindowId;
use pengwm_core::workspace::Workspace;
use pengwm_core::layout;

pub struct StateManager {
    //  workspaces: Vec<Workspace>,
    //  active_workspace: usize,
    //  pid_to_windows: HashMap<i32, Vec<WindowId>>,
    //  ax_senders: HashMap<i32, mpsc::Sender<DaemonEvent>>,  (for AXObserver per PID)
    //  event_tx: mpsc::Sender<DaemonEvent>,                  (to send back keybind events)
}

impl StateManager {
    pub fn new(event_tx: mpsc::Sender<DaemonEvent>) -> Self {
        //  query CGDisplay for all active monitors
        //  create one Workspace per monitor (or default to one workspace with one monitor)
        todo!()
    }

    // -----------------------------------------------------------------------
    // Window lifecycle
    // -----------------------------------------------------------------------

    pub fn on_window_created(&mut self, window_id: WindowId) {
        //  determine which workspace should own this window (focused workspace or the one
        //    on the same monitor as the window's current position)
        //  add_window to that workspace
        //  apply_layout for that workspace
        todo!()
    }

    pub fn on_window_destroyed(&mut self, window_id: WindowId) {
        //  find the workspace containing this window
        //  remove_window from that workspace
        //  apply_layout for that workspace
        //  remove window_id from pid_to_windows
        todo!()
    }

    pub fn on_window_focused(&mut self, window_id: WindowId) {
        //  find the workspace containing this window
        //  call workspace.focus_window(window_id)
        //  if the workspace isn't already active, switch to it
        todo!()
    }

    // -----------------------------------------------------------------------
    // App lifecycle
    // -----------------------------------------------------------------------

    pub fn on_app_launched(&mut self, pid: i32) {
        //  attach an AXObserver to the new app PID
        //  discover existing windows via AXUIElementCopyAttributeValue("AXWindows")
        //  fire WindowCreated for each
        todo!()
    }

    pub fn on_app_terminated(&mut self, pid: i32) {
        //  get all windows associated with pid
        //  fire WindowDestroyed for each
        //  clean up the observer
        todo!()
    }

    pub fn on_app_activated(&mut self, pid: i32) {
        //  track which app is frontmost for focus-follows-mouse or similar
        todo!()
    }

    // -----------------------------------------------------------------------
    // Monitor lifecycle
    // -----------------------------------------------------------------------

    pub fn on_monitor_added(&mut self, display_id: u32) {
        //  create a new workspace mapped to this monitor
        //  move any orphan workspaces (from a prior removal) back on-screen if needed
        todo!()
    }

    pub fn on_monitor_removed(&mut self, display_id: u32) {
        //  move all workspaces on this monitor to the primary monitor
        //  mark the workspace as detached (monitor_id = 0)
        todo!()
    }

    pub fn on_monitor_resized(&mut self, display_id: u32) {
        //  query new monitor geometry
        //  update workspace.monitor_size
        //  apply_layout for affected workspaces
        todo!()
    }

    // -----------------------------------------------------------------------
    // Commands
    // -----------------------------------------------------------------------

    pub fn on_command(
        &mut self,
        cmd: pengwm_core::command::DaemonCommand,
        tx: mpsc::Sender<pengwm_core::command::DaemonResponse>,
    ) {
        //  match cmd:
        //    FocusLeft/Righ/Up/Down
        //      → active_workspace.focus_neighbor(...)
        //    Swap* -> active_workspace.swap_window(...)
        //    SwitchWorkspace(n)    → set active_workspace, apply_layout
        //    MoveWindowToWorkspace(n) → move focused window to target workspace
        //    ToggleLayout           → cycle through BSP / monocle / floating per-workspace
        //    SetGapOuter/Inner(n)   → update config, apply_layout
        //    ReloadConfig           → re-read config.toml
        //    QueryState             → build DaemonResponse::State and send back
        //  after layout changes, call apply_layout for the affected workspace(s)
        todo!()
    }

    // -----------------------------------------------------------------------
    // Layout application
    // -----------------------------------------------------------------------

    fn apply_layout(&self, workspace_idx: usize) {
        //  get the workspace
        //  build output HashMap
        //  call calculate_layout(workspace.root, monitor_rect, workspace.arena, &mut output, gap_size)
        //  for each (window_id, rect):
        //      global = screen_local_to_global(rect, workspace.monitor_origin)
        //      macos::ax_element::set_window_rect(window_id, global)
        todo!()
    }
}
