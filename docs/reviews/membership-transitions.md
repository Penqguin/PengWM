# Membership transitions review

Date: 2026-11-12. Scope: uncommitted membership refactor against
`2fe81ddcb1c2d06ec8ee6847b02b697946aa9d50`, including the new membership
module, characterization tests, and design spec that `git diff HEAD` alone
does not include. Twelve changed paths were reviewed. Requirements:
[membership transitions spec](../specs/0004-membership-transitions.md).

## Independent results

| Lane | Verdict | Actionable findings |
| --- | --- | --- |
| Code reviewer | APPROVE | 0 critical, high, medium, or low |
| Architect | WATCH | No architecture blocker; native fullscreen validation remains open |
| Combined | COMMENT | No fixes requested; retain the validation watch |

Both lanes independently read the diff and untracked files. No custom
code-review quality gates are configured (`omx quality-gates list --stage
code-review --json` returned an empty gate list).

## Evidence

- Shared pending-transfer, capture-before-parking, remembered-return, and
  removal implementation concentrates ordering knowledge in
  [membership.rs](../../pengwm-daemon/src/state/membership.rs).
- Workspace remains the membership owner; WindowStore retains verdict
  and Park ownership. Routing and restart identity verification remain
  outside the transition implementation, consistent with ADR-0003.
- The review found transfer intent/order, hidden-sibling promotion,
  gated topology recovery, restart priority, RevealAll sequencing,
  layout re-entry, and caller focus/publication checkpoints preserved.
- The native test helper retains the element across minimize, requires
  successful attribute reads, and verifies actual rectangles in
  [window_placement_live.rs](../../pengwm-daemon/tests/window_placement_live.rs).

## Verification and limits

The code reviewer freshly ran all 370 daemon library tests, the four
membership characterization tests, formatting, diff whitespace checks,
and strict workspace/all-target Clippy; all passed. The architect performed
a read-only source review and did not independently rerun tests.

The earlier implementation run passed all 490 workspace tests and the
dedicated native fixture for moves, overflow, minimize/return, recovery,
and drift repair. These GUI results were not rerun during this review.
Native fullscreen return remains unverified because the fixture's floating
windows did not enter fullscreen. Deterministic fullscreen regressions pass;
this is a validation gap, not a confirmed defect.

## Architectural counterargument

The private StateManager module concentrates implementation knowledge but
does not prevent sibling modules from bypassing it. Carry and restart still
retain caller sequencing for switch placement and final recovery layout.
Those constraints match the approved synchronous, behavior-preserving
design. The review does not recommend adding a state-owning object,
scheduler, or generic transition framework without evidence of need.
