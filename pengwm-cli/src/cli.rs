use clap::{Parser, Subcommand, ValueEnum};
use pengwm_core::tree::{Direction, SplitDirection};
use pengwm_core::command::Command;

#[derive(Parser, Debug)]
#[command(name = "pengwm", about = "Control the PengWM daemon")]
pub struct Cli {
    #[command(subcommand)]
    pub command: CliCommand,
}

#[derive(Subcommand, Debug)]
pub enum CliCommand {
    Focus { direction: DirectionArg },
    MoveWindow { direction: DirectionArg },
    Split { direction: SplitArg },
    Workspace { id: u32 },
    MoveWindowToWorkspace { id: u32 },
    Close,
    ToggleLayout,
    SetGapOuter { pixels: i32 },
    SetGapInner { pixels: i32 },
    ReloadConfig,
    State,
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
            CliCommand::Focus { direction } => Command::Focus { direction: direction.into() },
            CliCommand::MoveWindow { direction } => Command::MoveWindow { direction: direction.into() },
            CliCommand::Split { direction } => Command::Split { direction: direction.into() },
            CliCommand::Workspace { id } => Command::Workspace { id },
            CliCommand::MoveWindowToWorkspace { id } => Command::MoveWindowToWorkspace { id },
            CliCommand::Close => Command::Close,
            CliCommand::ToggleLayout => Command::ToggleLayout,
            CliCommand::SetGapOuter { pixels } => Command::SetGapOuter { pixels },
            CliCommand::SetGapInner { pixels } => Command::SetGapInner { pixels },
            CliCommand::ReloadConfig => Command::ReloadConfig,
            CliCommand::State => Command::QueryState,
        }
    }
}
