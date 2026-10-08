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
| `restricted_apps`      | list   | `[]`    | Bundle ids of apps whose windows always pop out as centered overlays — never tiled (see [Popups](#popups)) |
| `restore_last_session` | bool   | `true`  | Restore last session (workspace layout/focus) from `~/.local/share/pengwm/state.toml` on startup |
| `main_ratio`           | float  | `0.6`   | Share of the split the first window takes in the `main-horizontal` / `main-vertical` presets (clamped to 0.2–0.8) |
| `prefix`               | string | `"alt-space"` | Chord that arms the tmux-style prefix window, e.g. `"ctrl-b"` |
| `prefix_timeout_ms`    | int    | `1000`  | How long the prefix stays armed for follow-ups, in milliseconds |

```toml
gap_outer = 8
gap_inner = 4
max_tiles = 6
restricted_apps = ["com.whatever.floating-app"]
main_ratio = 0.65
prefix = "alt-space"
prefix_timeout_ms = 1000
```

## Workspaces

On startup the daemon creates one global set of named workspaces — five by
default (**Development**, **Browsing**, **Notes**, **Music**, **Messaging**) —
shared across all monitors, i3-style. Each workspace lives on exactly one
monitor at a time and each monitor shows exactly one workspace. Each entry
routes the windows of its listed apps into it, so your editor opens on the
Development workspace, Safari on Browsing, and so on.
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
daemon restart. Switching (`workspace-N`, global 1-based config order) to a
workspace shown on another monitor **swaps** it onto the focused monitor.
`focus-display` / `move-window-to-display` move focus and windows between
monitors; moves always land, even on full workspaces.

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

Workspaces with `monitor` set start on that display (an initial-output hint);
entries without `monitor` start on the primary. Workspaces move freely between
monitors afterwards via switching and display moves. Orphaned workspaces (saved
for a disconnected display) are remapped to the primary on restore — windows
are never dropped. `autostart` runs once per workspace regardless of the hint.

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

## Menubar

`pengwm-menubar` is a menu-bar icon spawned by the daemon. It lists every
workspace and the apps owning windows in it, with the active workspace marked;
clicking a workspace switches to it. It subscribes to the daemon's push socket
(state is refreshed each time the menu opens). The **Quit PengWM** menu item
stops everything: the daemon shuts down — and deregisters its LaunchAgent job,
so nothing respawns it — and the menubar exits. (`pengwm quit` does the same;
restart with a fresh login, `launchctl kickstart gui/$(id -u)/com.pengwm.daemon`,
or run `~/.pengwm/bin/pengwm` directly.)

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
| `popup_ratio`     | float  | `0.75`         | Share of the usable area a popup overlay takes (clamped to 0.1–1.0). See [Popups](#popups).    |

```toml
[windows]
hidden_strategy = "bottom_edge" # or "far_offscreen"
popup_ratio = 0.75
```

### Popups

PengWM never tiles popup windows — it tracks them on their workspace and
renders each as a centered overlay (same tmux-popup geometry as
`toggle-magnify`, sized by `popup_ratio`). A window becomes a popup when:

- its app is listed in `restricted_apps` (every newly discovered window of
  those apps pops out, never tiled — overlay-app bundles like launchers;
  a config reload doesn't re-route windows that are already tiled), or
- its macOS subrole is `AXDialog`, `AXSystemDialog`, or `AXFloatingWindow`
  (app dialogs, system dialogs, PiP / launcher / Chromium popups).

Sheets and unknown-subrole windows are left alone, as always. Popups are
placed once when they appear (and re-placed on wake) and can be dragged
freely afterwards — nothing snaps them back. They hide with their workspace
and return on top when you switch back. They never count against
`max_tiles` and never persist across restarts.

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
| `focus-display-left` / `focus-display-right` / `focus-display-up` / `focus-display-down` | Move focus to the monitor in direction (defaults: `alt-ctrl-arrows`) |
| `move-window-to-display-left` / `move-window-to-display-right` / `move-window-to-display-up` / `move-window-to-display-down` | Throw focused window to the monitor in direction (defaults: `alt-ctrl-shift-arrows`) |
| `workspace-1` .. `workspace-9`                               | Switch to workspace                           |
| `move-window-to-workspace-1` .. `move-window-to-workspace-9` | Move window to workspace                      |
| `split-horizontal` / `split-vertical`                        | Split the focused area                        |
| `close`                                                      | Close the focused window                      |
| `cycle-layout`                                               | Advance to the next layout preset             |
| `toggle-magnify`                                             | Toggle magnify popup on the focused window    |
| `select-layout-even-horizontal` / `select-layout-even-vertical` / `select-layout-main-horizontal` / `select-layout-main-vertical` / `select-layout-tiled` | Rearrange into a tmux-style preset |
| `resize-pane-left` / `resize-pane-right` / `resize-pane-up` / `resize-pane-down` | Push the divider one step in the direction |
| `set-gap-outer-{pixels}` / `set-gap-inner-{pixels}`          | Set gaps                                     |
| `reload-config`                                              | Reload configuration from disk                |
| `query-state`                                                | Dump workspace state to stdout                |
| `quit`                                                       | Shut down the daemon and the menubar          |
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
alt-t = "cycle-layout"
alt-m = "toggle-magnify"
cmd-shift-r = "reload-config"

# tmux-style presets and resizing
alt-shift-left  = "resize-pane-left"
alt-shift-right = "resize-pane-right"
alt-shift-up    = "resize-pane-up"
alt-shift-down  = "resize-pane-down"
alt-ctrl-1 = "select-layout-even-horizontal"
alt-ctrl-2 = "select-layout-even-vertical"
alt-ctrl-3 = "select-layout-main-horizontal"
alt-ctrl-4 = "select-layout-main-vertical"
alt-ctrl-5 = "select-layout-tiled"
```

### Prefix key

Like tmux's `ctrl-b`, the prefix (default `alt-space`, settable via the
top-level `prefix` key) arms a ~1s window (`prefix_timeout_ms`) where the full
action table above is reachable from short follow-ups — and it is purely
additive, so every direct bind keeps working.

- Hit `alt-space`, release, then type a bare key: it inherits the prefix
  modifiers, so `prefix, h` behaves like `alt-h` and `prefix, 1` like `alt-1`
  (workspace switching, tmux-style). Exact chords work too.
- Repeatable actions (resize, focus, presets, splits, display moves) extend
  the window, so `prefix, h, h, h` keeps resizing without re-arming.
  One-shot actions (`close`, `quit`, …) fire once and disarm.
- An unmatched key disarms and passes through, so normal typing is never
  swallowed beyond the armed window.

### Key Codes

Letter keys use their QWERTY keycodes. If no keybinding file exists,
a sensible set of defaults is used (see [Getting Started](getting-started.md)).
