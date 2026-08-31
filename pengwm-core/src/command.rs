use crate::layout::Rect;
use crate::tree::{Direction, SplitDirection, WindowId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// Shut the daemon down (and the bar with it). Used by the menubar's Quit
    /// item and `pengwm quit`.
    Quit,
}

impl Command {
    /// Parse one action string from the shared command vocabulary (the
    /// keybind-config surface). Every string is the kebab-case of a [`Command`]
    /// variant plus its arguments, so the keybind surface can never drift from
    /// the wire type it feeds: `move-window-left`, `set-layout-tile`,
    /// `workspace-3`, …
    pub fn parse_action(s: &str) -> Option<Command> {
        for (name, action) in ACTION_TABLE {
            if s == *name {
                return Some(action.clone());
            }
        }
        if let Some(n) = s.strip_prefix("workspace-") {
            return Command::parse_id(n).map(|id| Command::Workspace { id });
        }
        if let Some(n) = s.strip_prefix("move-window-to-workspace-") {
            return Command::parse_id(n).map(|id| Command::MoveWindowToWorkspace { id });
        }
        if let Some(n) = s.strip_prefix("set-gap-outer-") {
            return n
                .parse::<i32>()
                .ok()
                .map(|pixels| Command::SetGapOuter { pixels });
        }
        if let Some(n) = s.strip_prefix("set-gap-inner-") {
            return n
                .parse::<i32>()
                .ok()
                .map(|pixels| Command::SetGapInner { pixels });
        }
        None
    }

    fn parse_id(n: &str) -> Option<u32> {
        n.parse::<u32>().ok().filter(|&n| n > 0)
    }
}

/// The single table of action names → [`Command`]. Keybind configs parse
/// through this so their vocabulary is exactly the wire type's.
const ACTION_TABLE: &[(&str, Command)] = &[
    (
        "focus-left",
        Command::Focus {
            direction: Direction::Left,
        },
    ),
    (
        "focus-right",
        Command::Focus {
            direction: Direction::Right,
        },
    ),
    (
        "focus-up",
        Command::Focus {
            direction: Direction::Up,
        },
    ),
    (
        "focus-down",
        Command::Focus {
            direction: Direction::Down,
        },
    ),
    (
        "move-window-left",
        Command::MoveWindow {
            direction: Direction::Left,
        },
    ),
    (
        "move-window-right",
        Command::MoveWindow {
            direction: Direction::Right,
        },
    ),
    (
        "move-window-up",
        Command::MoveWindow {
            direction: Direction::Up,
        },
    ),
    (
        "move-window-down",
        Command::MoveWindow {
            direction: Direction::Down,
        },
    ),
    (
        "split-horizontal",
        Command::Split {
            direction: SplitDirection::Horizontal,
        },
    ),
    (
        "split-vertical",
        Command::Split {
            direction: SplitDirection::Vertical,
        },
    ),
    ("close", Command::Close),
    ("toggle-layout", Command::ToggleLayout),
    (
        "set-layout-tile",
        Command::SetLayout {
            mode: LayoutMode::Tile,
        },
    ),
    (
        "set-layout-accordion",
        Command::SetLayout {
            mode: LayoutMode::Accordion,
        },
    ),
    ("toggle-bar", Command::ToggleBar),
    ("reload-config", Command::ReloadConfig),
    ("query-state", Command::QueryState),
    ("quit", Command::Quit),
];

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum LayoutMode {
    Tile,
    Accordion,
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
    /// Display names of the apps owning each window in this workspace (e.g.
    /// `["Safari", "Terminal"]`). One entry per window; consumers that only
    /// need counts ignore it. `#[serde(default)]` keeps old payloads readable.
    #[serde(default)]
    pub windows: Vec<String>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_action_covers_every_command() {
        assert_eq!(
            Command::parse_action("focus-left"),
            Some(Command::Focus {
                direction: Direction::Left
            })
        );
        assert_eq!(
            Command::parse_action("move-window-right"),
            Some(Command::MoveWindow {
                direction: Direction::Right
            })
        );
        assert_eq!(
            Command::parse_action("split-horizontal"),
            Some(Command::Split {
                direction: SplitDirection::Horizontal
            })
        );
        assert_eq!(
            Command::parse_action("split-vertical"),
            Some(Command::Split {
                direction: SplitDirection::Vertical
            })
        );
        assert_eq!(Command::parse_action("close"), Some(Command::Close));
        assert_eq!(
            Command::parse_action("toggle-layout"),
            Some(Command::ToggleLayout)
        );
        assert_eq!(
            Command::parse_action("set-layout-tile"),
            Some(Command::SetLayout {
                mode: LayoutMode::Tile
            })
        );
        assert_eq!(
            Command::parse_action("set-layout-accordion"),
            Some(Command::SetLayout {
                mode: LayoutMode::Accordion
            })
        );
        assert_eq!(
            Command::parse_action("set-gap-outer-12"),
            Some(Command::SetGapOuter { pixels: 12 })
        );
        assert_eq!(
            Command::parse_action("set-gap-inner-6"),
            Some(Command::SetGapInner { pixels: 6 })
        );
        assert_eq!(
            Command::parse_action("toggle-bar"),
            Some(Command::ToggleBar)
        );
        assert_eq!(
            Command::parse_action("reload-config"),
            Some(Command::ReloadConfig)
        );
        assert_eq!(
            Command::parse_action("query-state"),
            Some(Command::QueryState)
        );
        assert_eq!(Command::parse_action("quit"), Some(Command::Quit));
    }

    #[test]
    fn parse_action_ids() {
        assert_eq!(
            Command::parse_action("workspace-3"),
            Some(Command::Workspace { id: 3 })
        );
        assert_eq!(
            Command::parse_action("move-window-to-workspace-5"),
            Some(Command::MoveWindowToWorkspace { id: 5 })
        );
    }

    #[test]
    fn parse_action_rejects_invalid() {
        assert_eq!(Command::parse_action("swap-left"), None);
        assert_eq!(Command::parse_action("workspace-0"), None);
        assert_eq!(Command::parse_action("workspace-"), None);
        assert_eq!(Command::parse_action("do-the-hokey-pokey"), None);
        assert_eq!(Command::parse_action(""), None);
    }

    #[test]
    fn parse_action_accepts_any_positive_id() {
        assert_eq!(
            Command::parse_action("workspace-12"),
            Some(Command::Workspace { id: 12 })
        );
    }
}
