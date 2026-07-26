use serde::{Serialize, Deserialize};
use crate::layout::Rect;
use crate::tree::{Direction, SplitDirection, WindowId};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Command {
    Focus { direction: Direction },
    MoveWindow { direction: Direction },
    Split { direction: SplitDirection },
    Workspace { id: u32 },
    MoveWindowToWorkspace { id: u32 },
    Close,
    ToggleLayout,
    SetGapOuter { pixels: i32 },
    SetGapInner { pixels: i32 },
    ReloadConfig,
    QueryState,
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
