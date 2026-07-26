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

Switch to workspace `id` (1-indexed). Workspaces are emulated by moving
windows off-screen (x: -32000) when hidden.

## Move Window to Workspace

```bash
pengwm move-window-to-workspace <id>
```

Move the focused window to a different workspace. The window is removed
from the current workspace and inserted into the target.

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
