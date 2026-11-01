# Spec 0001: Transient Workspace HUD

Date: 2026-11-01
Status: Accepted (decided in a grilling session, 2026-11-01)

## Problem

After an explicit `workspace-N` switch or a monitor focus change
(`focus-display-*`), the user does not know what landed where: which
workspace is on which physical monitor, and which monitor is focused.
Both failures — momentary switch-time disorientation and wanting
persistent cross-monitor awareness — are in scope; switch-time is the
heavier one.

### Why the removed bar's shape is off the table (short-term)

The v0.5.2 status bar (egui strip + `[bar]` config + `BarReserve`)
died because it did not look right, caused tiling friction, and was
over-complicated for its value — not because the concept was wrong.
The pure geometry scaffolding (`BarPosition`, `bar_strip_rect`,
`Workspace::set_reserved_rect`) stays retained exactly as CONTEXT.md
records, so a future persistent bar can be reconsidered *after* the
HUD teaches us whether persistent awareness is actually missed. A
future bar's design decisions get to inherit the HUD's styling knobs
rather than the dead egui strip's.

### Why menubar-only is disqualified

NSStatusItems render on the menu bar, which lives on the primary
display. A menubar icon can list workspaces, but it can never annotate
a *second* monitor with what that monitor at that moment shows. Per-
monitor truth requires overlays that live at the monitors.

## Decision

A **transient per-monitor HUD** — tmux-flavored, drawn by the daemon
process:

1. **One pill per monitor.** Small dark translucent rounded pill,
   dead-center of each monitor, monospaced text: `2 · Browsing`.
   The focused monitor's pill is accented (brighter text). Pills are
   click-through, non-activating, floating-level, and join all spaces
   (they overlay native-fullscreen workspaces too).
2. **Explicit triggers only.** Pills flash on:
   - `workspace-N` switch / swap (the `Command::Workspace` path),
   - monitor focus change (`Command::FocusDisplay`),
   - the new recall command below.
   Background changes — app-routing spills, hidden-workspace swaps,
   topology sync, the tick sweeps — never flash. A HUD popping up
   mid-typing is the removed-bar failure in flashlight form.
3. **Recall keybind.** New `Command::HudToggle`, action string
   `hud-toggle`, CLI `pengwm hud-toggle`, default keybind
   `alt-w`. (The planned `prefix, h` was rejected: prefix follow-ups
   inherit the prefix's modifiers, so `prefix, h` collides with
   `alt-h` = focus-left, the single most-used chord.)
4. **Prefix fusion.** While the `alt-space` chord is armed, the pills
   stay visible (every tick extends them while `PrefixKey::is_armed`)
   and show a `»` marker, so the HUD doubles as the invisible-so-far
   prefix indicator. The pills hide ~`duration_ms` after the chord
   disarms.
5. **Timing knob.** Flash duration is `hud_duration_ms` (default
   500); the whole feature can be silenced with `hud_enabled`
   (default true). Config reload refreshes both like every other
   setting.

## Architecture

Same split the codebase already uses everywhere else: pure planner,
OS executor.

- **Planner (pure, tested):** `state/hud.rs` builds
  `Vec<HudCard>` from the current snapshot — display list, per-
  monitor visible workspace (`DisplaySet::active`), focused output
  (`DisplaySet::focused_output`), and the prefix's armed state.
  `HudCard { display_id, number, name, focused, prefix_armed }` and
  the label formatter (`1 · Development`, `» 2 · Browsing`) are
  pure functions; state tests assert cards without any macOS.
- **Executor:** `OsAdapter::show_hud(cards)` / `hide_hud()`.
  - `MacOsAdapter` owns a `HudPanels` map (display id → panel +
    label): panels are created lazily on the main thread
    (`MainThreadMarker::current()` — the event loop's thread;
    a `None` marker logs and skips, keeping the off-thread path
    safe), positioned from the matched
    `NSScreen` frame (AppKit global coordinates, via the
    `NSScreenNumber` device-description mapping the codebase
    already uses), restyled per flash, `orderFrontRegardless`.
    `hide_hud` orders every panel out.
  - `TestAdapter` records shows/hides so state tests can assert
    triggering without macOS.
- **Prefix-armed signal:** the event tap, which observes
  `PrefixOutcome::Armed`, queues a new `DaemonEvent::PrefixArmed`;
  the event loop dispatches it to `StateManager::on_prefix_armed`,
  which draws the prefix marker.
- **Expiry:** `StateManager` owns one deadline cell
  (`hud_hide_at`). `on_tick` extends the deadline while the prefix
  is armed and hides through the adapter once it passes. The
  display path never owns timers; the run loop's 50 ms tick is the
  clock.

## Rejected alternatives

- **Persistent bottom bar now** — deferred, not rejected; the
  retained geometry keeps the option open, and this HUD is the
  cheapest honest probe of whether the persistent surface is
  missed at all.
- **`pengwm-hud` child process** — initially rejected here in favor
  of daemon-process drawing; the hotplug field data later reversed
  that: a daemon running its own CFRunLoop never receives AppKit's
  screen-list maintenance the normal way, and every manual refresh
  path produced clamps or default placements on freshly re-plugged
  displays. The final architecture is the iterate-but-correct one:
  the **menubar process renders the pills** (it runs
  `NSApplication.run()`, so AppKit maintains its screens itself), the
  daemon plans cards — centers precomputed in AppKit space from the
  tiler's own display snapshot — and pushes them over the existing
  `BarMessage` channel (`Hud { cards }` / `HudHide`, cached and
  replayed like `State`).- **Grid of the menu bar** (per-monitor text slate in the
  status item) — disqualified above: no per-second-monitor
  positioning exists.
- **`prefix, h` default binding** — collides with focus-left
  (follow-ups inherit the prefix modifiers); `alt-w` chose a free
  direct chord instead.

## Consequences

- Switch-time disorientation gets a per-monitor answer at every
  monitor, cheap and never triggered by background work.
- The daemon grows one idle-time UI surface; it creates panels
  only when something flashes, so boot and idle cost nothing.
- HUD windows are never in PengWM's own AX element cache: the
  daemon polls only other pids' windows, and even a stray
  encounter classifies `Untracked` via the non-zero window-level
  gate (ADR-0002). No reservation machinery is touched — the HUD
  floats; the bar scaffolding stays dormant.
- Refs: CONTEXT.md (Daemon↔Menubar contract, status-bar removal
  note). Follow-up: if the persistent bar ever lands, it should
  restyle from these pill knobs instead of resurrecting the egui
  strip.
