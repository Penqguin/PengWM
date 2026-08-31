use clap::{Parser, Subcommand, ValueEnum};
use pengwm_core::command::Command;
use pengwm_core::tree::{Direction, SplitDirection};

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
    /// Toggle the status bar visibility
    ToggleBar,
    SetGapOuter {
        pixels: i32,
    },
    SetGapInner {
        pixels: i32,
    },
    ReloadConfig,
    State,
    /// Stop the daemon (and the status bar with it)
    Quit,
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
            CliCommand::ToggleBar => Command::ToggleBar,
            CliCommand::SetGapOuter { pixels } => Command::SetGapOuter { pixels },
            CliCommand::SetGapInner { pixels } => Command::SetGapInner { pixels },
            CliCommand::ReloadConfig => Command::ReloadConfig,
            CliCommand::State => Command::QueryState,
            CliCommand::Quit => Command::Quit,
        }
    }
}
