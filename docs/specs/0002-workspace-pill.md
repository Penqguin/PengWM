# Workspace pill reimplementation

Status: Implemented, 2026-11-04; interactive macOS validation pending.

## Settled requirements

- Identify the destination workspace after explicit workspace switching.
- Retain the previous pill's appearance; the user liked it when it worked.
- Prioritize reliability, including placement after monitor unplug/replug and
  avoiding focus theft.
- Exclude monitor-focus notifications, manual recall, and prefix-key indication.
- Show pills on both monitors when switching to a workspace visible on the other
  monitor; highlight the monitor gaining focus.
- Requesting the already-focused workspace flashes confirmation as well.
- Keep the previous 500 ms duration. Rapid switches immediately replace labels
  and restart the duration, without queued pills or animations.
- Validate hotplug placement, latest labels during rapid switching, automatic
  dismissal, and preservation of keyboard focus.
- On each explicit workspace request, show each managed monitor's current
  workspace and highlight only the destination monitor. This applies to one,
  two, or more monitors and to empty destination workspaces.
- Respect the native-Space gate in ADR-0007: excluded displays receive no pill;
  refused workspace requests do not flash. Background workspace changes do not
  trigger pills.
- Restore reloadable enable/disable and duration settings, defaulting to enabled
  and 500 ms. Keep the previous appearance without additional styling settings.

## Rendering and lifecycle

The daemon determines workspace labels and the destination from the resulting
workspace state. The menubar process renders click-through, non-activating pills
on its AppKit main thread using its normal application run loop.

`DisplaySet::plan_switch` returns `None` for an already-active workspace, so
the command path must distinguish a valid confirmation request from an invalid
or refused request before relying on the switch decision to trigger a flash.

Treat each flash as transient delivery, separate from cached workspace state.
Never replay a flash after a client reconnects. The renderer owns local expiry
so losing the daemon connection cannot leave pills stuck. A newer flash replaces
the previous labels and deadline; old expiry work must not hide a newer flash.

Disconnecting a monitor removes its pill. Reconnecting a monitor or the menubar
does not trigger a flash; wait for the next explicit workspace request. Unaffected
displays keep their existing pills and deadlines. A display
entering an excluded native desktop loses its pill without flashing other
displays. Disabling the feature dismisses visible pills immediately.

Match displays by stable display identity using current geometry. Never silently
place an unmatched display's pill on the primary monitor. Validate positioning
on replugged monitors, mixed scaling, and monitors above or left of the primary.

## Validation required before completion

- Focused tests for explicit switching, already-active confirmation,
  already-visible destinations, multi-monitor labels/highlighting, excluded
  displays, and absence of flashes for background changes.
- Lifecycle coverage for replacement/expiry races, reconnect without replay,
  disconnection cleanup, and configuration reload.
- Run `cargo test` and `cargo fmt --check` after Rust changes.
- Interactive macOS validation: rapid switching, empty workspaces, monitor
  unplug/replug, mixed scaling and monitor arrangement, native-Space exclusion,
  menubar reconnect, correct centering, dismissal, and preserved keyboard focus.
  Build success alone does not establish correct window behavior.

## Evidence

The retired design is recorded in `0001-workspace-hud.md`. Historical commit
`882877b` moved rendering into the menubar process because daemon-side AppKit
screen information became stale after hotplug. This is a constraint to investigate
and validate, not proof that restoring the old implementation fixes the bugs.

Current workspace commands focus the destination display when the requested
workspace is already visible elsewhere. They do not automatically swap it onto
the originating display.

## Scope boundary

This spec supersedes the retired HUD spec for the workspace pill only. It does
not restore the persistent bar, monitor-focus notifications, manual recall, or
prefix-key indication.
