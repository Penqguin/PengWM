# ADR-0007: Native-Space gate — manage only each display's first desktop

Date: 2026-11-03
Status: Accepted

## Context

PengWM assumed it owned the whole display: every tracked window is
tiled, hidden, raised and focused regardless of which macOS Space the
display is showing. Users routinely keep other desktops for full-screen
apps, personal work, or a "do not disturb" space — and on those Spaces
PengWM still moves windows, hides them, and answers keybinds for a
desktop the user is not looking at.

The requirement: the WM is functional on the **first desktop of every
display** and ignores every other native Space. With "Displays have
separate Spaces" (already a prerequisite), the gate must be per display:
display B on Desktop 1 stays fully managed while display A sits on
Desktop 2.

Native Spaces are invisible to the public APIs PengWM already uses:
`kCGWindowWorkspace` reads 0 for every window on modern macOS (confirmed
on this machine: 0/136 windows), and AX does not expose space
membership. The WindowServer's own per-display list is available through
SkyLight's read-only SPI — the same source yabai and AeroSpace use.

## Decision

### Gate model: display-level action freeze

While a display's current space is a **user desktop other than the first
user desktop** in that display's ordered space list, the WM performs no
OS-visible action on that display. The model keeps tracking windows
(births, notifications, membership bookkeeping) — only the writes are
suppressed:

- `apply_layout` computes targets and updates the drag hit-test rects,
  then returns before any `set_window_rect`.
- `hide_workspace` / `hide_window_in_workspace` queue the hide instead of
  writing it (the deferred-hide queue).
- `reveal_stack` / `raise_untracked_windows` skip the display.
- Misplaced, hidden-state and listing-omission judgments skip windows
  homed to the display (their AX evidence is ambiguous across a Space
  transition).
- Commands acting on the display are refused (see below).
- `focus_window` is never called for it.

A membership filter (untrack windows when the user leaves) was rejected:
it would lose seats, re-route on return, and fight the hidden-member
shield. Actions, not membership, are gated.

### Anchor: first type-0 space per display

`SLSCopyManagedDisplaySpaces` returns per display: a `Display Identifier`
(UUID), ordered `Spaces` (each with `id64`, `type`), and the current
space. The anchor is the first space of type `0` (user desktop) in that
display's own list. Each display is compared independently, so the rule
works with separate Spaces and degrades naturally in span mode (each
display's current space is judged against its own list).

Fullscreen (`type 4`) and system (`type 2`) spaces are **exempt**: the
existing fullscreen parking machinery owns those transitions, and a
display showing a fullscreen app is not "off in another desktop". A
display missing from the snapshot is treated as allowed (fail open).

### Detector: read-only SkyLight, no SIP changes

`macos/spaces.rs` `dlopen`s SkyLight and resolves
`CGSMainConnectionID`, `SLSCopyManagedDisplaySpaces`,
`SLSSpaceGetType` and `CGDisplayGetDisplayIDFromUUID`. It only reads —
no injection, no code signing changes, no SIP disabling. If SkyLight or
any symbol is unavailable, the detector returns `None` and the state
layer fails open (all displays managed), logging once; it recovers
automatically if a later snapshot succeeds.

The snapshot is polled at most every 100 ms on the tick and forced
immediately by `NSWorkspaceActiveSpaceDidChangeNotification` and by
every command (a keybind right after a swipe cannot wait for the poll).
There is no debounce: enter limbo immediately, leave (and resync) on the
first answer that says first desktop.

### Return resync

Leaving limbo scopes the resync to that display: drain its deferred
hides, re-tile the workspace it shows, restore stacking, publish bar
state. No element-cache clear is needed (unlike wake): the AX refs were
never invalid, only unused. Deferred hides are also drained on every
snapshot for workspaces whose display changed while they waited (a
hotplug remap must not strand them); entries whose window moved,
was destroyed, or whose workspace became visible are dropped.

Entering limbo clears the drag state: an in-flight drag must not keep
swapping tiles on the desktop the user is leaving.

### Command refusal

The gate is per acting display:

- Commands that act on the focused workspace (`Focus`, `MoveWindow`,
  `Split`, moves to a workspace, `Close`, layout presets, magnify,
  resize) are refused when that display is gated.
- `Workspace` gates at the resolved target; `FocusDisplay` and
  `MoveWindowToDisplay` gate at every display they would act on;
  `MoveWindowToWorkspace` gates source and destination.
- Global, model-only commands stay available: `SetGap*`, `ReloadConfig`,
  `RevealAll`, `QueryState`, `Quit`. Their OS writes still
  go through the gated write paths, so `RevealAll` reveals windows on
  allowed displays and defers the rest.
- Refused commands still ack the caller.

### Startup and scope

The gate is armed in `StateManager::new` **before** the initial
hide/layout pass, so starting the daemon while a display shows Desktop 2
does not tile or hide anything there. The feature is always on — no
config key in v1.

## Rejected alternatives

- **Untrack memberships while away**: loses seats and placement, forces
  re-routing on return, and needs per-window space attribution (the
  detector answers per display, not per window).
- **Config knob**: the user's requirement is unconditional; an off switch
  only adds a state matrix to test.
- **Per-window `CGSCopySpacesForWindows` gating**: the per-display gate
  already satisfies "don't act while I'm on Desktop 2"; per-window
  attribution belongs to the rejected membership model.
- **Debounce before entering limbo**: a command in the debounce window
  would act on the wrong desktop. Immediate is safer.

## Consequences

- Keybinds, layout writes, hides, raises and focus calls are inert on
  non-first desktops; every first-desktop display remains fully managed.
- Windows created on a gated display are still discovered and routed
  (model bookkeeping); a window admitted to a hidden workspace there is
  hidden on return, not while away.
- Known limitation (accepted under the action-gate model): a *managed*
  window that lives on another native Space can still be focused from a
  first-desktop display and macOS may follow it to that Space; the WM
  then observes the display entering limbo and goes idle. Full per-window
  space ownership is the rejected model.
- The detector is private SPI. It is read-only and fails open; a future
  macOS that removes it degrades to the old "manage everything"
  behavior with one warning, not a crash.
- `docs/architecture.md` (Native-Space gate), `CONTEXT.md` (glossary),
  `docs/getting-started.md` and `docs/troubleshooting.md` describe the
  behavior.

Refs: `macos/spaces.rs`, `state/space_gate.rs`, `state/tests/space_gate.rs`,
`native_space_snapshot_matches_active_displays` integration test.
