# CLI Commands

All commands are run via `pengwm <subcommand> [args]`.

## Focus

```bash
pengwm focus <left|right|up|down>
```

Move keyboard focus to the nearest window in the given direction.
Wraps around at workspace boundaries.

## Move Window

```bash
pengwm move-window <left|right|up|down>
```

Swap the focused window with its neighbor in the given direction.

## Split

```bash
pengwm split <horizontal|vertical>
```

- If a **window** is focused: sets `pending_split` — the next window
  created will be placed in a new split with this direction.
- If a **split container** is focused: changes the container's split
  direction and flattens any resulting redundancy.

## Workspace

```bash
pengwm workspace <id>
```

Switch to workspace `id` (1-indexed, global config order — `1` is the first
`[[workspaces]]` entry, on any monitor). Switching to a workspace shown on
another monitor swaps it onto the focused monitor. Hidden workspaces park
their windows position-only: a 1×1 strip at their own monitor's bottom edge
by default (`hidden_strategy = "bottom_edge"`), fully offscreen with
`"far_offscreen"`.

## Move Window to Workspace

```bash
pengwm move-window-to-workspace <id>
```

Move the focused window to a different workspace. The window is removed
from the current workspace and inserted into the target. Same-monitor
moves respect `max_tiles` (overflow redirects to the next workspace with
room); moves across monitors always land.

## Focus Display

```bash
pengwm focus-display <left|right|up|down>
```

Move focus to the workspace shown on the monitor in the given direction
(default binds: `alt-ctrl-arrows`). Always succeeds — focusing an empty
workspace just moves the focus there, so the next switch or move resolves
on the newly focused monitor.

## Move Window to Display

```bash
pengwm move-window-to-display <left|right|up|down>
```

Throw the focused window onto the monitor in the given direction (default
binds: `alt-ctrl-shift-arrows`). Always lands on that monitor's visible
workspace, bypassing `max_tiles`; focus stays on the source monitor.

## Set Layout

Keybind-only actions (`set-layout-tile`, `set-layout-accordion`) that force
the focused workspace into tiling or accordion (monocle) mode directly,
instead of toggling. There is no CLI subcommand — use `toggle-layout` from
the CLI or bind the actions in config.toml.

## Toggle Bar

```bash
pengwm toggle-bar
```

Show or hide the status bar. A no-op when the bar process isn't running.

## Close

```bash
pengwm close
```

Close the focused window by sending an `AXCancel` action via the
Accessibility API.

## Toggle Layout

```bash
pengwm toggle-layout
```

Toggle the focused workspace between tiling mode and monocle (fullscreen)
mode.

## Select Layout

```bash
pengwm select-layout <even-horizontal|even-vertical|main-horizontal|main-vertical|tiled>
```

Rearrange the focused workspace into a tmux-style preset. The `main-*`
presets give the first window the `main-ratio` share (default `0.6`, settable
in the config). A preset clears monocle and re-equalizes shares.

## Resize Pane

```bash
pengwm resize-pane <left|right|up|down>
```

Push the divider one step (5%) in the given direction: the arrow-side edge
moves with the arrow. Inward presses grow the focused window; outward presses
at the screen edge shrink it. No pane drops below a 10% share.
Manual sizes survive windows opening and closing; only a preset re-equalizes.

## Set Gap Outer

```bash
pengwm set-gap-outer <pixels>
```

Set the outer gap (between windows and screen edge) in points.

## Set Gap Inner

```bash
pengwm set-gap-inner <pixels>
```

Set the inner gap (between adjacent windows) in points.

## Reload Config

```bash
pengwm reload-config
```

Re-read `~/.config/pengwm/config.toml` from disk and apply changes at runtime.

## State

```bash
pengwm state
```

Print the current daemon state — workspaces, window counts, and focused
windows — as JSON.

## Reveal All

```bash
pengwm reveal-all
```

Re-tile every hidden window back into its workspace. Daemon-down recovery:
if windows ever get stranded offscreen, this brings them back.

## Clear Session

```bash
pengwm clear-session
```

Delete the persisted session so the next launch starts fresh from
config.toml instead of restoring the last topology.

## Quit

```bash
pengwm quit
```

Shut down the daemon (and the status bar with it), persisting the session
first.
