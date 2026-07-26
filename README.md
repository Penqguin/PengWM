# PengWM 🐧

A tiling window manager for macOS built in Rust. No System Integrity Protection (SIP) disabling
required — uses only public Apple APIs (Accessibility & Core Graphics).

## Quick Start

```bash
# Build from source
git clone https://github.com/your-org/pengwm
cd pengwm
cargo build --release

# Grant Accessibility permissions first, then start the daemon
./target/release/pengwm-daemon

# In another terminal, control it via the CLI
./target/release/pengwm focus left
./target/release/pengwm split horizontal
./target/release/pengwm workspace 3
./target/release/pengwm close
```

## Prerequisites

1. **macOS 14+** (Ventura should work, Sequoia tested)
2. **Accessibility permissions:** System Settings → Privacy & Security → Accessibility → add
   your terminal (or `pengwm-daemon` binary directly after code-signing)
3. **Displays have separate Spaces:** System Settings → Desktop & Dock → turn on
   _Displays have separate Spaces_

## Configuration

PengWM looks for `~/.config/pengwm/config.toml` (or `$XDG_CONFIG_HOME/pengwm/config.toml`).
If no file exists, defaults are used and a config watcher reloads changes at runtime.

```toml
gap_outer = 10
gap_inner = 5
max_tiles = 4
mod_key = "cmd"
```

### Keybindings

Keybindings are defined in the same file with `modifier-key = "action"` entries:

```toml
cmd-h     = "focus-left"
cmd-j     = "focus-down"
cmd-k     = "focus-up"
cmd-l     = "focus-right"
cmd-left  = "focus-left"
cmd-down  = "focus-down"
cmd-up    = "focus-up"
cmd-right = "focus-right"

cmd-shift-h     = "swap-left"
cmd-shift-j     = "swap-down"
cmd-shift-k     = "swap-up"
cmd-shift-l     = "swap-right"
cmd-shift-left  = "swap-left"
cmd-shift-down  = "swap-down"
cmd-shift-up    = "swap-up"
cmd-shift-right = "swap-right"

cmd-1 = "workspace-1"
cmd-f = "toggle-layout"
cmd-shift-r = "reload-config"
```

**Modifiers:** `cmd`, `alt`/`option`, `ctrl`/`control`, `shift` (join with `-`).

**Actions:** `focus-{left,right,up,down}`, `swap-{left,right,up,down}`,
`workspace-{1..9}`, `move-to-workspace-{1..9}`, `toggle-layout`, `reload-config`.

## CLI Usage

```
pengwm focus <left|right|up|down>
pengwm move-window <left|right|up|down>
pengwm split <horizontal|vertical>
pengwm workspace <id>
pengwm move-window-to-workspace <id>
pengwm close
pengwm toggle-layout
pengwm set-gap-outer <pixels>
pengwm set-gap-inner <pixels>
pengwm reload-config
pengwm state
```

## Development

```bash
# Run all tests (layout engine, workspace tree, IPC, FFI stubs)
cargo test

# Check lints
cargo clippy

# Run the daemon with verbose logs
RUST_LOG=debug ./target/release/pengwm-daemon
```

### Project Structure

```
pengwm-core/       Pure data types & layout engine (no macOS deps)
pengwm-daemon/     The background daemon (event loop, state, FFI)
pengwm-cli/        CLI client (clap parser, UDS sender)
```

See [docs/](docs/) for architecture, configuration, and command reference.
