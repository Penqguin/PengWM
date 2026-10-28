# Changelog

## v0.6.9 — New windows open where you are (2026-10-28)

- **New windows open on the current workspace, not their remembered one.**
  Routing a birth no longer consults remembered placement — neither last
  session's (bundle, title → workspace) memory nor the successor ask that let
  an app relaunch inherit its dead window's home. The precedence is now:
  seatbelt (same-window rediscovery only) → config routing (`apps = [...]`)
  → the active workspace. Session restore is untouched (topology, active
  workspaces, gaps), and placement records are still projected into
  `state.toml` — but as diagnostic data nothing consumes (ADR-0005).

## v0.6.8 — Memory and the tick's reads (2026-10-23)

- **Fixed: unbounded memory growth at idle.** The daemon never drained
  an autorelease pool: its main thread runs a bare `CFRunLoop`, so every
  AppKit/Foundation call's autoreleased temporaries stayed reachable
  forever — `leaks` reported almost nothing while RSS climbed ~2.7 MB/min.
  The dominant source, measured on a live daemon: every
  `NSWorkspace.runningApplications()` scan materializes fresh
  `NSRunningApplication` objects, and the dead-pid sweep (1s) plus the
  new-window sweep (2s) ran ~90 scans/min × every running app ≈ 10,300
  objects/min, exactly matching the heap delta. Every AppKit touchpoint
  (`ns_workspace`'s scans, the notification blocks, `frontmost_pid`,
  `activate_app`, `menu_bar_height`) now owns one autorelease pool per
  call: the churn drains at the seam instead of piling up in the
  pool-less runloop.
- **The tick reads only what its verdicts can use** — steady-state AX
  traffic roughly halved, with the measured hot stacks gone:
  - **App identity is one memoized ask at the `OsAdapter` seam**
    (`app_identities`, batched for a pass; `app_identity` for events).
    Resolution used to cost two full `runningApplications` scans per pid
    per sweep — O(apps²), each `processIdentifier()` a synchronous XPC
    round trip — now one scan per miss wave, memoized per pid, GC'd
    against the live-pid snapshot so a recycled pid can never inherit a
    dead app's identity. Per-event identity reads (routing, placement
    capture, bar labels) hit the memo. `AppIdentity` moved to the
    adapter seam; GLOSSARY records the term.
  - **The hidden reconcile polls only windows whose verdict it can act
    on**: tree/popup members + Hide-parked. Fullscreen-parked windows
    and Untracked overlays were read every second and the answer
    discarded by construction. Membership is also one pass over the
    trees instead of an arena walk per window.
  - **The misplaced sweep plans its reads** (`LayoutWriteCache::sweep_plan`):
    only at-target windows past the write grace get their actual read
    (the drift detector); unsettled windows are already scheduled for a
    write, so their workspace re-applies without one — popups and the
    drag window were read every sweep and discarded. An expired pin
    backoff re-arms the sweep's retry through the re-apply; the pin's
    cadence is untouched.
  - **The Enhanced-UI guard borrows an app-element memo**: one app
    element per pid (evicted via `forget_pid`) instead of one per
    layout write — each write drops from ~3 synchronous round trips to
    the one flag read.
- **Interned CF strings** (`macos::cf_intern`): AX attribute names and
  CG dict keys are one interned `CFStringRef` per constant for the
  process lifetime, not a fresh `CFString` per AX call on the hot
  thread. `cg_window_level`'s single-window query deliberately has no
  snapshot cache: first-classification-wins makes a stale snapshot's
  `None` answer misroute a level≠0 overlay (documented at the site).
- **Seatbelt boundedness**: the placement map's entries die with the
  pid by design, so a churn-heavy app was the one unbounded map left —
  `note_where` now keeps the newest 8 records per pid (`noted_at`);
  the successor ask's candidates are always the recent ones.
- **Build footprint**: `tokio` narrowed to `sync`-only (the daemon
  never enters a runtime), a tuned `[profile.release]` (thin LTO,
  codegen-units=1, strip), and both UDS accept loops replaced the 25ms
  would-block spin with `poll(2)` — clients still connect instantly,
  the socket-identity tick rides the timeout, and ~80 wake-ups/s are
  gone. Plus in-place `window_count`, a lazily-formatted transient log,
  and one prefix lock per keystroke.

## v0.6.7 — Windows come home (2026-10-22)

- **Fixed: windows jumping workspaces after sleep.** A window parked on
  a hidden workspace (moved there, then sleep) woke up on whatever
  workspace was open at wake. Root cause chain: the wake resync only
  re-polls *listed* windows, so a hidden member's element ref went
  stale; macOS omits off-Space windows from its listings; and once the
  settle window passed, the listing diff's element probe misread the
  live window as dead, untracked it, and rediscovery re-homed it onto
  the active workspace. Two fixes: the listing diff now **never judges
  a hidden-workspace member** (tree membership is the evidence it is
  alive; a genuinely closed one drains through the write path's gone
  grace when its workspace is next revealed), and the new
  **placement seatbelt** (`state::placement::where_should`) re-homes
  any rediscovered window to where it last lived instead of the
  active-workspace fallback. Two wake tests pin the repro.
- **Placement is continuously maintained and persisted.** The
  seatbelt's identity (bundle + title) is captured at the moment a
  window's placement changes — including park time, so minimized
  windows keep their home — and the session snapshot is a projection
  of it: no quit-time identity read storm, and a throttled periodic
  save (≤10s stale) means a crash no longer costs the session's
  arrangement. The quit funnel lost its `cfg!(test)` guard (the I/O
  seam is the path argument) and the dead `session::clear_default` is
  gone.
- **`state::placement` is now one deep module**: one ask
  (`where_should` — seatbelt, then durable memory) and one write
  (`note_where`) behind a small interface; the reveal funnel gained
  the placement ask it was missing, and `DisplaySet::index_of_name` is
  the one name→index conversion.

## v0.6.6 — The wake settle window (2026-10-21)

- **Fixed: tiling after sleep.** The Space-switch fix's element probe
  was too eager at wake: the resync commits on any answered listing
  while window-element reads can still be blacked out, so the probe
  read a live window's element as unreadable, judged it dead, and
  untracked it in the same pass — and tiling broke after every sleep.
  The probe now inherits the wake patience the write path already
  had: the resync's commit opens a settle window (10s, the same
  "post-wake AX is still settling" grace the write path uses), inside
  which an unreadable element is the blackout's signature, not a
  death; the sweep re-judges once it passes, and genuinely closed
  windows' dead elements still testify against them then. The
  commit pass's own untrack was removed (it could judge nothing
  inside the window it had just opened). Two wake tests cover the
  regression face and the re-timed cleanup.

## v0.6.5 — A live element outranks a listing omission (2026-10-20)

- **Fixed: Space switches no longer lose windows' workspaces.** A
  window that went native fullscreen (its own macOS Space) came back
  routed onto whatever workspace was *active* after the user switched
  Spaces and returned — Firefox ws1→ws2, DrRacket ws3→ws1. The chain:
  while a window's Space is inactive, its app answers `kAXWindows`
  without it — visible tiled members included — and the listed-set
  sweep judged that answered omission a death, untracking the window
  outright (routing verdict, pid map, park entry and remembered
  workspace all destroyed). The next sweep rediscovered the window
  fresh, re-routed it onto the active workspace, and the refused first
  write parked it there. The listing diff now checks the window's
  element before judging: a live element (its rect still readable)
  outranks the omission — a listing artifact, not a death — while a
  genuinely closed window's dead element is judged dead exactly as
  before, so the fast close-detector keeps its speed. Real deaths stay
  owned by the destroyed notification, app termination, and the
  dead-pid sweep. The GLOSSARY's Listing entry records the sharpened
  judgment rule; two state-loop tests replay the full symptom chain
  (parked-window and visible-member faces).

## v0.6.4 — One read per owner (2026-10-20)

- **Drag detection restored for sweep-discovered windows; the AX
  registration dedupe is per (window, notification).** The registration
  bookkeeping deduped on window id alone with one set shared across
  notification types: the poll registered destroyed and then silently
  skipped moved — so `kAXWindowMovedNotification` was never registered
  by the poll path for any window, and drag detection (swap-on-hold,
  snap-back) for windows discovered by the 2s sweep with no
  `WindowCreated` notification (Firefox tear-offs, incognito) never
  fired. Keying per (window id, notification) registers both, each
  exactly once per observer lifetime. The `WindowCreated` callback's
  direct registrations stay direct, deliberately: it is a free C
  callback holding only the ObserverContext, and the element it fires
  on is the freshest evidence of the live element — the duplicate the
  next poll makes is tolerated by AX and bounded by the age refresh.

- **A client that never EOFs can no longer wedge the IPC pool.** The
  command server read until EOF with no timeout, so a client that
  wrote a command and never shut down its write side parked a pool
  thread in `read` forever; 32 wedged threads was "ipc thread pool
  full" and every CLI command failing until restart. Each read now
  carries a 5s timeout that fires as `WouldBlock` — the loop's
  existing end-of-payload path — so a stalled client frees its thread,
  and a partially-sent payload gets the same Invalid-JSON error a
  garbage payload gets.

- **Window creation pays one identity resolution per pid, not five
  NSWorkspace scans per window.** One routing pass re-read the app's
  bundle id four times and its name once — each a linear walk of
  `runningApplications` — so an app launching ten windows did ~50
  scans. The pass resolves bundle + name once and threads them down
  through the existing funnel; the burst discovery loop routes every
  window of an app against the same answer. Every check now also
  matches the same resolution instead of consecutive live scans.
  Precedence (CG level → restricted → float → tile) and placement
  memory's hit-once consumption are untouched.

## v0.6.3 — The sweep owns its reads (2026-10-20)

- **Install replaces binaries via temp file + rename, never truncating
  the live inode.** The previous in-place write left a window where
  `pengwm` was a half-written file — a daemon crash, a restart, or
  `pengwm update` running while the binary was executing could leave a
  truncated or busy binary. The new binary is written to a sibling
  temp file and renamed over the old one (atomic on the same
  filesystem), so the running inode is never touched mid-write.

- **Idle AX traffic roughly halved: the sweeps own their reads.** Every
  recurring sweep used to re-derive shared answers once per item — the
  1s hidden reconcile rebuilt the app element and re-read app-hidden
  for *every* window (N windows on one app = N reads/second, forever),
  the 2s window sweep took one `CGWindowListCopyWindowInfo` snapshot
  *per app* and re-read role/subrole for windows already in the element
  cache, and an app the OS could not name re-issued two linear
  NSWorkspace scans on every bar publish. Each shared answer is now
  resolved once at its natural owner and shared: app-hidden once per
  pid (`windows_are_hidden` batch), the level snapshot once per poll
  batch, classification exactly once per window id (the element cache
  keeps the class it first won — first-classification-wins now has a
  single enforcing point), and a failed label resolution is a
  timestamped verdict retried after 30s instead of every publish. No
  behavioral rule changed: Listing judgment, the AX blackout, level
  gating at birth and every park/funnel are untouched — on an idle
  8-app desktop the sweeps go from ~40–80 AX mach RPCs/s plus 8 CG
  snapshots per 2s to roughly half the RPCs and one snapshot, and a
  wedged app's 250ms-timeout stalls hit fewer reads per tick.

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
