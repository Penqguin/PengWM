# Getting Started

## Prerequisites

1. **macOS 14+** (Sequoia recommended, Ventura works)
2. **Accessibility permissions:** System Settings → Privacy & Security → Accessibility
3. **Displays have separate Spaces:** System Settings → Desktop & Dock

## Build & Run

```bash
git clone https://github.com/your-org/pengwm
cd pengwm
cargo build --release
```

### Start the daemon

```bash
./target/release/pengwm
```

The daemon will:
1. Check Accessibility permissions (exit with instructions if missing)
2. Load config from `~/.config/pengwm/config.toml`
3. Attach AXObservers to all running apps
4. Start the CGEventTap for global keybindings
5. Listen for CLI commands on `/tmp/pengwm.sock`

`pengwm` is a single binary: run it with no arguments to start the daemon,
or pass a subcommand to control a running daemon (see below).

### Control with the CLI

```bash
# Focus movement (vim-style)
./target/release/pengwm focus left
./target/release/pengwm focus down

# Split windows
./target/release/pengwm split horizontal
./target/release/pengwm split vertical

# Switch workspaces
./target/release/pengwm workspace 2
./target/release/pengwm workspace 4

# Move windows
./target/release/pengwm move-window right
./target/release/pengwm move-window-to-workspace 3

# Close focused window
./target/release/pengwm close

# Toggle monocle mode
./target/release/pengwm toggle-layout

# View daemon state
./target/release/pengwm state
```

## Default Keybindings

| Keys | Action |
|------|--------|
| `Alt-h/j/k/l` or `Alt-arrows` | Focus left/down/up/right |
| `Alt-Shift-h/j/k/l` or `Alt-Shift-arrows` | Move window into the neighbor's space (swap + resize) |
| `Alt-1..9` | Switch to workspace |
| `Alt-Shift-1..9` | Move window to workspace |
| `Alt-/` | Switch to tiling layout |
| `Alt-,` | Switch to accordion layout |
| `Cmd-Shift-r` | Reload config |

## Debugging

```bash
RUST_LOG=debug ./target/release/pengwm
```

Logs include window creation events, focus changes, keybind matches, and
layout calculations.
