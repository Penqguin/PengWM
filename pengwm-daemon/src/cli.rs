use clap::{Parser, Subcommand, ValueEnum};
use pengwm_core::command::Command;
use pengwm_core::release::{self, UpdateCheck};
use pengwm_core::tree::{Direction, SplitDirection};
use pengwm_core::workspace::LayoutPreset;

const INSTALL_SCRIPT_URL: &str = "https://pengwm.penqguin.com/install.sh";

#[derive(Parser, Debug)]
#[command(
    name = "pengwm",
    about = "PengWM — a tiling window manager for macOS.\n\nRun with no arguments to start the daemon.",
    version
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<CliCommand>,
}

#[derive(Subcommand, Debug)]
pub enum CliCommand {
    /// Start the daemon (used by launchd and manual starts)
    Daemon,
    /// Update the installed PengWM binaries from the latest GitHub release
    /// (does not need a running daemon). Asks before installing; answer
    /// anything but y to keep your current edition.
    Update {
        /// Update without asking (for scripts and the menubar's Update now…)
        #[arg(short = 'y', long = "yes")]
        yes: bool,
    },
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
    CycleLayout,
    ToggleMagnify,
    /// Rearrange the active workspace into a named tmux-style preset
    SelectLayout {
        preset: PresetArg,
    },
    /// Grow the focused window one step toward a direction (5% steps)
    ResizePane {
        direction: DirectionArg,
    },
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
            CliCommand::CycleLayout => Command::CycleLayout,
            CliCommand::ToggleMagnify => Command::ToggleMagnify,
            CliCommand::SelectLayout { preset } => Command::SelectLayout {
                preset: preset.into(),
            },
            CliCommand::ResizePane { direction } => Command::ResizePane {
                direction: direction.into(),
            },
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
            CliCommand::Update { .. } => unreachable!("update handled before IPC"),
            CliCommand::ClearSession => unreachable!("clear-session handled before IPC"),
        }
    }
}

/// `pengwm update` — check GitHub for the latest release and, with the
/// user's consent, hand off to the published install.sh (which downloads,
/// verifies, installs, and restarts the launchd agent). Deliberately does
/// not need the daemon socket: the daemon may be down while updating.
///
/// Consent lives here because an update costs the user an Accessibility
/// re-grant (ADR-0001's accepted trade-off for ad-hoc-signed bare
/// binaries): nobody pays that toll unasked. The default answer is No —
/// declining means staying on the current edition, which is a successful
/// outcome, not an error.
pub fn update_main(yes: bool) {
    // One-copy policy applies here too: updates modify the installed copy,
    // so a dev checkout must run them itself, not via this binary.
    pengwm_daemon::location::enforce_installed_for_update();

    match update_to_latest(yes) {
        Ok(()) => {}
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

fn update_to_latest(yes: bool) -> Result<(), String> {
    // The temp dir is removed on every exit path (success and failure) —
    // update_to_latest_in owns the steps; this wrapper owns the cleanup.
    let dir = request_temp_dir()?;
    let result = update_to_latest_in(&dir, yes);
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn update_to_latest_in(dir: &std::path::Path, yes: bool) -> Result<(), String> {
    println!("Checking for the latest PengWM release…");
    let current = env!("CARGO_PKG_VERSION");

    match release::check(current) {
        UpdateCheck::Failed { reason } => Err(format!("update check failed: {reason}")),
        UpdateCheck::UpToDate { latest } => {
            println!("PengWM v{current} is up to date (latest {latest})");
            Ok(())
        }
        UpdateCheck::UpdateAvailable { latest } => {
            println!("Latest release is {latest} (this build is v{current}).");
            if !yes {
                println!("Note: releases are ad-hoc signed — updating costs one");
                println!("Accessibility re-grant in System Settings.");
                if !confirm_install(&latest)? {
                    println!("Staying on v{current}. Run `pengwm update` to upgrade later.");
                    return Ok(());
                }
            }
            install_release(dir, &latest)
        }
    }
}

/// The consent seam. Prints the question and reads one line; EOF (piped
/// stdin, closed terminal) counts as No, so a scripted call without `--yes`
/// still lands on "keep my current edition" instead of hanging.
fn confirm_install(latest: &str) -> Result<bool, String> {
    use std::io::Write;
    print!("Install PengWM {latest} now? [y/N] ");
    std::io::stdout()
        .flush()
        .map_err(|e| format!("could not write prompt: {e}"))?;
    let mut line = String::new();
    match std::io::stdin().read_line(&mut line) {
        Ok(0) => Ok(false),
        Ok(_) => Ok(accepts(&line)),
        Err(e) => Err(format!("could not read confirmation: {e}")),
    }
}

/// The consent rule, pure so it tests without stdin: `y`/`yes` installs,
/// anything else (including empty) keeps the current edition.
fn accepts(line: &str) -> bool {
    matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

fn install_release(dir: &std::path::Path, tag: &str) -> Result<(), String> {
    let script_path = dir.join("install.sh");

    println!("Downloading the installer…");
    let script = curl(INSTALL_SCRIPT_URL, None)?;
    if script.is_empty() {
        return Err(format!(
            "downloaded installer from {INSTALL_SCRIPT_URL} is empty"
        ));
    }
    std::fs::write(&script_path, script)
        .map_err(|e| format!("failed to write {}: {e}", script_path.display()))?;

    // install.sh does download + checksum/signature verification + install
    // into the blessed root + agent restart — one tested code path instead
    // of reimplementing it here.
    println!("Installing PengWM {tag} (the installer downloads and verifies the release)…");
    let status = std::process::Command::new("/bin/bash")
        .arg("install.sh")
        .arg("--version")
        .arg(tag)
        .current_dir(dir)
        .status()
        .map_err(|e| format!("failed to run {}: {e}", script_path.display()))?;
    if !status.success() {
        return Err(format!(
            "installer failed ({status}); PengWM was not updated"
        ));
    }

    println!("Updated. The daemon restarts itself via launchd.");
    Ok(())
}

/// Fetch `url` with the system curl (keeps the daemon dependency-free).
fn curl(url: &str, accept: Option<&str>) -> Result<Vec<u8>, String> {
    let mut cmd = std::process::Command::new("/usr/bin/curl");
    cmd.arg("-fsSL");
    if let Some(accept) = accept {
        cmd.arg("-H").arg(format!("Accept: {accept}"));
    }
    cmd.arg(url);
    let out = cmd
        .output()
        .map_err(|e| format!("failed to run curl: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "curl failed for {url} (exit status {})",
            out.status.code().unwrap_or(-1)
        ));
    }
    Ok(out.stdout)
}

fn request_temp_dir() -> Result<std::path::PathBuf, String> {
    let dir = std::env::temp_dir().join(format!("pengwm-update-{}", std::process::id()));
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("failed to create {}: {e}", dir.display()))?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consent_rule() {
        assert!(accepts("y\n"));
        assert!(accepts("Y"));
        assert!(accepts(" yes "));
        assert!(!accepts("n\n"));
        assert!(!accepts(""));
        assert!(!accepts("no\n"));
        assert!(!accepts("y es"));
    }
}
