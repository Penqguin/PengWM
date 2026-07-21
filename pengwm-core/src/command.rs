//! IPC protocol types shared between the daemon and CLI client.
//!
//! Serialized as JSON over a Unix Domain Socket (/tmp/pengwm.sock).

use serde::{Serialize, Deserialize};
use crate::layout::Rect;
use crate::tree::WindowId;

// ---------------------------------------------------------------------------
// Commands (CLI → Daemon)
// ---------------------------------------------------------------------------

/// Every message the CLI can send to the daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DaemonCommand {
    // -- Focus movement --
    FocusLeft,
    FocusRight,
    FocusUp,
    FocusDown,

    // -- Window movement --
    SwapLeft,
    SwapRight,
    SwapUp,
    SwapDown,

    // -- Workspace management --
    /// Switch to workspace by 1-indexed number.
    SwitchWorkspace(u8),
    /// Move the focused window to another workspace.
    MoveWindowToWorkspace(u8),

    // -- Layout --
    /// Toggle between BSP and monocle (fullscreen) for the focused workspace.
    ToggleLayout,

    // -- Configuration --
    /// Set outer (screen-edge) gap in points.
    SetGapOuter(i32),
    /// Set inner (inter-window) gap in points.
    SetGapInner(i32),
    /// Reload config.toml at runtime.
    ReloadConfig,

    // -- Query --
    /// Request the full daemon state for the CLI to display.
    QueryState,
}

// ---------------------------------------------------------------------------
// Responses (Daemon → CLI)
// ---------------------------------------------------------------------------

/// Every message the daemon can send back to the CLI.
#[derive(Debug, Serialize, Deserialize)]
pub enum DaemonResponse {
    /// Action completed successfully.
    Ack,

    /// State snapshot, returned in reply to QueryState.
    State {
        workspaces: Vec<WorkspaceInfo>,
    },

    /// Something went wrong.
    Error { message: String },
}

/// Human-friendly representation of a workspace for CLI display.
#[derive(Debug, Serialize, Deserialize)]
pub struct WorkspaceInfo {
    pub name: String,
    pub monitor_id: u32,
    pub window_count: usize,
    pub focused_window: Option<WindowId>,
}

/// Human-friendly representation of a window rect for CLI display.
#[derive(Debug, Serialize, Deserialize)]
pub struct WindowInfo {
    pub id: WindowId,
    pub rect: Rect,
}
