# PengWM 🐧

A tiling window manager for macOS built in Rust. No System Integrity Protection (SIP) disabling
required — uses only public Apple APIs (Accessibility & Core Graphics).

## Quick Start

```bash
# Install the latest signed release (stable Accessibility grant across updates):
#   /Applications/PengWM.app (appears in Launchpad) + a `pengwm` CLI on PATH
curl -fsSL https://pengwm.penqguin.com/install.sh | bash

# Grant Accessibility permissions when prompted, then control it
pengwm focus left
pengwm workspace 3
pengwm close
```

Prefer a checkout (or already have the repo cloned):

```bash
git clone https://github.com/Penqguin/PengWM
cd PengWM
./install.sh
```

Uninstall later with:

```bash
curl -fsSL https://pengwm.penqguin.com/uninstall.sh | bash -s -- --yes
```

From source instead (requires Rust; ad-hoc signed, so macOS re-prompts
for Accessibility after every rebuild):

```bash
./install.sh --from-source
# or manually:
cargo build --release
./target/release/pengwm
```

## Prerequisites

1. **macOS 14+** (Ventura should work, Sequoia tested)
2. **Accessibility permissions:** System Settings → Privacy & Security → Accessibility → add
   PengWM (the app, when installed via the bundle; or your terminal / the `pengwm`
   binary directly for source builds)
3. **Displays have separate Spaces:** System Settings → Desktop & Dock → turn on
   _Displays have separate Spaces_

## Configuration

PengWM looks for `~/.config/pengwm/config.toml` (or `$XDG_CONFIG_HOME/pengwm/config.toml`).
If no file exists, defaults are used and a config watcher reloads changes at runtime.

```toml
gap_outer = 10
gap_inner = 5
max_tiles = 4
restricted_apps = []
prefix = "alt-space"
prefix_timeout_ms = 1000

[menubar]
enabled = true
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

alt-shift-h     = "move-window-left"
alt-shift-j     = "move-window-down"
alt-shift-k     = "move-window-up"
alt-shift-l     = "move-window-right"
alt-shift-left  = "move-window-left"
alt-shift-down  = "move-window-down"
alt-shift-up    = "move-window-up"
alt-shift-right = "move-window-right"

alt-1 = "workspace-1"
alt-shift-1 = "move-window-to-workspace-1"
alt-ctrl-left  = "focus-display-left"
alt-ctrl-shift-right = "move-window-to-display-right"
alt-t = "cycle-layout"
alt-m = "toggle-magnify"
cmd-shift-r = "reload-config"
```

**Modifiers:** `cmd`, `alt`/`option`, `ctrl`/`control`, `shift` (join with `-`).

**Actions:** `focus-{left,right,up,down}`, `move-window-{left,right,up,down}`,
`workspace-{id}`, `move-window-to-workspace-{id}`, `focus-display-{left,right,up,down}`,
`move-window-to-display-{left,right,up,down}`, `split-horizontal`, `split-vertical`,
`close`, `cycle-layout`, `toggle-magnify`, `select-layout-{even-horizontal,even-vertical,main-horizontal,main-vertical,tiled}`,
`resize-pane-{left,right,up,down}`, `set-gap-outer-{pixels}`,
`set-gap-inner-{pixels}`, `reload-config`, `query-state`,
`reveal-all`, `quit`.

## Workspaces

On startup PengWM creates one global set of five named workspaces —
**Development**, **Browsing**, **Notes**, **Music**, **Messaging** — shared
across all monitors, i3-style. Each workspace lives on exactly one monitor at
a time and each monitor shows exactly one workspace; switching to a workspace
shown on another monitor swaps it onto the focused monitor. Each routes the
windows of its configured apps into it (match by bundle id or app name,
case-insensitive), so apps land in the workspace you use them in. Override or
replace them with `[[workspaces]]` tables in config.toml; see
[docs/configuration.md](docs/configuration.md).

## Menubar

`pengwm-menubar` is a menu-bar icon (spawned automatically by the daemon) that
lists every workspace and the apps with windows in it. Clicking a workspace
switches to it. Enable/disable with `[menubar] enabled = true|false` in
config.toml. It rebuilds its menu from the latest pushed state each time it
opens. Choosing **Quit PengWM** shuts the whole app down — the daemon exits
and deregisters its LaunchAgent (`quit` stays `quit`), and the menubar
terminates; if the daemon is unreachable the menubar kills it directly.

## CLI Usage

```
pengwm focus <left|right|up|down>
pengwm move-window <left|right|up|down>
pengwm split <horizontal|vertical>
pengwm workspace <id>
pengwm move-window-to-workspace <id>
pengwm focus-display <left|right|up|down>
pengwm move-window-to-display <left|right|up|down>
pengwm close
pengwm cycle-layout
pengwm toggle-magnify
pengwm select-layout <even-horizontal|even-vertical|main-horizontal|main-vertical|tiled>
pengwm resize-pane <left|right|up|down>
pengwm set-gap-outer <pixels>
pengwm set-gap-inner <pixels>
pengwm reload-config
pengwm state
pengwm reveal-all
pengwm clear-session
pengwm quit
```

## Project Structure

```
pengwm-core/       Pure data types, layout engine, workspace logic (no macOS deps)
pengwm-daemon/     The `pengwm` binary — daemon, CLI client, macOS FFI
pengwm-menubar/    The `pengwm-menubar` menu-bar icon — workspace/app list
```

See [docs/](docs/) for full architecture, configuration, and command reference.
Stuck? Start with [docs/troubleshooting.md](docs/troubleshooting.md). Upgrading
past v0.5? See [CHANGELOG.md](CHANGELOG.md).

## Contributing

### Testing

Run the full test suite (works on any platform — no macOS FFI required):

```bash
cargo test
```

This runs the full suite covering:

- **Layout engine:** window placement, gaps, ratios, nested splits, magnify
- **Workspace tree:** add/remove/focus/swap windows, split direction alternation
- **DisplaySet:** global workspace pool, swap-on-switch, output moves, monitor add/remove
- **StateManager:** command dispatch, event handling, workspace switching
- **Layout-write policy:** skip-if-unchanged, gone grace, pinned-write backoff
- **Wake resync:** deferred AX probing, deadline commit
- **Keybind parsing:** modifier combinations, action names, TOML parsing
- **IPC round-trip:** UDS send/receive/response (macOS only, no AX required)

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
./target/debug/pengwm cycle-layout
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

- **Workspace::layout()** — produces global-coordinate window rects
  from the tree. Tree internals (`NodeId`, `Arena`) are private.
- **OsAdapter trait** — the seam between state logic and macOS FFI.
  Two implementations: `MacOsAdapter` (real) and `TestAdapter` (mock).
- **WindowElementCache** — O(1) `WindowId → AXUIElementRef` lookup inside
  `MacOsAdapter`, populated by AX observer callbacks.

See [docs/architecture.md](docs/architecture.md) for the full design.
