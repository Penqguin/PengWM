# Focus timing review

Date: 2026-11-12. Scope: the uncommitted focus refactor against
[Spec 0005](../specs/0005-focus-timing.md), compared with original focus
methods at `2fe81ddcb1c2d06ec8ee6847b02b697946aa9d50`. The earlier membership
refactor remains in the same worktree and was treated as an accepted
dependency, not as a new focus change.

## Independent results

| Lane | Verdict | Findings |
| --- | --- | --- |
| Code reviewer | APPROVE | 0 critical, high, medium, or low issues |
| Architect | CLEAR | No actionable architecture defects |
| Combined | APPROVE | No changes requested within the agreed scope |

The code reviewer examined 16 focus-scope files plus integration dependencies.
Both lanes explicitly included untracked modules, tests, and the spec rather
than relying on `git diff` alone. No custom code-review gates are configured.

## Evidence

- [focus.rs](../../pengwm-daemon/src/state/focus.rs) concentrates temporal
  state and executes acceptance/settling, preserving the 150 ms protection,
  250 ms quiet period, publication, parking, wake, and fallback checkpoints.
- The three temporal fields are private to the module. Workspace membership
  and window focus, DisplaySet output assignment, and WindowStore verdict/Park
  ownership remain with their established owners.
- Observations and both settled branches use the same explicit decision
  instant. Switch protection is installed at the existing command checkpoint.
- Characterization and exact-time tests cover observed memory before
  rejection, None-only fallback, Workspace focus before output protection,
  wake retention versus switch consumption, reset retention, activation
  without arming, and Native-Space bookkeeping.

## Fresh verification

The code reviewer ran the 15 focus tests and `cargo fmt --check`; both passed.
The parent also ran formatting and diff whitespace checks successfully.
The architecture lane performed a source review without executing tests.

Previously reported implementation evidence remains 501 passing workspace
tests, strict Clippy, and a separately passing native fixture. This review
did not rerun GUI tests or disturb the running daemon while the user tried it.

## Limits and architectural counterargument

The implementation remains a private module of StateManager with access to
its shared state. That matches the approved design: concentrating temporal
decisions and keeping their fields private provides depth without inventing
another coordinator or relocating domain ownership.

Native readback with injected notifications does not establish actual mouse
clicks or observer-delivery ordering. Native fullscreen also remains
unverified. These are disclosed validation limits, not confirmed defects or
architectural approval blockers; the approval does not claim those paths
were newly validated.
