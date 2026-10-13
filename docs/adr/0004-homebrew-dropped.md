# ADR-0004: Homebrew distribution dropped (deferred, not decided against)

Date: 2026-10-13
Status: Accepted (distribution flows are curl + in-place update only)
Defers: the brew half of ADR-0001's distribution plan ("the real Formula
returns").

## Context

ADR-0001 (bare-binary distribution) kept Homebrew as a secondary install
path: a real Formula on the `Penqguin/homebrew-tap` tap, plus
`.github/workflows/homebrew-bump.yml` — an ubuntu workflow that,
after every release, verified the `.sha256` sidecars and opened a PR
bumping `Formula/pengwm.rb`'s version and per-arch sha256 lines (PAT
secret `HOMEBREW_TAP_TOKEN`).

In practice that path never completed a cycle: the tap is still serving
v0.6.0 while v0.6.1 and v0.6.2 shipped through the curl one-liner, because

1. the `release.published` trigger never produced a bump run for either
   release, and
2. `HOMEBREW_TAP_TOKEN` was never configured on this repo, so every
   dispatch skipped cleanly at the guard by design.

With no formula even *arriving* on the tap, there are no `brew install`
users to serve; every would-be user was redirected to the installer. So
the tap bump automation was pure standing cost — a secret to mint and
rotate, a workflow to babysit, and a stale in-repo `Formula/` copy
muddying every release checklist — for zero observed demand.

## Decision

Drop Homebrew distribution for now:

- Delete `.github/workflows/homebrew-bump.yml` and the stale in-repo
  `Formula/pengwm.rb` (the tap repo keeps its old files; we simply stop
  bumping them).
- Distribution is the curl one-liner (`install.sh`), a checkout's
  `./install.sh`, and in-place updates via `pengwm update`.
- Pre-existing Homebrew Cellar installs stay grandfathered by the
  one-copy policy (`location.rs` still accepts Cellar paths) and remain
  updatable in place via `pengwm update` — nothing existing breaks.

## Consequences

- No `HOMEBREW_TAP_TOKEN` secret is needed or expected; a future review
  finding "bump automation missing, secret unconfigured" should read
  this ADR instead of proposing to re-add it.
- The tap's `Formula/pengwm.rb` stays at v0.6.0 forever unless the path
  is re-opened — a `brew install` copy would pin old binaries. Nothing
  points users at it anymore.
- Re-adding brew later means: restore a tap-bump workflow, set the PAT
  secret, and bump the formula across every release published in the
  interim (the sha256 sidecars of old tags are already published, so the
  formula can catch up in one PR).
