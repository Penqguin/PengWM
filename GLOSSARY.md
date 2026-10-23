# Glossary

Domain language for PengWM. Terms here name the seams and the concepts
behind them; code and docs should use these words exactly. Architecture
vocabulary (module, interface, seam, adapter, depth, leverage, locality)
comes from the design skill, not this file.

- **Routing verdict** — The store's durable answer to "what is this window
  to me": `Tiled`, `Popup`, or `Untracked`. Recorded once at routing in
  `WindowStore` (`note_routed`, first-call-wins) so window identity
  survives the evictable element cache and its fallback
  re-classification. Ask the store, never re-derive from `window_kind`.
- **Placement memory** — The session's durable "where did this window
  live" record: bundle + title → workspace **name** (per window,
  captured at quit, `state::placement`). Consumed through one ask at
  routing, hit-once; it outranks config routing because it is what the
  user *did*, not what the app *is*. Restore input only — membership
  stays single-sourced in the trees (ADR-0003).
- **Placement seatbelt** — The in-session, continuously maintained
  placement record (`state::placement`, `note_where`/`where_should`):
  window → (pid, workspace index, bundle, title), noted at every
  placement change including park time. Consulted at rediscovery before
  the durable memory (a pid match admits the recall), and projected
  into the session snapshot — so placement survives sleep, crashes,
  and the listing-diff misjudgments the hidden-member shield can't
  cover. Also restore input only — never a second membership owner
  (ADR-0003). A WindowId macOS reissues mid-session (window re-created,
  app relaunch) resolves through the **successor ask**: a new id with
  the same pid + identity while the recorded id no longer lists
  inherits the home — exactly-one candidate admits the transfer, and
  only when the successor itself lists.
- **Hidden-member shield** — The listing diff never judges a window
  that is a member of a hidden workspace: macOS omits off-Space windows
  from listings and only listed windows get their element refs
  re-polled, so both discriminators can misread a live hidden member as
  dead (the wake repro). Tree membership *is* the evidence it is
  alive; its death verdict defers to reveal time, where the write
  path's gone grace owns cleanup.
- **Park** — A tracked window that left the tiling tree but waits to come
  back: `ParkReason::Hide` (minimize / app-hide) or `ParkReason::Fullscreen`
  (native fullscreen). One parking map, one reveal funnel.
- **Popup** — A workspace-bound overlay window (dialog / floating panel /
  restricted app): tracked by its Workspace, never tiled, never counted
  against `max_tiles`.
- **Untracked** — A cross-workspace overlay (ADR-0002): store-tracked but
  tree-less; never tiled, never parked, re-raised after every visibility
  change. Reached by the CG window-level gate or the `float_apps` list.
- **Workspace** — The container for one named workspace's tree, popup set
  and monitor seat. Trees hold tiles; popups live on the Workspace too.
- **DisplaySet** — The global workspace pool + per-output assignment
  (which Workspace is visible on which monitor) and the routing policy
  that answers move/switch/topology decisions.
- **LayoutWriter** — The module owning the layout-write policy maps
  (skip-if-unchanged, read-before-write, gone grace, pin backoff) behind
  `plan_writes` / `record_failure` → `AfterWrite`.
- **Listing** — An app's answer to the window poll (`WindowListing` at the
  adapter seam): `Answered` (AX gave the app's window list, possibly
  empty — a closed-only-window app's "zero windows" verdict) or
  `NoAnswer` (AX could not answer at all). Judgment sweeps ask
  `listing_evidence`: an answered listing is admissible death evidence
  **only for windows whose element no longer answers** — macOS Space
  transitions make apps answer without their off-Space windows (live
  tiled members included), so a live element outranks the omission; a
  dead one (unreadable rect) is judged dead on it. `NoAnswer` is never
  judged on.
- **AX blackout** — The post-wake window where AX answers nothing
  (`NoAnswer` listings, unreadable attributes). No structural judgment
  may run during a blackout; see the wake resync machinery. Distinct
  from an answered-empty listing, which IS judgment evidence.
- **Skip** — The menubar's persisted "Not now" verdict for one release
  tag, recorded in `$PENGWM_HOME/skip-update`. Silences the update ask
  for that tag only; a different, newer tag re-arms it. Explicit commands
  (`pengwm update`) never consult it.
