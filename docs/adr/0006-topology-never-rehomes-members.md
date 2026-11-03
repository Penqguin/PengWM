# ADR-0006: Topology changes never re-home members

Date: 2026-11-03
Status: Accepted

## Context

A topology sync (monitor add/remove/resize, a wake handshake, or
geometry drift from learned top bands / AppKit insets) currently calls
`wm_reset` (`reset.rs`), which replaces every `Workspace` tree, the
`WindowStore`, and clears `live_placement` — then re-routes all members
through the `WindowCreated` birth handler: config routing, then the
active seat. For an unassociated app (no configured workspace) both
config routing and the seatbelt miss, so the birth falls through to
`active_workspace_idx()`. After the fresh no-session assemble that cell
is the primary's initial seat: flat index 0 — workspace 1.

Result: any non-noop topology sync carries every unassociated window
(and the view) back to workspace 1 within seconds, with no user action.
That is the v0.6.10–6.12 regression the reporter hit ("moved to another
workspace, then moved again, eventually returns to workspace 1").

Pre-ab83108 behavior was an incremental heal — reseat, relayout, prune
bands — that never touched tree membership or the seatbelt, so manual
moves survived every topology wobble. The reset replaced that heal.

This also contradicts ADR-0005, which promises the seatbelt returns a
window the store lost to where it lives *this session*: the reset
deletes that memory (`live_placement.clear()`) *before* re-discovering
the very windows it applies to.

## Decision

A topology change never re-homes a member. It rebuilds seats and
geometry only (incremental heal: reseat affected workspaces, relayout,
prune learned bands); membership and the active/visible arrangement are
preserved.

- A full reset (`wm_reset`) is reserved for structural config changes or
  an explicit command — never an automatic topology reaction.
- The seatbelt (`live_placement`) is never cleared by a topology
  reaction; birth re-routing consults it first, honoring ADR-0005.
- Trigger discipline: a real monitor add/remove runs the incremental
  heal (orphan reseat to primary, new output seats the first hidden
  workspace); a resize reseats + relayouts only; learned top bands and
  AppKit insets are presentation, not topology — they must never report
  as geometry-changed for this diff.

## Rejected alternative

- Keep reset-as-rebirth but only for real add/remove (not resize or
  drift): still re-homes on every real hotplug — manual moves would be
  lost whenever a monitor is actually plugged or unplugged, exactly when
  the user most wants them preserved.
- Keep the current semantics and fix only the phantom triggers: leaves
  the self-perpetuating reset machine (bands are read before they are
  cleared, guaranteeing another non-noop on the next tick) and the
  ab83108 contract intact; the next trigger variant re-introduces the
  bug.

## Consequences

- Manual moves and the visible workspace arrangement survive sleep/wake,
  hotplug, resize, and band/inset drift.
- Topology sync becomes a reseat/relayout, never a re-derivation of
  membership; `wm_reset`'s "members re-route as births" contract no
  longer applies to automatic topology reactions.
- ADR-0005's promise holds across topology changes: the seatbelt returns
  a window the store lost to where it lives this session.
- `docs/architecture.md` (Workspace Pool) and `docs/commands.md` must
  describe incremental-heal semantics; the old "swap on switch" wording
  is stale (v0.6.10 made cross-monitor switches focus-only).

Refs: ADR-0003 (identity/membership split), ADR-0005 (placement seatbelt),
ab83108 (introduced the reset), cf02db1 (workspace ask precedence),
55fe390 (inset/band geometry), b0f55fd (AppKit cache refresh).