# Changelog

## Unreleased

- **Typed settings no longer kill the keybind table.** The keybind
  scrape walks the whole config file; a typed value at the top level
  (`gap_outer = 5`, `restricted_apps = [...]`) hard-errored the parse
  and silently replaced every custom keybind with defaults behind a
  misleading warning. Non-string values are skipped now — a real
  keybind mistake (`alt-h = "bogus-action"`) still errors.

- **Placement memory: manual workspace moves survive a WM restart.**
  Every tileable window's home is captured at quit as a durable record —
  bundle + title → workspace *name* (WindowId and pid are deliberately
  not part of the identity: both die with the process / are app-level).
  Restore-time routing consults the record once per window, *before*
  config routing — the record is what the user did, the config list is
  what the app is — and a hit is consumed once so two identically-titled
  windows don't inherit each other's home. Misses (retitle, rename,
  unidentifiable window) fall through to config routing exactly as
  before, so nothing loses more than it keeps. The new `window_title`
  adapter seam backs the title half of the identity.

- **Restack fix: a member of a hidden workspace is offscreen — at every
  routing, not just the first switch.** The initial hide pass ran inside
  `new()` while every tree was still empty, so windows routed after it
  (bootstrap poll, sweep, new windows of routed apps, manual moves to a
  hidden workspace) kept whatever position macOS had them in — floating
  visibly over the active workspace until the next switch. Both add
  funnels now take the same hide placement a switch would give them
  (`hide_window_in_workspace`); reveal funnels are deliberately
  untouched (a deminimized window must stay where the user is looking).

- **Keybindings: no arrow keys, resize on alt-ctrl-shift-hjkl.** All
  default arrow-chord bindings are gone (`alt-arrows` focus,
  `alt-ctrl-arrows` focus-display, `alt-ctrl-shift-arrows`
  move-to-display, `alt-shift-arrows` resize): option-arrow is macOS text
  editing (word/line jump) and the WM must not steal it. Resizing is
  `alt-ctrl-shift-h/j/k/l`; focus-display moved to `alt-ctrl-h/j/k/l`;
  move-window-to-display now rides `alt-cmd-h/j/k/l`. New regression test
  asserts no arrow keycode is bound under any modifier combination.

- **Homebrew distribution dropped (deferred).** The tap-bump workflow
  (`.github/workflows/homebrew-bump.yml`) and the stale in-repo
  `Formula/pengwm.rb` are gone; the decision is recorded in ADR-0004
  because the bump automation had never once completed a cycle (the
  tap stayed at v0.6.0 through v0.6.1 and v0.6.2, which shipped via the
  curl installer). Installs are the curl one-liner / `./install.sh`,
  kept current by `pengwm update`. Pre-existing Homebrew Cellar copies
  are grandfathered: the one-copy policy still accepts them and they
  update in place; nothing existing breaks. Re-adding brew later means
  restoring the bump workflow, setting the PAT secret, and catching the
  formula up in one PR — the ADR records how.

## v0.6.2 — Ask-before-update & remembered skips (2026-10-12)

- **`pengwm update` now asks before installing.** Once the release check
  finds a strictly newer release it prints what it found and asks
  `Install PengWM vX now? [y/N]` — the default is No, so answering
  anything else (or closing stdin) keeps you on your current edition,
  reported as a success. The question is worth asking because every
  update costs one Accessibility re-grant (ADR-0001); scripts and the
  menubar's `Update now…` row pass `--yes` to skip the prompt — the
  menu click is itself the consent.
- **The release check is one module, not two drifting copies.** Fetch,
  tag parsing, and version comparison moved out of `pengwm-daemon/src/cli.rs`
  and `pengwm-menubar/src/update_check.rs` into `pengwm_core::release`,
  which answers `check(current)` in one shape: `UpdateAvailable`,
  `UpToDate`, or `Failed`. Both callers previously disagreed on the
  edge case of an equal-version-but-different-tag release (one reported
  an update, one didn't); the unified rule is *strictly newer numeric
  core only* — a re-published same-version release is never an update.

- **The menubar asks once per release, and remembers the answer.** Next
  to "Update now…" the menu now offers "Skip this version": one click
  records the declined tag in `$PENGWM_HOME/skip-update` and the rows
  vanish — across menubar restarts, because `$PENGWM_HOME` is the layout
  seam. When a different, newer release appears the ask re-arms. The
  watcher's first check reads the file, so a user who declined v0.7.0 is
  not re-nagged on every menu open. `pengwm update` never consults the
  skip: an explicit command always offers.
- **`pengwm_home()` moved to core.** `$PENGWM_HOME` (default `~/.pengwm`)
  is the layout seam the whole repo shares, so its resolution now lives
  in `pengwm_core::home` (the daemon's one-copy policy re-exports it);
  the menubar uses the same seam for the skip file instead of a private
  copy of the env logic.

## v0.6.1 — Untracked windows & AX hardening (2026-10-11)

- **A rogue `rm` of a socket path no longer ghost-locks the daemon's
  IPC.** Each server's accept loop (command socket, bar push socket)
  runs a per-tick socket-identity check: the bounded path's `(dev, ino)`
  is compared against the identity captured at bind time, and a missing
  or foreign file is logged loudly — including whether a live peer
  answers on it — and rebound, with the fresh listener swapped into the
  accept loop. Previously the listener survived on an unlinked vnode and
  every CLI connect hit `ECONNREFUSED` while the WM kept tiling (the
  ghost-daemon state); now CLI reachability recovers within one poll.
  The bind-first contract is unchanged: a live daemon still can never be
  stolen at startup. (`socket_guard.rs` owns the contract; `bind_first`
  is now one implementation shared by both servers.)

- **A closed display can no longer leave the tiling wedged.** All display
  topology healing collapsed into `DisplaySet::sync` — one pass that diffs
  the *live* display list against the workspace pool: dead seats leave the
  active map, orphaned workspaces remap to primary with fresh geometry, a
  newly live output is seated with the first hidden workspace, and every
  workspace on a live output self-heals its geometry. The three
  `Monitor{Added,Removed,Resized}` handlers are now hints that trigger it,
  and the 2s tick sweep runs it first — so a dropped or out-of-order
  hotplug event (an AirPlay tear-down emits no clean RemoveFlag) heals
  within one sweep instead of leaving stale geometry that the misplaced
  sweep re-asserted every 2s. Hidden workspaces on a healed display are
  re-parked at fresh coordinates; the layout-write cache drops its
  "already applied" evidence on any topology change (`clear_topology`,
  same reasoning as `clear_on_wake`). Session restore seats outputs
  through the same pass (`sync_with`): a session saved on one display no
  longer claims a boot-time output shows workspace 0 while workspace 0's
  geometry stays on the old display.

- **Fullscreen windows now leave the tiling tree.** A window entering
  native fullscreen is *parked*: it leaves its tree slot (siblings reflow
  around the freed space) while its workspace index is remembered — no
  more refused-geometry write storms (`kAXErrorFailure` retries / pin
  backoff against a window macOS owns). On unfullscreen it reveals back
  through the same funnel as an un-hidden window, retiled at its
  remembered workspace (overflowing to the next workspace with room when
  the home is full). Detection is a `AXFullScreen` attribute probe — no
  dedicated AX notification exists — on three paths: the
  focused-window-changed fast path, any refused/failed layout write (the
  probe answers "fullscreen or just zoomed?" for the pin backoff), and a
  per-tick exit probe of the parked set (≈1s worst-case unfullscreen
  latency). Parked-fullscreen windows are exempt from the listing-diff
  sweep (the app still lists its own fullscreen window) and the exit
  probe is suppressed during the wake blackout (a blacked-out AX reports
  "not fullscreen" wrongly — same "zero listing is no judgment" rule as
  the hidden reconcile). Hide-while-fullscreen upgrades the parking
  reason: the hidden flow owns the reveal afterwards. Popups (dialogs,
  restricted apps, `AXFloatingWindow`) are exempt from parking entirely.
  New config: `[windows] fullscreen = "park" | "tile"` (default `"park"`;
  `"tile"` is the pre-change behavior).
- **Store vocabulary: the hidden map is now a parking map.** `HiddenEntry`
  → `ParkedEntry { idx, popup, reason }` with `ParkReason::Hide |
  Fullscreen`; one parking map, one reveal funnel, two reasons. The
  `pending_for_reconcile` reveal set counts `Hide`-reason entries only —
  fullscreen exits are owned by the probe, never the hidden predicate.

## v0.6.0 — Bare binaries

- **Distribution: bare binaries, no `.app` bundle (ADR-0001).** PengWM
  installs to `$PENGWM_HOME/bin` (default `~/.pengwm/bin`) with `pengwm` /
  `pengwm-menubar` symlinks on PATH and the same launchd LaunchAgent; the
  v0.5.x `PengWM.app` layout is retired (install.sh migrates in place,
  removing old bundles). One artifact per arch (flat `.tar.gz` + `.sha256`);
  Homebrew moves to a real formula (cask tombstoned by the bump workflow);
  the one-copy policy (`location.rs`) is restated against `$PENGWM_HOME`
  plus Homebrew Cellar paths. Accepted, documented cost: releases stay
  ad-hoc signed, so **every update costs one Accessibility re-grant** —
  the same cost the ad-hoc bundle already paid per update. The bundle's
  LaunchServices/`lsregister` attribution gotcha and the Spotlight
  fake-app issue are gone with it. The menubar is unchanged and now
  locates `pengwm update` next to itself instead of a hardcoded bundle
  path.
- **Firefox Developer Edition now routes to Browsing by default.** It is a
  separate app (bundle id `org.mozilla.firefoxdeveloperedition`, display name
  "Firefox Developer Edition") and neither identifier was in the default
  Browsing assignment, so its windows fell through to the active workspace —
  typically Development after a daemon restart. Both identifiers joined the
  default list; a regression test pins either-match routes to Browsing.
- **Fix: closing a window no longer takes ~10s (or forever) to re-tile when its
  destroyed notification is missed.** Under a Wake-resync fix, the write-path
  gone grace was 10s everywhere — and since there was no active liveness
  probe, a closed window whose `kAXUIElementDestroyed` never arrived (1Password
  exiting, a Preview window opened from Finder, a stale ref failing CFEqual)
  stranded a dead tile member until some unrelated event triggered a layout,
  then paid the 10s on top. Untracking the dead window is now event-driven and
  fast: the 2s sweep already polls every running app, so each pid's listing is
  diffed against its tracked windows and any tracked window its own app no
  longer lists is untracked through the normal destroyed path (re-tile in the
  same pass). Two judgment rules keep the fast path safe: a pid whose poll
  returns **zero** windows is never judged (that is the post-wake blackout
  signature, not a mass close), and the whole untrack is suppressed while a
  wake resync is pending. The write-path grace itself became wake-scoped —
  250ms in normal operation, 10s only while the resync is pending — and a
  destroyed notification for an element we cannot map to a window id now
  probes the pid it reports instead of dropping a warning, so the
  CFEqual-mismatch failure mode untracks immediately too. Close-to-retile
  latency in every missed-notification case drops from "unbounded + 10s" to
  one sweep cycle (~2s).
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
- **Socket servers bind first, probe only on `Address already in use`.**
  `remove_file` + `bind` could never be race-safe: anything that unlinked a
  live daemon's socket file (a rival start racing the guard, an external
  cleanup) left the WM running on an unlinked vnode — healthy window
  management, every CLI call refused. Now `bind` on a live socket fails with
  `EADDRINUSE`, the probe distinguishes a live daemon (exit cleanly) from a
  stale file of a crashed one (safe to remove and rebind), and the single-
  instance guard remains as the early friendly exit.
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
