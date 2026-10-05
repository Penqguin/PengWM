# Changelog

## Unreleased

- **Quitting the menubar now quits the whole app — and stays quit.** The
  Quit item (renamed **Quit PengWM**) sends `Command::Quit` and terminates;
  the daemon deregisters its LaunchAgent job (`launchctl bootout`) during
  clean shutdown, because on current macOS `KeepAlive { SuccessfulExit =
  false }` still respawns the job after a clean exit 0 — quit looked like a
  no-op with the daemon reappearing seconds later. A failed Quit IPC (daemon
  wedged or gone) escalates instead of leaving a survivor icon: bootout the
  job, `killall -TERM pengwm`, then `killall -KILL` after a short grace.
  The menubar's IPC client also bounds its wait for the daemon's reply (3s
  read timeout) so a stalled daemon cannot freeze the Quit flow on the main
  thread. Restart afterwards with `open /Applications/PengWM.app` (or any
  login); crashes still auto-respawn via the agent.
- **Fix: a second daemon can no longer silently steal the IPC socket.**
  Both UDS servers used to `remove_file` + `bind` unconditionally, so a
  daemon opened twice (Spotlight double-click on a stray copy, manual run
  beside the LaunchAgent) rebound the sockets under the first one: two WMs
  tiled at once and `pengwm` CLI commands landed on the wrong generation —
  the "it has access but never works" failure. The daemon now probes the
  command socket before binding (probe-then-bind) and exits cleanly with a
  "already running" message when a live daemon answers; a stale socket file
  from a crashed daemon still rebinds fine. The menubar holds an instance
  lock so an orphan icon can't stack next to the respawned one.
- **Fix: Accessibility grant could never apply to the launchd daemon after
  an update.** Replacing `PengWM.app` on disk orphans the bundle's
  LaunchServices registration, and TCC cannot attribute an unregistered
  bundle — `AXIsProcessTrusted()` failed under launchd while the same
  binary launched by hand was trusted (System Settings showed the grant;
  the daemon disagreed). `install.sh` now runs `lsregister -f` on the
  installed bundle; ad-hoc re-signs per update still require one fresh
  grant click, but after that the grant actually sticks.
- **Spotlight hygiene:** the repo's committed Info.plist template moved from
  `packaging/PengWM.app` to `packaging/app-template` (Spotlight indexes any
  `.app`-shaped folder it finds — a template with no executables surfaced as
  a fake third PengWM in searches). Opening a stray copy is now harmless
  anyway: the single-instance guard above makes it exit immediately.
- **Fix: windows stopped being tiled after any transient AX miss.** v0.5.1
  shrank the layout writer's `GONE_GRACE` from 10s to 1s, mislabeled as a
  "layout tracking delay". `GONE_GRACE` is the window's *death* grace — how
  long a window stays tracked after the OS stops listing it. Post-wake AX
  blackouts and busy Chromium/Gecko apps answer nothing for seconds, so with
  a 1s grace a live window missed once at the wrong moment got untracked
  permanently: it silently fell out of tiling. Restored to 10s (real closes
  never wait on it — they arrive via the destroyed notification).
- **The egui status bar (`pengwm-bar`) is removed**; the menu-bar icon
  (`pengwm-menubar`) is the only UI surface. Dropped: the `pengwm-bar` crate,
  the `[bar]` config table, `BarReserve` reservation machinery, the
  `toggle-bar` command/bind (`alt-b`), and the `pengwm toggle-bar` CLI
  subcommand. The daemon now excludes only the menubar child, `pengwm quit`
  sends an explicit `Exit` to the menubar (it terminates immediately instead
  of lingering 10s), and the daemon→menubar push channel is unchanged.

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
