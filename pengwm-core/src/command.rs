use crate::layout::Rect;
use crate::tree::{Direction, SplitDirection, WindowId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Command {
    Focus { direction: Direction },
    MoveWindow { direction: Direction },
    Split { direction: SplitDirection },
    Workspace { id: u32 },
    MoveWindowToWorkspace { id: u32 },
    Close,
    ToggleLayout,
    SetLayout { mode: LayoutMode },
    SetGapOuter { pixels: i32 },
    SetGapInner { pixels: i32 },
    ToggleBar,
    ReloadConfig,
    QueryState,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum LayoutMode {
    Tile,
    Accordion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BarPosition {
    #[default]
    Top,
    Bottom,
    Left,
    Right,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum DaemonResponse {
    Ack,
    State { workspaces: Vec<WorkspaceInfo> },
    Error { message: String },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WorkspaceInfo {
    pub name: String,
    pub monitor_id: u32,
    pub window_count: usize,
    pub focused_window: Option<WindowId>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WindowInfo {
    pub id: WindowId,
    pub rect: Rect,
}

/// Messages the daemon pushes to a connected `pengwm-bar` over the bar socket.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BarMessage {
    Show,
    Hide,
    Exit,
    Reload,
    State(BarState),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BarWorkspace {
    pub name: String,
    pub monitor_id: u32,
    pub window_count: usize,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BarState {
    pub workspaces: Vec<BarWorkspace>,
    /// Index into `workspaces` of the currently focused workspace.
    pub active_workspace: usize,
    /// Split direction of the active workspace's focused split container
    /// (drives the split-direction icon). `None` when there is no split.
    pub split_direction: Option<SplitDirection>,
    /// Global-coordinate rect of the bar strip on the primary display, as
    /// reserved by the window manager. The bar positions itself exactly here.
    /// `None` while the bar is hidden.
    #[serde(default)]
    pub rect: Option<Rect>,
}
