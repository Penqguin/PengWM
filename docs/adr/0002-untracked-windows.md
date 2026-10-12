# Untracked windows: tracked-but-floating, with a default float list

Context: geometry- Overlay-style windows (System Settings, Calculator, PiP, Slack-call
decks) either tile or get misclassified as workspace-bound popups today. We decided
untracked is a *first-class, tracked-but-floating* window class, not the yabai-style
"leave-the-window-alone" drop.

## Decision

- **`WindowClass::Untracked`** exists and follows the same cache-rides-the-class pattern
  as popups: store-tracked (pid maps, destroy/`forget_pid`/prune funnels all work), never
  tiled, never parked, never counted against `max_tiles`, never surface in `BarState`.
- **Cross-workspace visibility is achieved with a raise funnel**, not by re-parenting:
  whenever the visible workspace on a monitor changes, every untracked window on that
  monitor gets one `AXRaise` after the show/hide completes. This reuses the popup
  switch-back-raise trick.
- **Structural detection (default-on):** a window whose CG window level ≠ 0 (queried via
  `CGWindowListCopyWindowInfo`, matched by the AX window id) classifies `Untracked` at
  creation, whatever its subrole (level > popup subroles). First-classification-wins.
- **App-level detection:** `[windows] float_apps` (default: System Settings, Calculator,
  Photo Booth) — bundle list matched like `restricted_apps`, at creation time only.
- **Precedence:** CG level ≠ 0 → popup subroles (`restricted_apps` keeps its existing
  force-popup role) → `float_apps` → tile. `AXSheet`/unknown still drop.
- **Kill-switch:** `[windows] float_level_overlays = true` (default). When false,
  windows that are untracked *only* because of the level gate tile again; windows that
  are untracked because of `float_apps` (or cached classes) still float.

## Rejected alternative

The yabai-equivalent "unmanaged" shape (never stored, never cached) was rejected: a
level-0 window like Settings does not stay above PengWM's hide/show workspace switching,
would not survive parking sweeps by construction, and could not be re-raised. It buys
zero state but cannot deliver "visible on top until closed" for the apps the user asked
about.

## Consequences

- iOS-PiP-style `AXFloatingWindow` windows and Chromium popups that are level ≠ 0 stop
  being workspace-bound popups and become cross-workspace untracked windows.
- Slack/Teams call decks (`AXStandardWindow`, level 3) stop tiling.
- Untracked windows are never hidden by workspace switches; app-level Cmd-H (macOS)
  still hides them and PengWM re-raises only on the next visibility change.

Refs: `docs/research/untracked-windows.md` (evidence: yabai `window_manager.c:272-280`,
AeroSpace `windowLevelCache.swift`, Apple `NSWindow.Level`).
