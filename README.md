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
./target/release/pengwm

# In another terminal, control it
./target/release/pengwm focus left
./target/release/pengwm split horizontal
./target/release/pengwm workspace 3
./target/release/pengwm close
```

## Prerequisites

1. **macOS 14+** (Ventura should work, Sequoia tested)
2. **Accessibility permissions:** System Settings → Privacy & Security → Accessibility → add
   your terminal (or the `pengwm` binary directly after code-signing)
3. **Displays have separate Spaces:** System Settings → Desktop & Dock → turn on
   _Displays have separate Spaces_

## Configuration

PengWM looks for `~/.config/pengwm/config.toml` (or `$XDG_CONFIG_HOME/pengwm/config.toml`).
If no file exists, defaults are used and a config watcher reloads changes at runtime.

```toml
gap_outer = 10
gap_inner = 5
mod_key = "cmd"
restricted_apps = []

[bar]
position = "top"
thickness = 32
visible = true
enabled = true
theme = "tokyo-night"
```

### Keybindings

Keybindings are defined in the same file with `modifier-key = "action"` entries:

```toml
alt-h     = "focus-left"
alt-j     = "focus-down"
alt-k     = "focus-up"
alt-l     = "focus-right"
alt-left  = "focus-left"
alt-down  = "focus-down"
alt-up    = "focus-up"
alt-right = "focus-right"

alt-shift-h     = "swap-left"
alt-shift-j     = "swap-down"
alt-shift-k     = "swap-up"
alt-shift-l     = "swap-right"
alt-shift-left  = "swap-left"
alt-shift-down  = "swap-down"
alt-shift-up    = "swap-up"
alt-shift-right = "swap-right"

alt-1 = "workspace-1"
alt-shift-1 = "move-to-workspace-1"
alt-/ = "layout-tile"
alt-, = "layout-accordion"
alt-b = "toggle-bar"
cmd-shift-r = "reload-config"
```

**Modifiers:** `cmd`, `alt`/`option`, `ctrl`/`control`, `shift` (join with `-`).

**Actions:** `focus-{left,right,up,down}`, `swap-{left,right,up,down}`,
`workspace-{1..9}`, `move-to-workspace-{1..9}`, `layout-tile`, `layout-accordion`,
`toggle-layout`, `toggle-bar`, `reload-config`.

## Status Bar

PengWM ships a minimal status bar (`pengwm-bar`, built on egui/eframe) that the
daemon spawns automatically. It shows a split-direction icon and clickable
workspace pills on the primary display, themed tokyo-night by default. See
[docs/configuration.md](docs/configuration.md) for the `[bar]` table, themes,
and corner-radius options.

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

## Project Structure

```
pengwm-core/       Pure data types, layout engine, workspace logic (no macOS deps)
pengwm-daemon/     The `pengwm` binary — daemon, CLI client, macOS FFI
pengwm-bar/        The `pengwm-bar` status bar — egui/eframe frontend
```

See [docs/](docs/) for full architecture, configuration, and command reference.

## Contributing

### Testing

Run the full test suite (works on any platform — no macOS FFI required):

```bash
cargo test
```

This runs ~90+ tests covering:

- **Layout engine:** window placement, gaps, ratios, nested splits, monocle
- **Workspace tree:** add/remove/focus/swap windows, split direction alternation
- **StateManager:** command dispatch, event handling, workspace switching
- **Keybind parsing:** modifier combinations, action names, TOML parsing
- **IPC round-trip:** UDS send/receive/response (macOS only, no AX required)

For macOS-specific integration tests (requires AX permissions):

```bash
cargo test -- --include-ignored
```

These test the real Accessibility API: window rect get/set, observer attach/detach,
and end-to-end window-created flow. They're `#[ignore]`d by default.

### Visual / Interactive Testing

For visual testing against a real display, build and run the daemon with debug logging:

```bash
cargo build && RUST_LOG=debug ./target/debug/pengwm
```

In another terminal, send commands through the CLI to see windows
rearrange in real time:

```bash
./target/debug/pengwm focus right
./target/debug/pengwm split horizontal
./target/debug/pengwm toggle-layout
```

To monitor state without visual side effects:

```bash
./target/debug/pengwm state | jq
```

### Code Quality

```bash
cargo clippy        # Lint checks
cargo fmt           # Formatting
cargo test          # All unit tests
```

### Architecture Notes

The project uses a **pure/dirty split** — `pengwm-core` is pure Rust with no
macOS dependencies and runs `cargo test` on any platform. `pengwm-daemon`
holds all macOS FFI and ships as the single `pengwm` binary: run it with no
arguments to start the daemon, or pass a subcommand to control a running
daemon. Key abstractions:

- **Workspace::layout() / hide()** — produce global-coordinate window rects
  from the tree. Tree internals (`NodeId`, `Arena`) are private.
- **OsAdapter trait** — the seam between state logic and macOS FFI.
  Two implementations: `MacOsAdapter` (real) and `TestAdapter` (mock).
- **WindowElementCache** — O(1) `WindowId → AXUIElementRef` lookup inside
  `MacOsAdapter`, populated by AX observer callbacks.

See [docs/architecture.md](docs/architecture.md) for the full design.
