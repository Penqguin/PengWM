# ADR-0005: Placement memory is recorded but never consumed at routing

Date: 2026-10-27
Status: Accepted

## Context

A new tiled window's workspace was decided by a precedence chain:
the seatbelt (same-window rediscovery), then the **placement memory** —
the last session's (bundle, title → workspace name) records projected
from `live_placement` into `state.toml` — then the **successor ask**
(a re-created WindowId with the same pid + identity inheriting the dead
original's home), then config routing (`apps = [...]`), then the active
workspace.

The memory paths exist to honor what the user *did*: a window manually
moved to workspace 3 last session re-homes there after a WM restart or
app relaunch. In practice the observable behavior is the opposite of
the intent: launching an app opens its new window on wherever the app
(or an identically-titled window) happened to live before — not where
the user is. A birth is a birth: the user launching iTerm2 while on
workspace 1 expects the window on workspace 1.

## Decision

New-window routing never consults remembered placement. The precedence
becomes:

1. **Seatbelt** (`live_placement`, pid-matched same WindowId) —
   rediscovery of an existing window only, not a birth decision.
2. **Config routing** (`routed_workspace_for` — what the app *is*).
3. **The active workspace** — the fallback, where the user is.

Concretely:

- `where_should` shrinks to the seatbelt check and returns `None` for
  any window it has no same-id record for.
- The `placement_memory` store and its hit-once consumption are
  removed, along with the successor ask (`successor_home`) and the
  identity-match contract it shared with the memory
  (`WindowPlacement::matches` / `session::identity_hit`).
- `restore_last_session = true` keeps restoring boot-time state:
  workspace topology, active seats, gaps. It no longer re-homes
  windows.
- The projection keeps **writing** placement records to `state.toml`
  (`capture_placements`/`save_session` unchanged). The format is
  stable — old files parse, new files carry the same shape — but
  nothing consumes the records at restore. This decouples what the
  daemon records from what it acts on, so a future consumer (or a
  revert) needs no re-history.

## Rejected alternative

A config flag (e.g. `remember_window_placements = false`) preserving
the old behavior was rejected: no second behavior is wanted, and a
flag nobody defaults off is dead config surface. `restore_last_session
= false` as the off-switch was also rejected — it disables layout and
focus restore, which remain wanted.

## Consequences

- An app relaunched while the user is on another workspace opens on
  the active workspace (or its config-routed workspace), not where it
  used to live. A window macOS recreates under a new WindowId likewise
  routes as a birth.
- Rediscovery is unaffected: the seatbelt still returns a window the
  store lost (listing misjudgment, minimize, wake resync) to where it
  lives *this session*.
- Placement records in `state.toml` are diagnostic data; a stale
  `state.toml` can never pull a new window across workspaces.

Refs: ADR-0003 (identity/membership split), `GLOSSARY` (Placement
seatbelt), `pengwm-daemon/src/state/placement.rs`.
