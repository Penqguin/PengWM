# Window placement and overflow

Status: Implemented and validated, 2026-11-11. Automated checks, fullscreen
gestures, physical hotplug membership, native-desktop gating and return, and
periodic geometry recovery passed. Continuous geometry during physical hotplug
remains unverified.

## Agreed behavior

- Fresh tileable windows open on the current workspace by default, including
  apps associated with another workspace in the existing configuration.
- A fresh window tiles if the current workspace has room under `max_tiles`.
- A fresh window opened on a full workspace appears as an overflow overlay.
- Overflow windows travel with workspace switches. A full destination keeps
  them as overlays until room becomes available.
- When a destination has room, overflow windows tile there and become settled
  members of that workspace.
- When a slot opens on the current workspace before a switch, overflow windows
  immediately fill the available slots.
- Available slots go to overflow windows in opening order. Remaining overflow
  windows continue as overlays.
- An explicit workspace selection carries overflow even when the destination
  is already visible on another monitor. Ordinary monitor focus changes leave
  overflow on its current monitor. A traveling window that held OS focus at
  the source keeps it at the destination.
- A settled window restored from minimize, app hide, or fullscreen retains its
  home workspace. If full, it returns as a workspace-bound overlay until a
  slot opens.
- An explicit routing setting defaults to current-workspace placement. Users
  can select app routing to enable existing `apps` associations.
- Current workspace is evaluated when PengWM first discovers a fresh window.
  A slow launch discovered after a workspace switch opens on the newly current
  workspace.

- Overlays use the existing centered popup size. The newest overlay is raised
  and focused when opened. Window cycling reaches every overlay and tile.
- The directional move command carries a focused overlay to the workspace of
  the neighboring display: a destination with room tiles it, a full
  destination keeps it waiting there as a bound overlay.
- Explicit moves and enabled app rules establish a home workspace immediately.
  A full destination receives a workspace-bound overlay.
- Free slots serve returning settled windows first, then explicitly placed
  windows, then traveling overflow. Each group uses opening order.
- Across daemon restart, restore homes and pending overflow order for surviving
  windows whose identity can be verified. Fresh windows use the configured
  new-window placement policy.
- Minimizing traveling overflow pauses carrying. On restore, it resumes on the
  workspace where it was minimized and can travel on later workspace selections.
- Monitor removal transfers its pending traveling overflow to the primary
  monitor's current workspace. Settled homes follow the existing topology heal.
- On an unmanaged native desktop, suspend placement and carrying for that
  display until it returns to the managed desktop.

## Placement model

Represent placement intent separately from the window's OS classification and
parking reason. A tileable window can be traveling, waiting at an established
home, or tiled at its home. Minimize, app hide, and fullscreen suspend the
placement state and preserve its home, current workspace, and opening order.

Use one placement coordinator for admission, slot filling, carry, and return.
Keep workspace membership single-sourced, including membership for waiting
overlays. Store classification and parking through `WindowStore` as today.
Reuse overlay geometry and hide/show machinery while representing tileable
overflow distinctly from permanent popup windows and untracked windows.

Each live window has exactly one current owner. Traveling overflow belongs to
the workspace currently hosting it until an explicit workspace selection
transfers it. Explicit placement establishes a home and cancels traveling.
Pending windows count toward workspace membership and focus access, while only
tiled windows consume tile capacity. Workspace window counts and window lists
shown to the menubar and CLI cover every owned window: tiles, pending windows,
and popups.

For each event, update ownership and all applicable slot assignments before
writing layouts. Carry outgoing traveling overflow to the requested destination,
fill eligible slots in priority order, then hide/show and lay out the resulting
membership. A same-workspace request leaves placement unchanged. Capacity changes
must also run slot filling. Closing a pending window removes its queue entry.
Repeated discovery and return notifications must preserve order and ownership.

The in-session placement seatbelt trusts a record only while the evidence says
it is the same window: the window is still tracked (alive or parked), or its
"death" came from indirect evidence alone — the listing sweep, a Gone write
outcome — and may be a misjudgment of a live window, which is the rediscovery
the seatbelt exists for. A destroyed notification for the exact WindowId is
authoritative: the record expires, and a later window reusing that WindowId
routes fresh through the placement policy with its own opening order.

## Implementation sequence

1. Add the typed placement states and ordered slot-allocation decisions. Keep
   pure policy in `pengwm-core` and OS queries/actions in `pengwm-daemon`. Extend
   workspace ownership and focus access for tileable overlays. Avoid a second
   membership map alongside the existing workspace model.
2. Replace global capacity spill for the affected admission paths with local
   overlay admission. Route fresh births to the focused workspace unless the
   app-routing mode is enabled. Use `[windows] new_window_placement` with
   `current-workspace` as the default and `app-rules` as the opt-in value. Existing
   `apps` entries remain parseable and become active in app-routing mode.
3. Connect slot filling to close, move, minimize, fullscreen entry/exit, app
   hide/show, rediscovery, and capacity reload. Preserve established homes on
   return. Integrate overlay focus, close, move, and cycling through the normal
   command paths. Add forward/backward window cycling through the shared
   command type, CLI, and keybind parser because existing directional focus
   traverses tiled tree neighbors only. A newly promoted tile retains normal
   settled behavior.
4. Integrate carrying into explicit workspace selection, including destinations
   already visible on another output. Keep ordinary focus changes independent
   of carrying. Apply topology recovery and native-desktop gating through the
   existing seams. Reconcile deferred changes on return without duplicating a
   window or exceeding tile capacity.
5. Add verified restart recovery before bootstrap treats discoveries as births.
   Extend the versioned session shape with optional live-window identity,
   placement state, workspace name, and opening order. Restore verified records
   before allocating fresh windows. Reuse session writing and dirty-state
   persistence, including pending and parked windows.
6. Update configuration and architecture documentation, including the changed
   default for existing app associations, capacity behavior, explicit moves,
   restart recovery, and the new glossary terms. Mark this spec implemented only
   after the required tests and interactive validation pass.

## Restart identity work

Current persisted placement records contain bundle, title, and workspace name.
They cannot distinguish a surviving window from a newly opened replacement.
The in-memory seatbelt checks WindowId and PID and retains live members plus
eight stale records per PID. It also preserves pending intent and opening order
through same-window rediscovery. Restart snapshots cover every live member and
pending window independently of stale-record pruning.

Recovery compares WindowId, PID, OS boot-session UUID, process start time, and a
readable AXIdentifier unique among the application's current windows. Current
OS ownership and current classification must agree. Bundle/title matches alone
cannot authorize recovery. Missing or duplicate identifiers and mismatched
lifetimes use fresh-window routing. Recreated windows after app relaunch or OS
reboot also use fresh placement. Application-provided identifiers remain a
limitation, and apps without suitable identifiers cannot recover their homes.

Old session files continue to load. Records lacking verified identity provide
topology/gap restoration and diagnostic placement data. Deleted workspace names
fall back to fresh placement. Honor the existing `restore_last_session` control.
Recovery reflects the latest successful session save, including the existing
periodic-save interval after an abrupt daemon failure.

## Required validation

- Fresh-window routing with and without app associations and app-routing mode;
  delayed discovery after switching; old config and session decoding.
- Full origin, full destination, all workspaces full, multiple waiting windows,
  automatic promotion, priority ordering, explicit placement, and repeated events.
- Minimize, app hide, fullscreen, and rediscovery of settled and traveling windows,
  including return to a full or hidden home.
- Explicit cross-monitor workspace requests, ordinary monitor focus changes,
  monitor unplug/replug, native-desktop exclusion, and deferred return handling.
- Every overlay and tile reachable through window cycling and usable through
  focus, close, and move commands, including stacked overlays and hidden targets.
- Restart with more than eight windows in one app, pending and parked windows,
  identical titles, changed titles, stale records, PID/WindowId reuse, app relaunch,
  missing identity evidence, removed workspaces, and interrupted session writes.
- Run `cargo test` and `cargo fmt --check` after Rust changes. Interactively
  validate the scenarios on macOS with Accessibility permission, including
  rapid switches and multiple displays. Build success alone cannot verify
  ownership, focus, stacking, or actual cross-display placement.

## Validation record

- `cargo test --workspace` passes, including placement, lifecycle, compatibility,
  restart ordering, monitor-removal, and native-desktop gate regression tests.
- `cargo fmt --check`, `cargo clippy --workspace --all-targets`, and
  `git diff --check` pass.
- The ignored `window_placement_live` integration test was run with Accessibility
  access. It creates and automatically cleans up three dedicated floating-level
  macOS windows. It verifies actual tile/overlay sizes, focus cycling, workspace
  carrying, destination geometry, settled membership, and verified homes through
  recreation of the real adapter and state manager. Its temporary configuration,
  session, and UI socket are isolated from the installed daemon.
- Reproduce the native fixture test with
  `cargo test -p pengwm-daemon --test window_placement_live -- --ignored --test-threads=1`.
  It requires a macOS GUI session, Accessibility access, and Swift.
- Follow-up validation on 2026-11-11 proceeded after the user enabled window
  control permission. Both displays were observed on their first user desktop.
  The fixture supports `PENGWM_HARDWARE_VALIDATION=1` for a monitored session,
  with workspace, window, display, and Space snapshots in its temporary directory.
- Native fullscreen entry and exit passed through the fixture's green button
  and Control-Command-F shortcut. Traveling overflow preserved its home and
  traveling intent while parked in a type-4 Space. A settled tile freed its
  slot, another window filled it, and exit returned the settled window to its
  original home as a Returning overlay. Standard window levels were selected
  through the fixture's View menu for the keyboard gesture checks.
- The user physically disconnected display 15 and reconnected it as display 16.
  The settled Fixture B window retained its home while its workspace was reseated
  to primary, then returned to the reconnected display. Traveling overflow moved
  from Fixture B to the primary display's active Fixture A and remained there
  after reconnect. Membership counts changed from [1, 2, 0] to [2, 1, 0].
- The initial hardware monitor omitted the periodic recovery tick and exposed
  a late offscreen position after hotplug. The monitor now runs `on_tick` and
  excludes other running application PIDs from admission. A real-window replay
  deliberately displaced a tile and verified that the production tick restored
  its original rectangle within ten seconds. The native integration test passes
  with this recovery assertion. Continuous geometry during the physical
  disconnect/reconnect transition remains unverified.
- The user switched external display 16 from its first type-0 desktop 1802 to
  type-0 desktop 1805. The detector reported `allowed = false`. An explicit
  request for Fixture B on that display left homes, workspace counts, and OS
  rectangles unchanged. A switch from Fixture A to Fixture C on the built-in
  display remained functional and admitted traveling overflow there.
- The user returned display 16 to desktop 1802. The detector reported
  `allowed = true`, the previously blocked workspace request succeeded, and
  the carried window tiled within the external display's usable bounds.
  Final membership counts were [1, 1, 1], with settled homes retained. The
  keyboard desktop shortcut did not change Spaces in this session, and
  automated Mission Control access timed out, so the native-desktop gestures
  were performed by the user.
- The final workspace suite, Clippy, formatting, and diff checks pass. The
  monitored test exited successfully, and its dedicated application and test
  windows were confirmed closed. Validation exercised the working-tree engine
  with isolated fixture configuration and windows.
- Review follow-up (2026-11-11), same day: a confirmed destroy now expires
  the in-session seatbelt record, so a churn-reused WindowId routes fresh —
  the rediscovered-overflow preservation test now drives the indirect
  untrack funnel (`on_window_unlisted`), and a regression covers the
  recycled id. New tests cover the directional move of a focused overlay,
  the deleted-workspace restore fallback, and a truncated session write
  decoding as no session. Counts, carried focus, and the directional-move
  behavior were decided in this pass and amended into the spec text above.
  `cargo test --workspace` (365+100), `cargo fmt --check`, and
  `cargo clippy --workspace --all-targets` pass.

## Prior implementation constraints

Before this change, `state/lifecycle.rs` routed fresh tiled windows through
rediscovery placement, app rules, and then the active workspace. Its admission
helper applied capacity overflow to both fresh and returning windows. Returning
windows could leave their remembered workspace when it had filled up.

`DisplaySet::plan_switch` focuses another monitor when the requested workspace
is already visible there. A requested workspace switch and an ordinary monitor
focus change need explicit overflow semantics.

ADR-0005 originally omitted persisted placement consumption at daemon restart.
ADR-0008 extends it with verified live-window recovery. ADR-0006 preserves membership through
automatic topology changes. ADR-0007 gates OS actions per display while its
native desktop is outside PengWM's managed desktop.
