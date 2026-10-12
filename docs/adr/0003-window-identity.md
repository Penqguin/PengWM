# ADR-0003: Window identity lives in the WindowStore; membership lives in the trees

Date: 2026-10-11
Status: Accepted

## Context

Understanding "what is this window to me?" used to require bouncing
between four owners: tree membership (Workspace), parking
(WindowStore), window-class classification (the adapter's element
cache), and the `float_apps` / level-gate routing verdict on nothing at
all. Worse, the verdict was not durable: the element cache evicts on
stale elements and the fallback discovery re-classifies from scratch
(`adapter_macos::discover_window`), so a `float_apps` window could
silently read `Standard` again and start being managed by the WM that
ADR-0002 promised never to touch it.

An architecture review proposed deepening WindowStore into a *full*
per-window record: membership kind (tree/popup/untracked) + park state +
pid + verdict, all queried through one ask() interface.

## Decision

Split truth by kind of question — three sources, each one owner:

1. **Membership** — *where a window is tiled or popup-attached* — stays
   single-sourced on the trees (`Workspace::find_window` / `is_popup`).
   One spelling to ask across the pool: `pengwm_core::workspace`
   `position_of_window` / `position_of_member`.
2. **Identity** — *what kind of member-or-overlay a tracked window is* —
   is a durable **routing verdict** in `WindowStore`
   (`RoutingVerdict::Tiled | Popup | Untracked`), recorded once at
   routing (`note_routed`, first-call-wins), cleared on unregister and
   on re-register (a re-created window id re-classifies). Ask the store
   (`routing_verdict` / `is_untracked`); never re-derive from
   `window_kind`.
3. **Classification** — *what macOS role/subrole level say* — stays at
   the adapter's discovery gate (`WindowClass`), cached on the element;
   the store's verdict is its *durable shadow*, not a second classifier.

Unverifiable-semantics decisions that hang off this:

- **`None` (never routed)** is not an error: windows registered but
  never routed (unmanageable classes, excluded-at-routing) proceed
  exactly as they did pre-refactor — smallest interface, no `Dropped`
  variant unless a third consumer needs the distinction.
- **Births only**: the verdict is frozen at creation, matching
  ADR-0002's first-classification-wins and "config checked at creation
  only". Config reload changes births, not incumbents — a live window
  is never surprise-promoted into management mid-session.
- **The verdict-tree invariant is enforced in tests**: unparked
  `Popup` ⊂ some workspace's popup set and never a tree member;
  `Untracked` never a member and never parked; unparked `Tiled` never
  in a popup set. (The positive "Tiled ⇒ tree member" side is
  unchecked: capacity-overflow windows are legitimately tracked-but-
  untiled between layouts.) Enforced in the test harness after every
  identity-mutating funnel.

## Rejected alternative

The full per-window record with membership duplicated into the store
was rejected: it creates a second source of truth that every tree
mutation site (add/remove/park/reveal/swap — five call surfaces) must
remember to keep in sync, which is the exact bug shape
`Workspace::reseat` made unrepresentable for monitor seats (ADR-notes
in candidate 5). Membership has no evictable-state problem: the trees
*are* the authority, and scanning them is cheap at our scale. The
verdict, by contrast, has no other authority once the element cache
evicts — that asymmetry is what justifies the asymmetry of ownership.

## Consequences

- Adding a new window-management funnel: classify at discovery, route
  once (note the verdict), trust the store afterwards.
- Adding a new *question* about identity: ask `routing_verdict`; only
  add to this interface when a second consumer exists.
- Membership scans that remain (popup-set snapshots in the misplaced
  sweep; `is_popup_anywhere` in parking) are deliberate — the workspace
  scan is doing membership work, not identity work. Migration to the
  verdict happens opportunistically when a scan is next touched, not
  wholesale.
- Future architecture scans proposing a "single window record" should
  read this ADR first: the narrow verdict + tree membership split is
  load-bearing.

Refs: ADR-0002 (untracked windows; first-classification-wins),
GLOSSARY (`RoutingVerdict`), `pengwm-daemon/src/state/store.rs`.
