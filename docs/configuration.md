# Configuration

PengWM looks for `~/.config/pengwm/config.toml` (or
`$XDG_CONFIG_HOME/pengwm/config.toml`). The file is watched at runtime — changes
apply on save (or via `pengwm reload-config`).

## Settings

| Key              | Type   | Default | Description                               |
| ---------------- | ------ | ------- | ----------------------------------------- |
| `gap_outer`      | int    | `10`    | Pixels between windows and screen edge    |
| `gap_inner`      | int    | `5`     | Pixels between adjacent windows           |
| `max_tiles`      | int    | `4`     | Max windows per workspace; overflow goes to the next workspace with room |
| `mod_key`        | string | `"cmd"` | Primary modifier for keybindings          |
| `restricted_apps`| list   | `[]`    | Bundle ids of apps that PengWM must not manage |

```toml
gap_outer = 8
gap_inner = 4
max_tiles = 6
mod_key = "cmd"
restricted_apps = ["com.whatever.floating-app"]
```

## Bar

PengWM ships a minimal status bar, `pengwm-bar` (an egui/eframe process spawned
by the daemon). It shows a split-direction icon plus clickable workspace pills
on the primary display. It is excluded from tiling by pid.

| Key             | Type    | Default         | Description                                        |
| --------------- | ------- | --------------- | -------------------------------------------------- |
| `position`      | string  | `"top"`         | `"top"`, `"bottom"`, `"left"`, or `"right"`        |
| `thickness`     | int     | `32`            | Bar width (left/right) or height (top/bottom) in px |
| `visible`       | bool    | `true`          | Initial visibility (toggle at runtime with `alt-b`) |
| `enabled`       | bool    | `true`          | Whether the daemon spawns the bar at startup       |
| `theme`         | string  | `"tokyo-night"` | Built-in theme name or path to a theme TOML        |
| `corner_radius` | float   | *(auto)*        | Explicit corner radius override, in points         |
| `colors`        | table   | *(none)*        | Per-color overrides on top of the resolved theme   |

```toml
[bar]
position = "top"
thickness = 32
visible = true
enabled = true
theme = "tokyo-night"

[bar.colors]
background = "#1a1b26"
accent = "#7aa2f7"
```

### Corner radius

`corner_radius` defaults to the corner radius of the current macOS version so
the bar matches the system chrome:

| macOS version | Default radius |
| ------------- | -------------- |
| 11–15         | `10` pt        |
| Tahoe (26)    | `26` pt        |
| Golden Gate (27) | `20` pt     |

### Themes

Built-in themes: `tokyo-night`, `catppuccin-mocha`, `catppuccin-latte`,
`nord`, `dracula`, `one-dark`, `solarized-dark`, `solarized-light`,
`gruvbox-dark`, `gruvbox-light`, `rose-pine`, `kanagawa`.

Custom themes are TOML files. Drop one in `~/.config/pengwm/themes/` and
reference it by filename (e.g. `theme = "my-theme"` reads
`~/.config/pengwm/themes/my-theme.toml`), or point `theme` at an absolute path.

```toml
background = "#1a1b26"
foreground = "#c0caf5"
accent = "#7aa2f7"
inactive = "#3b4261"
border = "#565f89"
font_size = 12.0
```

Known limitations: the bar renders on the primary display only, is not visible
over fullscreen apps, and enabling it at runtime requires a daemon restart.

## Keybindings

Keybindings are defined in the same config file using
`modifier-key = "action"` syntax.

### Modifiers

| Token              | Key         |
| ------------------ | ----------- |
| `cmd` / `command`  | Command (⌘) |
| `alt` / `option`   | Option (⌥)  |
| `ctrl` / `control` | Control (⌃) |
| `shift`            | Shift (⇧)   |

Join modifiers with `-`, e.g. `cmd-shift`, `cmd-alt-ctrl`.

### Actions

| Action                                                   | Description                                   |
| -------------------------------------------------------- | --------------------------------------------- |
| `focus-left` / `focus-right` / `focus-up` / `focus-down` | Move focus in direction                       |
| `swap-left` / `swap-right` / `swap-up` / `swap-down`     | Move focused window into the neighbor's space |
| `workspace-1` .. `workspace-9`                           | Switch to workspace                           |
| `move-to-workspace-1` .. `move-to-workspace-9`           | Move window to workspace                      |
| `layout-tile`                                            | Switch to tiling layout                       |
| `layout-accordion`                                       | Switch to accordion layout (focused window fills the screen) |
| `toggle-layout`                                          | Toggle between tiling and monocle             |
| `toggle-bar`                                             | Show/hide the bar                             |
| `reload-config`                                          | Reload configuration from disk                |

### Example

```toml
# Vim-style focus
alt-h = "focus-left"
alt-j = "focus-down"
alt-k = "focus-up"
alt-l = "focus-right"

# Arrow key focus
alt-left  = "focus-left"
alt-right = "focus-right"
alt-up    = "focus-up"
alt-down  = "focus-down"

# Window movement
alt-shift-h = "swap-left"
alt-shift-j = "swap-down"
alt-shift-k = "swap-up"
alt-shift-l = "swap-right"

# Workspaces
alt-1 = "workspace-1"
alt-2 = "workspace-2"
alt-3 = "workspace-3"

# Move to workspace
alt-shift-1 = "move-to-workspace-1"
alt-shift-2 = "move-to-workspace-2"

# Layout
alt-/ = "layout-tile"
alt-, = "layout-accordion"
cmd-shift-r = "reload-config"
alt-b = "toggle-bar"
```

### Key Codes

Letter keys use their QWERTY keycodes. If no keybinding file exists,
a sensible set of defaults is used (see [Getting Started](getting-started.md)).
