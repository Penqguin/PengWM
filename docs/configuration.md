# Configuration

PengWM looks for `~/.config/pengwm/config.toml` (or
`$XDG_CONFIG_HOME/pengwm/config.toml`). The file is watched at runtime — changes
apply on save (or via `pengwm reload-config`).

## Settings

| Key                    | Type   | Default | Description                               |
| ---------------------- | ------ | ------- | ----------------------------------------- |
| `gap_outer`            | int    | `10`    | Pixels between windows and screen edge    |
| `gap_inner`            | int    | `5`     | Pixels between adjacent windows           |
| `max_tiles`            | int    | `4`     | Max windows per workspace; overflow goes to the next workspace with room |
| `restricted_apps`      | list   | `[]`    | Bundle ids of apps that PengWM must not manage |
| `restore_last_session` | bool   | `true`  | Restore last session (workspace layout/focus) from `~/.local/share/pengwm/state.toml` on startup |

```toml
gap_outer = 8
gap_inner = 4
max_tiles = 6
restricted_apps = ["com.whatever.floating-app"]
```

## Workspaces

On startup the daemon creates a set of named workspaces on every monitor. You
get five by default — **Development**, **Browsing**, **Notes**, **Music**,
**Messaging** — and each one routes the windows of its listed apps into it, so
your editor opens on the Development workspace, Safari on Browsing, and so on.
`apps` entries match an app's bundle id or display name (case-insensitively);
windows from unlisted apps go to the currently focused workspace.

| Key         | Type   | Description                                                  |
| ----------- | ------ | ------------------------------------------------------------ |
| `name`      | string | Workspace name (shown in the bar and menubar)                |
| `apps`      | list   | Bundle ids / app names whose windows route to this workspace |
| `monitor`   | int/string | Optional display affinity (`1` or `"Display Name"`); `None` clones to every monitor |
| `autostart` | list   | Shell commands to run once when this workspace is created (not on session restore) |

The list replaces the defaults entirely — define your own five (or three, or
twelve). Workspaces are created at startup, so changing the list requires a
daemon restart.

```toml
[[workspaces]]
name = "Development"
apps = ["com.apple.dt.Xcode", "com.googlecode.iterm2", "iTerm2", "Code"]
monitor = 1
autostart = ["ghostty"]

[[workspaces]]
name = "Browsing"
apps = ["com.apple.Safari", "com.google.Chrome", "Chrome", "Firefox"]

[[workspaces]]
name = "Notes"
apps = ["com.apple.Notes", "md.obsidian", "Obsidian"]

[[workspaces]]
name = "Music"
apps = ["com.apple.Music", "com.spotify.client", "Spotify"]

[[workspaces]]
name = "Messaging"
apps = ["com.apple.MobileSMS", "com.hnc.Discord", "Slack", "WhatsApp"]
```

Workspaces with `monitor` set only appear on that display; entries without `monitor`
are cloned to every monitor (so `monitor = 1` on a 2-display setup gives 1+5*1=6
workspaces). Orphaned workspaces (saved for a disconnected display) are
remapped to the primary on restore — windows are never dropped.

### Session

On `pengwm quit` (or SIGTERM/SIGINT) the daemon atomically saves the session to
`~/.local/share/pengwm/state.toml` (`$XDG_STATE_HOME/pengwm/state.toml` if set):
active workspace per monitor, workspace names/monitors, gaps, and the split
skeleton (windows themselves are ephemeral and re-routed on next launch).

- `restore_last_session = true` (default) restores that file on next launch.
- `restore_last_session = false` always starts from `config.toml`.
- A corrupt/missing session falls back to defaults with a warning.
- `pengwm clear-session` deletes the saved state so the next launch is fresh.
- `autostart` does **not** run when restoring a session.

## Bar

PengWM ships a minimal status bar, `pengwm-bar` (an egui/eframe process spawned
by the daemon). It shows a split-direction icon plus clickable workspace pills
on the primary display. It is excluded from tiling by pid. The strip on the
screen edge is only reserved while the bar process is actually running, so a
disabled or failed bar never leaves a phantom gap.

| Key             | Type    | Default         | Description                                        |
| --------------- | ------- | --------------- | -------------------------------------------------- |
| `position`      | string  | `"top"`         | `"top"`, `"bottom"`, `"left"`, or `"right"`        |
| `thickness`     | int     | `32`            | Bar width (left/right) or height (top/bottom) in px |
| `visible`       | bool    | `true`          | Initial visibility (only effective when `enabled`; toggle at runtime with `alt-b`) |
| `enabled`       | bool    | `false`         | Whether the daemon spawns the bar at startup       |
| `theme`         | string  | `"tokyo-night"` | Built-in theme name or path to a theme TOML        |
| `corner_radius` | float   | *(auto)*        | Explicit corner radius override, in points         |
| `colors`        | table   | *(none)*        | Per-color overrides on top of the resolved theme   |

```toml
[bar]
enabled = true
position = "top"
thickness = 32
visible = true
theme = "tokyo-night"

[bar.colors]
background = "#1a1b26"
accent = "#7aa2f7"
```

## Menubar

`pengwm-menubar` is a menu-bar icon spawned by the daemon. It lists every
workspace and the apps owning windows in it, with the active workspace marked;
clicking a workspace switches to it. Like the bar, it subscribes to the daemon's
push socket (state is refreshed each time the menu opens). The **Quit PengWM
Menubar** menu item stops everything: the daemon shuts down, the status bar
closes, and the menubar exits. (`pengwm quit` does the same.)

| Key       | Type | Default | Description                              |
| --------- | ---- | ------- | ---------------------------------------- |
| `enabled` | bool | `true`  | Whether the daemon spawns the menubar    |

```toml
[menubar]
enabled = true
```

## Windows

Visibility and lifecycle settings scoped under `[windows]`.

| Key               | Type   | Default        | Description                                                                                   |
| ----------------- | ------ | -------------- | --------------------------------------------------------------------------------------------- |
| `hidden_strategy` | string | `"bottom_edge"` | Where inactive-workspace windows are parked: `"bottom_edge"` (1×1 at bottom-right, dark clamped strip visible in Mission Control as daemon-down escape hatch) or `"far_offscreen"` (legacy `-100k`, fully invisible) |

```toml
[windows]
hidden_strategy = "bottom_edge" # or "far_offscreen"
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

| Action                                                       | Description                                   |
| ------------------------------------------------------------ | --------------------------------------------- |
| `focus-left` / `focus-right` / `focus-up` / `focus-down`     | Move focus in direction                       |
| `move-window-left` / `move-window-right` / `move-window-up` / `move-window-down` | Move focused window into the neighbor's space |
| `workspace-1` .. `workspace-9`                               | Switch to workspace                           |
| `move-window-to-workspace-1` .. `move-window-to-workspace-9` | Move window to workspace                      |
| `split-horizontal` / `split-vertical`                        | Split the focused area                        |
| `close`                                                      | Close the focused window                      |
| `set-layout-tile`                                            | Switch to tiling layout                       |
| `set-layout-accordion`                                       | Switch to accordion layout (focused window fills the screen) |
| `set-gap-outer-{pixels}` / `set-gap-inner-{pixels}`          | Set gaps                                     |
| `toggle-layout`                                              | Toggle between tiling and monocle             |
| `toggle-bar`                                                 | Show/hide the bar                             |
| `reload-config`                                              | Reload configuration from disk                |
| `query-state`                                                | Dump workspace state to stdout                |
| `quit`                                                       | Shut down the daemon (and the bar)            |
| `reveal-all`                                                 | Re-tile all hidden windows into their workspaces (daemon-down recovery) |

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
alt-shift-h = "move-window-left"
alt-shift-j = "move-window-down"
alt-shift-k = "move-window-up"
alt-shift-l = "move-window-right"

# Workspaces
alt-1 = "workspace-1"
alt-2 = "workspace-2"
alt-3 = "workspace-3"

# Move to workspace
alt-shift-1 = "move-window-to-workspace-1"
alt-shift-2 = "move-window-to-workspace-2"

# Layout
alt-/ = "set-layout-tile"
alt-, = "set-layout-accordion"
cmd-shift-r = "reload-config"
alt-b = "toggle-bar"
```

### Key Codes

Letter keys use their QWERTY keycodes. If no keybinding file exists,
a sensible set of defaults is used (see [Getting Started](getting-started.md)).
