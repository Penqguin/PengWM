//! CLI argument definitions using clap.

use clap::{Parser, Subcommand};

/// A tiling window manager for macOS.
#[derive(Parser, Debug)]
#[command(name = "pengwm", about = "Control the PengWM daemon")]
pub struct Cli {
    #[command(subcommand)]
    pub command: CliCommand,
}

#[derive(Subcommand, Debug)]
pub enum CliCommand {
    /// Move focus in a direction
    Focus {
        direction: String,  // "left" | "right" | "up" | "down"
    },
    /// Swap the focused window with a neighbor
    Swap {
        direction: String,
    },
    /// Switch to a workspace by number
    SwitchWorkspace {
        number: u8,
    },
    /// Move the focused window to another workspace
    MoveWindowToWorkspace {
        number: u8,
    },
    /// Set outer gap (between windows and screen edge)
    SetGapOuter {
        pixels: i32,
    },
    /// Set inner gap (between adjacent windows)
    SetGapInner {
        pixels: i32,
    },
    /// Toggle between BSP and monocle layout
    ToggleLayout,
    /// Reload configuration from disk
    ReloadConfig,
    /// Print daemon state
    State,
}

impl CliCommand {
    /// Convert the CLI command into a DaemonCommand for sending over the socket.
    #[allow(dead_code)]
    pub fn to_daemon_command(&self) -> pengwm_core::command::DaemonCommand {
        //  match self and construct the appropriate DaemonCommand variant
        todo!()
    }
}
