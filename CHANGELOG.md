# Changelog

## v0.5.1 — PengWM.app

PengWM now ships as a proper macOS app bundle instead of loose binaries.

- **`PengWM.app`:** installed to `/Applications` (or `~/Applications` when
  not writable), shows in Launchpad/Spotlight, ad-hoc signed with the igloo
  penguin icon, `LSUIElement` (no Dock icon — the WM's UI surfaces are the
  bar and menubar). Bundle id `com.pengwm.daemon`, same as the LaunchAgent
  label.
- **install.sh revamp:** installs the bundle via `ditto` (preserves code
  signatures), symlinks the `pengwm` CLI into the prefix, and the
  LaunchAgent runs the bundle daemon directly. Releases older than the
  bundle fall back to the legacy flat layout; `--no-app` opts out, and
  `--app-dir` relocates the bundle. Uninstall script matches
  (`--app-dir`, removes shims).
- **Homebrew cask:** `brew tap penqguin/tap && brew install --cask pengwm`
  installs the bundle (`app` artifact + CLI `binary` shim). The legacy
  formula stays for the flat layout.
- **CI:** builds, signs and uploads `pengwm-app-<tag>-<target>.tar.gz` /
  `.zip` (+ sha256 sidecars) per arch, alongside the legacy flat artifacts.
- **Fix: `pengwm quit` stays quit.** The generated LaunchAgent used
  `KeepAlive = true`, so launchd instantly respawned the daemon after every
  clean exit — quitting looked like a no-op. The agent now uses
  `KeepAlive { SuccessfulExit = false }`: restart only on crash/non-zero
  exit, stay down after a clean `pengwm quit`.
- Layout tracking writes debounce 1s instead of 10s, so manual window
  moves settle faster.
- **CLI:** `pengwm --version`.

Also shipped in the v0.5.0 binaries (under-announced at the time):

- **Popups:** dialog, system-dialog and floating-panel windows (PiP,
  launcher panels, Chromium popups) are tracked on their workspace and
  rendered as centered overlays sized by `[windows] popup_ratio`
  (default 0.75). Placed once, freely draggable after; they hide with
  their workspace and are raised back on top on switch-back; they never
  count against `max_tiles` and never persist.
- **`restricted_apps` now works** (it was loaded but never consulted):
  every window of a listed bundle pops out as an overlay instead of
  tiling.

## v0.5 — i3-style multi-monitor

One global workspace pool shared across all monitors, replacing the old
per-monitor clone sets.

- **Global workspaces:** one tree per `[[workspaces]]` name (5 by default,
  not 5×N). Each workspace lives on exactly one monitor; each monitor
  shows exactly one workspace.
- **Swap-on-switch:** `workspace-N` (now global 1-based config order) pulls
  a workspace shown on another monitor onto the focused one, swapping the
  displaced workspace back.
- **Display moves:** `focus-display-*` always lands (bookkeeping-only when
  empty); `move-window-to-display-*` always lands, bypassing `max_tiles`;
  focus stays on move. Default binds `alt-ctrl-arrows` /
  `alt-ctrl-shift-arrows`.
- **`monitor` affinity is now an initial-output hint.** App routing lands
  in the global name-tree with no auto-pull; overflow spills to the next
  global workspace with room. Old sessions auto-migrate (first-name-wins);
  orphans restore onto primary.
- **Bar:** still primary-only, now with per-output active markers.
- **`autostart`** runs once per workspace regardless of hint.

Upgraders: after updating, `workspace-N` means something global
(`2` = Browsing everywhere). Clear a stale topology with
`pengwm clear-session`.
