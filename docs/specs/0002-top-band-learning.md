# Spec 0002: Confirmed Top-Band Learning

Date: 2026-11-03
Status: Implemented

## Problem

The macOS adapter can learn a display-wide top inset from a failed window
write. Previously, any vertical mismatch qualified, and the window's
absolute `y` coordinate was stored as the inset. A centered DrRacket dialog
targeted at `y=184` and read back at `y=189` therefore taught a 189 px inset.
Topology sync applied that value to every workspace on the display.

## Behavior

- Only ordinary tiled windows on a workspace's top row may provide evidence.
  Workspace popups, untracked overlays, and the magnified overlay are excluded.
- Evidence requires a failed layout write plus a fresh CoreGraphics window
  frame. AX-only readbacks never teach display geometry.
- The WindowServer frame must be at least 8 px below the requested top-row
  position and no more than 200 px below it. Two consistent observations on
  the same display (within 4 px) are required before changing its inset.
- The learned inset is the existing effective inset plus the verified extra
  displacement. The target already includes the macOS `visibleFrame` inset
  and outer gap, so neither is counted twice.
- Pending evidence and learned values are cleared when raw display bounds or
  the AppKit top inset changes by more than 0.5 px, and when a display
  disconnects.

## Rationale

The top-row designation comes from the workspace tree layout, not a guessed
coordinate threshold. This lets real top-edge clamping be detected while
centered overlays cannot alter geometry shared by the display's workspaces.

## Validation

`cargo build` and `cargo clippy` must pass. On macOS, reproduce a centered
dialog placement and confirm it produces no band-learning log; a real
top-edge clamp should log one candidate observation and learn only after a
second consistent CoreGraphics readback.
