use clap::{Parser, Subcommand, ValueEnum};
use pengwm_core::command::Command;
use pengwm_core::tree::{Direction, SplitDirection};
use pengwm_core::workspace::LayoutPreset;

const GITHUB_LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/Penqguin/PengWM/releases/latest";
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
    /// (does not need a running daemon)
    Update,
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
            CliCommand::Update => unreachable!("update handled before IPC"),
            CliCommand::ClearSession => unreachable!("clear-session handled before IPC"),
        }
    }
}

/// `pengwm update` — check GitHub for the latest release and, if newer than
/// this build, hand off to the published install.sh (which downloads,
/// verifies, installs, and restarts the launchd agent). Deliberately does
/// not need the daemon socket: the daemon may be down while updating.
pub fn update_main() {
    // One-copy policy applies here too: updates modify the installed copy,
    // so a dev checkout must run them itself, not via this binary.
    pengwm_daemon::location::enforce_installed_for_update();

    match update_to_latest() {
        Ok(()) => {}
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

fn update_to_latest() -> Result<(), String> {
    // The temp dir is removed on every exit path (success and failure) —
    // update_to_latest_in owns the steps; this wrapper owns the cleanup.
    let dir = request_temp_dir()?;
    let result = update_to_latest_in(&dir);
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn update_to_latest_in(dir: &std::path::Path) -> Result<(), String> {
    println!("Checking for the latest PengWM release…");
    let tag = latest_release_tag()?;
    let current = env!("CARGO_PKG_VERSION");

    if !tag_is_newer(current, &tag) {
        println!("PengWM v{current} is up to date (latest {tag})");
        return Ok(());
    }

    println!("Latest release is {tag} (this build is v{current}).");
    let script_path = dir.join("install.sh");

    println!("Downloading the installer…");
    let script = curl(INSTALL_SCRIPT_URL, None)?;
    if script.is_empty() {
        return Err(format!("downloaded installer from {INSTALL_SCRIPT_URL} is empty"));
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
        .arg(&tag)
        .current_dir(dir)
        .status()
        .map_err(|e| format!("failed to run {}: {e}", script_path.display()))?;
    if !status.success() {
        return Err(format!("installer failed ({status}); PengWM was not updated"));
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
    let out = cmd.output().map_err(|e| format!("failed to run curl: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "curl failed for {url} (exit status {})",
            out.status.code().unwrap_or(-1)
        ));
    }
    Ok(out.stdout)
}

/// Resolve the `tag_name` of the latest GitHub release with a tiny
/// hand-rolled JSON extraction (tags are plain strings, no escapes needed).
fn latest_release_tag() -> Result<String, String> {
    let body = String::from_utf8(
        curl(GITHUB_LATEST_RELEASE_URL, Some("application/vnd.github+json"))?,
    )
    .map_err(|e| format!("release list is not valid UTF-8: {e}"))?;
    match json_string_field(&body, "tag_name") {
        Some(tag) if !tag.is_empty() => Ok(tag.to_string()),
        _ => Err(format!(
            "could not parse a release tag from {GITHUB_LATEST_RELEASE_URL}"
        )),
    }
}

/// Pull `"key": "value"` out of a JSON document, tolerating variable spacing.
fn json_string_field<'a>(body: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!("\"{key}\"");
    let rest = &body[body.find(&needle)? + needle.len()..];
    let rest = &rest[rest.find(':')? + 1..];
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(&rest[..end])
}

/// Numeric version compare without a semver crate: compare `.` components
/// numerically and ignore prerelease suffixes (v1.2.3-rc1 parses as 1.2.3).
/// When the numeric cores are equal, a tag that merely differs from the
/// current version still counts as a new release worth offering.
fn tag_is_newer(current: &str, tag: &str) -> bool {
    let tag_num = numeric_version(tag);
    let cur_num = numeric_version(current);
    let n = tag_num.len().max(cur_num.len());
    for i in 0..n {
        let t = tag_num.get(i).copied().unwrap_or(0);
        let c = cur_num.get(i).copied().unwrap_or(0);
        if t != c {
            return t > c;
        }
    }
    tag != current && tag != format!("v{current}")
}

fn numeric_version(version: &str) -> Vec<u64> {
    let core = version.trim_start_matches('v').split(['-', '+']).next().unwrap_or(version);
    core.split('.').filter_map(|part| part.parse().ok()).collect()
}

fn request_temp_dir() -> Result<std::path::PathBuf, String> {
    let dir = std::env::temp_dir().join(format!("pengwm-update-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("failed to create {}: {e}", dir.display()))?;
    Ok(dir)
}
