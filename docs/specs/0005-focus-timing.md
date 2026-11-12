# Concentrate focus timing and acceptance

Status: Implemented and validated; native click/observer-ordering gaps are recorded below.

Research: [focus timing](../research/focus-timing.md).

## Settled scope

- Preserve current focus behavior: the 150 ms switch guard, 250 ms quiet
  period, fallback order, and existing event/command distinctions.
- Concentrate timing and focus acceptance in one module. A timer-only
  module would leave ordering knowledge with its callers.
- Workspace retains window focus and membership; DisplaySet retains
  focused output and visible assignments. Fullscreen parking executes
  through the membership module.
- Treat behavior fixes as separate decisions, not incidental refactor work.
- Implement the focus module privately within StateManager. It executes
  acceptance rather than returning update instructions for callers to
  assemble; reuse the existing Workspace, DisplaySet, membership, and
  OsAdapter operations.
- Use one explicit timestamp per decision, passed through immediate and
  settled timing comparisons. Retain the existing delays and comparison
  order; introduce no clock adapter. This explicitly clarifies timing:
  an OS read taking time does not advance the timestamp halfway through
  the same decision.
- Encapsulate the dirty timestamp, switch deadline, and settled focus
  memory in private focus state. Callers use intent-based operations,
  not temporal-field setters. This replaces the existing cells rather
  than duplicating them.
- Keep frontmost_pid shared on StateManager; activation, wake recovery,
  and active-workspace resolution retain their current use of it.
- Use explicit timestamps for timing tests and retain command/event
  integration coverage. Assert focus and observed adapter effects through
  the focus module's interface, not internal helper calls.

## Current behavior to preserve

- Only external WindowFocused dispatch arms/re-arms the quiet drain.
  AppActivated invokes immediate acceptance without arming it itself.
- Eligible visible observations update Workspace focus before switch
  protection is checked. Protection holds the focused-output seat,
  not the already-updated Workspace focus.
- A pending wake resync retains dirty focus work. Once quiet and permitted
  by wake judgment, the drain consumes dirty work before checking the
  switch deadline; a newer switch can therefore discard older due work.
- A settled OS Some(window) updates last-window memory before acceptance.
  Rejected Some observations do not invoke the fallback ladder. Only OS
  None tries a visible remembered member, then leaves the current seat.
- Structural reset clears switch protection but does not explicitly clear
  dirty focus work or last-window memory. Topology healing remains distinct
  from structural reset under ADR-0006.
- Native-Space gating continues to distinguish actions from bookkeeping
  under ADR-0007; this refactor adds no blanket freeze of focus bookkeeping.

The research note cites the current code and tests for these constraints.
These are preservation requirements, not newly selected behavior policies.

## Module and operation responsibilities

Implement `state/focus.rs` as a private module with private temporal state
and StateManager operations. Existing event entry points retain their
external call shape. Internal explicit-time operations let production
and tests exercise the same decisions. Final Rust helper names are an
implementation detail; the following responsibilities define the seam:

| Operation | Module responsibility |
| --- | --- |
| External focus arrival | Arm/re-arm quiet work and attempt immediate acceptance using one supplied instant |
| Immediate acceptance | Preserve parked/fullscreen/visibility checks, Workspace focus, switch protection, output update, and existing publication order |
| Settled drain | Respect wake judgment and quiet/switch precedence, sample adapter focus, update shared frontmost PID and settled memory, execute acceptance/fallback with the same instant |
| Successful workspace selection | Install switch protection at the existing post-placement/focus/stacking checkpoint |
| Structural reset | Clear switch protection only, preserving the current lifetime of dirty work and settled memory |

The focus module executes acceptance and its existing publication effects;
callers do not execute a returned checklist. Fullscreen detection continues
to request parking through the membership funnel, retaining its existing
publication and layout effects. Commands still resolve destinations and
perform their own explicit OS focus requests. App activation still performs
discovery and updates frontmost_pid before invoking immediate acceptance.

Keep timing constants and temporal-field access local to the focus module.
Constructor initialization and the blackout test's dirty-state query use
the module interface. Do not expose generic timer setters, inject a clock
trait, or change OsAdapter. MacOsAdapter and TestAdapter already justify
the existing observation/actuation seam.

## Time contract

- Production focus entry points capture the decision instant once; the
  external event uses that instant both to arm quiet work and to attempt
  acceptance. Activation's immediate acceptance starts its own decision.
- The tick continues supplying its existing sampled instant to the settled
  drain. Nested acceptance and fallback reuse it rather than sampling time
  again after OS queries.
- Workspace selection installs its 150 ms deadline using the instant at
  the existing checkpoint, not the earlier start of command execution.
- At exactly 250 ms of quiet, settling is eligible. At exactly the switch
  deadline, protection no longer suppresses the focused-output update.
- Later observations restart the quiet period; later successful selections
  replace the protection deadline. No timer callback or queue is added.
- One decision instant is the explicitly agreed clarification to current
  clock sampling. Do not infer other changes to acceptance, fallback,
  dirty consumption, or reset behavior from it.

## Verification and migration plan

1. Map existing focus, command, reset, fullscreen, Space-gate, and wake tests
   to the preservation requirements. Add missing non-clock behavior
   characterization before moving its implementation.
2. Introduce the internal explicit-time paths and pass the same instant
   through nested acceptance. Pin the agreed clock contract with exact
   deadline tests; avoid testing synthetic settle time against a remaining
   real-time acceptance check.
3. Move timing and acceptance into the private focus module, replacing the
   three StateManager cells with private focus state. Migrate switch/reset
   callers and the blackout dirty-state query to intent-based operations.
4. Replace timing sleeps in focused policy tests with supplied instants.
   Retain a small number of event/tick integration checks where real
   scheduling is the behavior under test; do not add a global tick-clock
   abstraction just to remove every sleep.
5. Independently review behavior/order preservation, run targeted tests,
   then `cargo test`, `cargo fmt --check`, and
   `cargo clippy --workspace --all-targets -- -D warnings`.
6. Validate actual focus with a dedicated macOS fixture: click/event
   correction, workspace/display selections including an empty output,
   and rejection of hidden-workspace focus. Pure timing tests do not
   establish native event ordering. Record any native validation gaps.

Required focused scenarios include quiet 249/250 ms, switch 149/150 ms,
repeated arrivals/selections, older dirty work overlapping a newer switch,
visible second-output observations during protection, rejected OS Some
candidates, None fallback to visible/hidden/destroyed remembered windows,
activation without a focus event, wake retention/recovery, and structural
reset with armed work. Check existing coverage before adding each test.

Update stale comments that describe switch protection as preventing
DisplaySet active-assignment mutation or settled memory as necessarily an
accepted window. Update architecture notes to name the new seam.

No dependencies, configuration changes, new commands, or new ownership of
membership/display assignment are required. The private state record is
an implementation choice that is easy to reverse; no new ADR is needed.

## Implementation record

- Private `state/focus.rs` owns FocusState and all immediate/settled
  acceptance. The three former StateManager cells are removed; shared
  frontmost_pid and the existing Workspace/DisplaySet owners remain.
- Public observation entry points retain their call shape; internal
  explicit-time entry points are used by production and deterministic tests.
  Nested OS observation and None fallback acceptance reuse the drain's time.
- Successful workspace selection and structural reset call intent-based
  operations at their original checkpoints. The blackout test's armed-state
  query lives with focus state rather than reaching across its seam.
- Four non-clock characterization tests passed before production edits.
  The existing four focused-policy tests now use supplied timestamps;
  seven additional tests cover exact deadlines, rearming, overlap, nested
  timestamp reuse, reset protection, and destroyed remembered members.
  The wake tick integration test retains its scheduling sleep.

## Validation results

- `cargo test --quiet`: 501 passed, 4 ignored. The suite ran outside the
  sandbox to permit Unix socket test binds.
- `cargo fmt --check`, `git diff --check`, and
  `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- Independent read-only review found no production behavior regressions.
  Its architecture-documentation omission was corrected and rechecked;
  the review closed with no outstanding findings.
- The dedicated native fixture passed separately on the connected physical
  outputs: real AX/frontmost readback corrected an injected stale visible
  event after quiet/tick; a hidden-workspace event preserved seats, the
  focused output, and native focus; selecting an empty output published its
  seat without changing the actual focused window. The conditional hardware
  branches ran without coverage-gap messages.
- These native checks inject notifications and actuate focus through the
  adapter. Actual mouse clicks and observer-delivery ordering remain
  unverified; native fullscreen remains the earlier documented fixture gap.
- One native run passed focus assertions but failed the existing geometry
  displacement injection afterward; an unchanged rerun passed the complete
  fixture. No production change was made for that intermittent AX write.
