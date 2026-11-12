# Deepen membership transitions

Status: Implemented and validated; native fullscreen validation remains open.

## Settled scope

- Preserve current window behavior. Any discovered behavior defect needs
  a separate decision rather than an incidental refactor fix.
- Cover the full membership lifecycle: admission, transfer, parking,
  return, and removal. Implement incrementally, starting with transfers.
- Preserve Workspace membership authority and WindowStore routing verdict
  and Park authority (ADR-0003); introduce no second membership record.
- Preserve ADR-0006 topology semantics, ADR-0007 Native-Space gating,
  and ADR-0008 placement intent and admission priority.
- The transition module executes transitions rather than returning plans
  for each caller to execute. It owns membership changes, parking,
  seatbelt updates, cache invalidation, deferred hides, and resulting
  placement. Callers retain intent/destination selection, command-specific
  focus, and menubar publication.
- Place the implementation in a private membership module within
  StateManager. Reuse the existing OsAdapter seam; introduce neither a
  separate state-owning object nor a new adapter trait.
- Execute synchronously and preserve existing ordering, including slot
  promotion and layout-triggered removal. Do not introduce a transition
  queue or batch/reorder writes as part of this refactor.

## Existing constraints verified in code

- Capture the placement seatbelt and Park state before removing membership:
  both read current ownership and pending intent.
- Park hidden pending siblings before filling freed slots; app-hide
  notifications arrive one window at a time.
- Explicit workspace/display transfers establish Explicit intent; overflow
  carry and directional pending transfers preserve kind and opening order.
- Detached-display recovery moves Traveling pending and parked overflow;
  its bookkeeping remains permitted when the destination display is gated.
- Confirmed destruction and indirect unlisting differ in whether the
  placement seatbelt remains trusted for rediscovery.
- Layout application currently fills slots, so it can change membership.
- Restart recovery and RevealAll also admit windows; retain their ordering
  when consolidating the lifecycle.

Evidence: `state/membership.rs` (formerly `state/overflow.rs`),
`state/commands.rs`, `state/lifecycle.rs`,
`state/placement.rs`, `state/store.rs`, `state/live_restore.rs`, and
`state/mod.rs` in `pengwm-daemon/src/`.

## Transition interface

Use intent-based operations in `state/membership.rs`, implemented on
StateManager. These are responsibility families, not a requirement to
introduce a generic transition enum or one method per row:

| Operation | Caller supplies | Module completes |
| --- | --- | --- |
| Admission | Window, resolved workspace, placement intent | Membership attachment, eligible slot fill, seatbelt/cache updates, placement |
| Transfer | Window or traveling set, destination, cause | Detachment and attachment, intent preservation/change, bookkeeping, existing placement sequence |
| Park | Window, Hide or Fullscreen cause | Eligibility, capture before removal, Park state, source placement |
| Return | Window, existing recovery context where needed | Consume Park state, restore membership/intent, admission and placement |
| Removal | Window, confirmed destruction or indirect unlisting | Membership removal, existing promotion/placement sequence, store and element cleanup, seatbelt trust handling |

Callers select destinations using existing routing and DisplaySet policy.
The implementation may reuse those existing policy queries where return
already resolves a missing or invalid remembered destination. Classification
and the durable routing verdict retain their current owners.

Internal helpers hide repeated bookkeeping. They do not expose a sequence
of detach/update/invalidate/place steps for every caller to assemble.
Operation causes must preserve the existing differences between explicit
moves, overflow carry, topology recovery, and confirmed/indirect removal.
Reuse existing domain types; introduce a new type only where needed to
express one of these distinctions safely.

## Sequencing and integration

- Keep command-specific focus selection/requests and menubar publication
  with callers, preserving their current position relative to placement.
- Preserve existing synchronous mutation/write checkpoints. Do not claim
  transactional or atomic completion across macOS writes.
- Keep slot admission and layout-triggered cleanup within the transition
  implementation's coordination; preserve their existing re-entry behavior
  rather than adding a queue, scheduler, or transactional framework.
- Preserve startup recovery ordering and RevealAll semantics. Consolidating
  their membership mutations does not change identity verification or
  session serialization.
- Continue to use the existing hidden-placement and LayoutWriter modules;
  the membership module owns when to use them, not their implementation.

## Test surface and validation

Retain event/command integration regressions. Add focused tests through
the transition interface with TestAdapter only where coverage is missing.
Assert membership, placement intent, Park state, and observed adapter
effects rather than private helper calls or redundant implementation steps.

Required scenarios include transfer/parking intent preservation, app-hide
sibling ordering before promotion, prohibited writes on gated displays,
layout-triggered removal and promotion, confirmed versus indirect removal,
topology recovery, restart priority, and RevealAll admission. Existing tests
already cover many of these; do not duplicate them to match the new file.

Run targeted tests after each migration slice, then `cargo test`,
`cargo fmt --check`, and `cargo clippy --workspace --all-targets -- -D warnings`.
Validate real window behavior on macOS for explicit moves, overflow carry,
hide/return, and fullscreen return. Record any validation that cannot run;
a successful build does not establish native behavior.

## Cleanup plan

1. Map the required scenarios to existing coverage and add any missing
   characterization tests before changing their implementation.
2. Introduce the private membership module and consolidate transfers,
   preserving explicit move, carry, directional move, and topology recovery
   differences. Remove superseded bookkeeping at callers.
3. Consolidate admission and parking/return, including hidden siblings,
   RevealAll, and verified restart recovery.
4. Consolidate removal, preserving cleanup order and indirect-unlisting
   seatbelt trust; audit remaining membership mutation call sites.
5. Independently review the diff against this design and the accepted ADRs,
   then complete workspace checks and native validation.

No dependencies, configuration changes, public commands, new state owners,
or new adapter traits are required. No new ADR is needed for the agreed
implementation placement; this spec records the refactor decisions.

## Implementation record

- `state/membership.rs` replaces `state/overflow.rs` and consolidates
  transfer, admission, parking/return, verified restart admission, and
  removal. Shared internal helpers own pending transfers, member detachment,
  capture-before-parking, popup placement, and remembered tree admission.
- Lifecycle, command, and recovery callers retain routing/verification,
  destination selection, focus, and publication responsibilities.
- Characterization tests were added and passed before production edits
  for explicit transfer/source promotion, gated detached recovery, and
  layout-triggered removal. Direct transition tests also cover captured
  pending intent across Park/return.
- Independent read-only review found no correctness blockers, including
  a second pass over shared detachment and the removal checkpoint.

## Validation results

- `cargo test --quiet`: 490 passed, 4 ignored. The workspace suite was
  run outside the sandbox because Unix socket binds are prohibited inside it.
- `cargo fmt --check`, `git diff --check`, and
  `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- The ignored `native_overflow_carries_and_settles` fixture was run
  separately and passed against MacOsAdapter: overflow carry/settling,
  verified restart recovery, explicit transfer into a full workspace,
  minimize/return with native rectangle restoration, and drift repair.
- The fixture retains its Accessibility element across minimize and
  requires a successful attribute read to confirm restoration; a missing
  listing entry or failed read is not treated as a false attribute value.
- Native fullscreen entry was attempted, but the fixture's floating windows
  did not enter fullscreen after the accepted attribute write. Native
  fullscreen return remains unverified; existing deterministic fullscreen
  regression tests pass. The unsupported fixture assertion was removed;
  no production behavior was changed to accommodate the test.
