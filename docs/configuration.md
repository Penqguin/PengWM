# Configuration

PengWM looks for `~/.config/pengwm/config.toml` (or
`$XDG_CONFIG_HOME/pengwm/config.toml`). The file is watched at runtime — changes
apply on save (or via `pengwm reload-config`).

## Settings

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `gap_outer` | int | `10` | Pixels between windows and screen edge |
| `gap_inner` | int | `5` | Pixels between adjacent windows |
| `max_tiles` | int | `4` | Max windows per workspace before overflow |
| `mod_key` | string | `"cmd"` | Primary modifier for keybindings |

```toml
gap_outer = 8
gap_inner = 4
max_tiles = 6
mod_key = "cmd"
```

## Keybindings

Keybindings are defined in the same config file using
`modifier-key = "action"` syntax.

### Modifiers

| Token | Key |
|-------|-----|
| `cmd` / `command` | Command (⌘) |
| `alt` / `option` | Option (⌥) |
| `ctrl` / `control` | Control (⌃) |
| `shift` | Shift (⇧) |

Join modifiers with `-`, e.g. `cmd-shift`, `cmd-alt-ctrl`.

### Actions

| Action | Description |
|--------|-------------|
| `focus-left` / `focus-right` / `focus-up` / `focus-down` | Move focus in direction |
| `swap-left` / `swap-right` / `swap-up` / `swap-down` | Move focused window in direction |
| `workspace-1` .. `workspace-9` | Switch to workspace |
| `move-to-workspace-1` .. `move-to-workspace-9` | Move window to workspace |
| `toggle-layout` | Toggle between tiling and monocle |
| `reload-config` | Reload configuration from disk |

### Example

```toml
# Vim-style focus
cmd-h = "focus-left"
cmd-j = "focus-down"
cmd-k = "focus-up"
cmd-l = "focus-right"

# Arrow key focus
cmd-left  = "focus-left"
cmd-right = "focus-right"
cmd-up    = "focus-up"
cmd-down  = "focus-down"

# Window movement
cmd-shift-h = "swap-left"
cmd-shift-j = "swap-down"
cmd-shift-k = "swap-up"
cmd-shift-l = "swap-right"

# Workspaces
cmd-1 = "workspace-1"
cmd-2 = "workspace-2"
cmd-3 = "workspace-3"

# Move to workspace
cmd-shift-1 = "move-to-workspace-1"
cmd-shift-2 = "move-to-workspace-2"

# Layout
cmd-f = "toggle-layout"
cmd-shift-r = "reload-config"
```

### Key Codes

Letter keys use their QWERTY keycodes. If no keybinding file exists,
a sensible set of defaults is used (see [Getting Started](getting-started.md)).
