use clap::{Parser, Subcommand, ValueEnum};
use pengwm_core::command::Command;
use pengwm_core::tree::{Direction, SplitDirection};
use pengwm_core::workspace::LayoutPreset;

#[derive(Parser, Debug)]
#[command(
    name = "pengwm",
    about = "PengWM — a tiling window manager for macOS.\n\nRun with no arguments to start the daemon."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<CliCommand>,
}

#[derive(Subcommand, Debug)]
pub enum CliCommand {
    /// Start the daemon (used by launchd and manual starts)
    Daemon,
    Focus {
        direction: DirectionArg,
    },
    MoveWindow {
        direction: DirectionArg,
    },
    Split {
        direction: SplitArg,
    },
    Workspace {
        id: u32,
    },
    MoveWindowToWorkspace {
        id: u32,
    },
    Close,
    ToggleLayout,
    /// Rearrange the active workspace into a named tmux-style preset
    SelectLayout {
        preset: PresetArg,
    },
    /// Grow the focused window one step toward a direction (5% steps)
    ResizePane {
        direction: DirectionArg,
    },
    /// Toggle the status bar visibility
    ToggleBar,
    SetGapOuter {
        pixels: i32,
    },
    SetGapInner {
        pixels: i32,
    },
    FocusDisplay {
        direction: DirectionArg,
    },
    MoveWindowToDisplay {
        direction: DirectionArg,
    },
    ReloadConfig,
    State,
    /// Stop the daemon (and the status bar with it)
    Quit,
    /// Re-tile all hidden windows back into their workspaces
    RevealAll,
    /// Clear the persisted session state (next launch uses config defaults)
    ClearSession,
}

#[derive(ValueEnum, Clone, Debug)]
pub enum DirectionArg {
    Left,
    Right,
    Up,
    Down,
}

#[derive(ValueEnum, Clone, Debug)]
pub enum SplitArg {
    Horizontal,
    Vertical,
}

#[derive(ValueEnum, Clone, Debug)]
pub enum PresetArg {
    EvenHorizontal,
    EvenVertical,
    MainHorizontal,
    MainVertical,
    Tiled,
}

impl From<DirectionArg> for Direction {
    fn from(d: DirectionArg) -> Self {
        match d {
            DirectionArg::Left => Direction::Left,
            DirectionArg::Right => Direction::Right,
            DirectionArg::Up => Direction::Up,
            DirectionArg::Down => Direction::Down,
        }
    }
}

impl From<SplitArg> for SplitDirection {
    fn from(d: SplitArg) -> Self {
        match d {
            SplitArg::Horizontal => SplitDirection::Horizontal,
            SplitArg::Vertical => SplitDirection::Vertical,
        }
    }
}

impl From<PresetArg> for LayoutPreset {
    fn from(p: PresetArg) -> Self {
        match p {
            PresetArg::EvenHorizontal => LayoutPreset::EvenHorizontal,
            PresetArg::EvenVertical => LayoutPreset::EvenVertical,
            PresetArg::MainHorizontal => LayoutPreset::MainHorizontal,
            PresetArg::MainVertical => LayoutPreset::MainVertical,
            PresetArg::Tiled => LayoutPreset::Tiled,
        }
    }
}

impl From<CliCommand> for Command {
    fn from(cmd: CliCommand) -> Self {
        match cmd {
            CliCommand::Daemon => unreachable!("daemon is handled before conversion"),
            CliCommand::Focus { direction } => Command::Focus {
                direction: direction.into(),
            },
            CliCommand::MoveWindow { direction } => Command::MoveWindow {
                direction: direction.into(),
            },
            CliCommand::Split { direction } => Command::Split {
                direction: direction.into(),
            },
            CliCommand::Workspace { id } => Command::Workspace { id },
            CliCommand::MoveWindowToWorkspace { id } => Command::MoveWindowToWorkspace { id },
            CliCommand::Close => Command::Close,
            CliCommand::ToggleLayout => Command::ToggleLayout,
            CliCommand::SelectLayout { preset } => Command::SelectLayout {
                preset: preset.into(),
            },
            CliCommand::ResizePane { direction } => Command::ResizePane {
                direction: direction.into(),
            },
            CliCommand::ToggleBar => Command::ToggleBar,
            CliCommand::SetGapOuter { pixels } => Command::SetGapOuter { pixels },
            CliCommand::SetGapInner { pixels } => Command::SetGapInner { pixels },
            CliCommand::FocusDisplay { direction } => Command::FocusDisplay {
                direction: direction.into(),
            },
            CliCommand::MoveWindowToDisplay { direction } => Command::MoveWindowToDisplay {
                direction: direction.into(),
            },
            CliCommand::ReloadConfig => Command::ReloadConfig,
            CliCommand::State => Command::QueryState,
            CliCommand::Quit => Command::Quit,
            CliCommand::RevealAll => Command::RevealAll,
            CliCommand::ClearSession => unreachable!("clear-session handled before IPC"),
        }
    }
}
