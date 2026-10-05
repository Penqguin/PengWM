# PengWM — Domain Glossary

## Layout

**LayoutPolicy** — The strategy used to arrange windows within a workspace.
- *Tiling* — Windows are arranged in splits according to the tree structure.
- *Magnify* — One pinned window overlays as a centered 75%x75% tmux-popup; tiling underneath stays computed but obscured.
- *Popups* — Workspace-bound overlay windows (dialogs, floating panels, restricted apps) that never join the tree; each renders as a magnify-style centered overlay.

**LayoutPreset** — One of five named tmux-style arrangements (`even-horizontal`, `even-vertical`, `main-horizontal`, `main-vertical`, `tiled`) applied via `Workspace::apply_preset`, which rewrites the tree into a canonical shape with weighted ratios. The `main-*` presets honor `main-ratio` (default 0.6).

**ResizePane** — Growing or shrinking the focused window by shifting the split ratios around it one step (5%, clamped at a 10% minimum pane share). Manual sizes survive structural edits; only a preset re-equalizes.
_Avoid_: pane (tmux word for window)

**Popup** — A window a Workspace tracks but never tiles. It renders as a centered overlay (magnify geometry, ratio from `popup_ratio`, default 0.75) on the workspace active on the monitor that contained it at creation. Sources: every Standard window of a `restricted_apps` bundle, plus — for any managed app — the `AXDialog`, `AXSystemDialog`, `AXFloatingWindow` subroles; `AXSheet` and unknown subroles stay dropped. Classified once at creation (first-classification-wins). Placed at creation and re-placed on wake; afterwards the user may drag it — nothing re-centers it. Hides with its workspace through the normal parking path and comes back on top on switch-back. Never counts against `max_tiles`, never participates in drag-swap, never swept as displaced, never persisted across sessions.

**WindowClass** — The typed classification of a window returned across the `OsAdapter` seam (`window_kind(window_id)`), replacing the binary manageable/dropped gate: discovery classifies instead of dropping, so tree routing, popup routing, and the background sweep share one answer. `Standard` tiles; `Dialog`/`SystemDialog`/`Floating` pop; `Sheet`/unknown drop as today.

**Workspace** — An independent window tree on a single monitor. The deepened interface exposes:
- `layout(gap_inner, gap_outer) -> HashMap<WindowId, Rect>` — single method that computes global-coordinate rects for every window. Uses stored monitor geometry internally. Tiling is always computed; when `magnified` is set the pinned window is overwritten with a centered 75% overlay; popup members are emitted into the same map without joining the tree.
- `apply_split_direction(direction)` — the split intent: re-orients the focused Split container, or — when a Window is focused — pends the direction for the next window added. The "only a Split container re-orients" invariant lives here with the tree.
- Hiding — no workspace method: `StateManager::hide_workspace` sends `all_owned()` (tree members + popups) to `OsAdapter::hide_windows` (batch offscreen). The workspace owns *which* windows; the adapter owns *how* to hide.
- Tree internals (`root`, `arena`, `monitor_origin`, `monitor_size`) are private — `focused_node` and `magnified` remain public for daemon integration tests.
- Implementation is split by responsibility (`workspace/preset.rs`, `add_remove.rs`, `focus_swap.rs`, `split.rs`, `geometry.rs`); the suite mirrors it (`workspace/tests/` + `common` harness). Fields stay on `Workspace`; the files only reorganize `impl` blocks.

## Platform Abstraction

**OsAdapter** — The trait seam between platform-independent state logic and macOS-specific FFI. Two implementations: `MacOsAdapter` (prod) and `TestAdapter` (tests). Narrowed into two traits — window/display query+actuation vs observer lifecycle:

```rust
pub trait ObserverRegistry { fn attach_observer(&mut self, pid: i32); fn detach_observer(&mut self, pid: i32); }
pub trait OsAdapter: ObserverRegistry {
    fn running_app_pids(&self) -> Vec<i32>;
    fn frontmost_pid(&self) -> Option<i32>;
    fn poll_windows_for_pid(&self, pid: i32) -> Vec<WindowId>;  // &self interior mut (cache insert); was &mut
    fn focused_window_for_pid(&self, pid: i32) -> Option<WindowId>;
    fn active_displays(&self) -> Vec<DisplayInfo>;
    fn primary_display_id(&self) -> u32;
    fn set_window_rect(&self, window_id: WindowId, rect: Rect) -> WriteOutcome;  // &self interior (WindowElementCache); position/size/position x3 with readback; typed outcome, never string-matched
    fn window_rect(&self, window_id: WindowId) -> Option<Rect>;  // readback seam for verify-and-retry + tests
    fn window_kind(&self, window_id: WindowId) -> Option<WindowClass>;  // typed classification; replaces the binary is_manageable gate
    fn raise_window(&self, window_id: WindowId);  // AXRaise; popups come back on top on switch-back reveal
    fn close_window(&self, window_id: WindowId);
    fn hide_windows(&self, placements: &HashMap<WindowId, HidePlacement>); // HidePlacement::BottomEdge vs FarOffscreen; no magic threshold; position-only, never resizes (no Firefox reflow)
    fn window_is_hidden(&self, window_id: WindowId) -> bool;  // kAXMinimized/kAXHidden; drives reconcile
    fn app_bundle_id(&self, pid: i32) -> Option<String>;
    fn app_name(&self, pid: i32) -> Option<String>;
    // test vocabulary is inherent to TestAdapter/TestHandle, never on the trait:
    // inject_window / inject_app_name / inject_bundle_id + set_fault / clear_fault / displace / writes / rect.
    // Tests hold a TestHandle sharing the boxed adapter's cells (common::setup_with_handle).
}
```

`MacOsAdapter::with_callback(callback: Box<dyn Fn(DaemonEvent) + Send>) -> Self` is an inherent constructor (not on the trait) and the observer callback is `Box<dyn Fn(DaemonEvent) + Send>` not a hard-coded mpsc sender. No `as_any_mut` — tests use `OsAdapter::inject_*` instead of downcasting to `TestAdapter`.

Hidden/minimized windows are detected two ways: per-window `kAXWindowMiniaturizedNotification` / app-level `kAXApplicationHiddenNotification` fire immediately, and a ~1s `on_tick` reconcile queries `window_is_hidden` per tracked window as a fallback for missed notifications. Both untile the window (like a close) while keeping pid tracking so `on_window_shown` can retile it where it came from.

**WriteOutcome** — The typed result of a layout write across the `OsAdapter` seam (`pengwm-core::layout::write`). Five variants: `Ok` (at target, or unreadable mid-write — the next sweep heals), `Pinned { target, actual }` (readbacks stopped moving; the cache backs off — see **PinBackoff**), `Drift { target, actual }` (accepted but never converged; retry, never record success), `Gone` (refresh + discover both missed; the caller applies gone-grace only, no second poll), `Transient(String)` (live-resize contention; the message is log payload, never a match key). The writer classifies; `layout_cache` matches.

**WindowStore** — The single owner of wid↔pid↔hidden state owned by `StateManager`. Owns the pid maps plus the hidden map (`HashMap<WindowId, usize>` remembering the workspace index each hidden/minimized window came from) and `last_reconcile`. Exposes `hide` / `reveal` / `reveal_all` / `is_hidden` / `pending_for_reconcile` / `should_reconcile` — the seam runs between state and trees: `hide` finds + remembers the workspace index, the caller removes from the tree and re-layouts; `reveal_all` drains remembered `(window, index)` pairs, the caller routes + retiles. Reconcile is testable via a `Fn(WindowId)->bool` predicate without `as_any_mut` downcast to `TestAdapter`.

**DragState** — The drag-to-swap gesture state owned by `StateManager` (`drag_window`, `overlap_target`, `overlap_start`, `last_move`). `on_moved` calls `layout::window_at_point` to update the overlap target; `on_tick` returns `DragTickAction::Swap | SnapBack | None` so `StateManager` owns `apply_layout` and workspace mutation. Keeps `last_layout_rects` borrowed from `StateManager` to avoid duplicating the rect map.

**DisplaySet** — The global Workspace pool + output assignment owned by `StateManager` (`active: BTreeMap<u32, usize>` + `entries: Vec<WorkspaceEntry>` + `max_tiles` + `focused_output`). One tree per config name globally (5 entries = 5 Workspaces, i3-style); each Workspace is assigned to exactly one output and each output shows exactly one Workspace. `BTreeMap` makes iteration deterministic (no `HashMap` random fallback in `active_workspace_idx`). Switching to a Workspace visible elsewhere **swaps** (the other output falls back to the Workspace just left); `monitor` affinity is an initial-output hint only. Addressing is global 1-based config order (`workspace-1` = first entry, any output); app routing lands in the global name-tree with no auto-pull and overflow spills to the next global Workspace with room (moves bypass the cap — #2). Orphans on monitor-remove go to primary; a newly added output shows the first hidden Workspace else pulls the first (swap). `focused_output` is an explicit cell (pid heuristic is fallback only) so pull/swap always knows "here". Owns `active` (which global Workspace is visible per output), the named-entry set, and the routing policy (`next_with_room`, `target_with_room`, `routed_workspace_idx`, `configured_workspace_name_for_pid`, `active_workspace_idx`, `display_in_direction`, `resolve_workspace` (global), `workspaces_on`, `is_visible`, `visible_or_first`) so output resolution, visibility and overflow have locality in one module; `Vec<Workspace>` stays on `StateManager` and is borrowed per call. Move decisions are answered, not re-branched: `plan_move(from, target)` (capacity + overflow redirect → `MoveDecision`) and `direction_target(current, direction)` (display hop + visible-or-first) feed the move commands, which only execute via a shared move helper. Switch decisions are answered the same way: `plan_switch(target, focus_first)` (pull/swap → `SwitchDecision{show, hide, relayout, focus}`, MRU by default, spatial-first when the flag is set, `None` when empty) feeds the switch command, which only records focus, re-layouts, then raises. Focus onto an empty output is bookkeeping-only (always succeeds); focus stays on move (#2). Exposes `init_workspaces`, `on_added`, `on_removed`, `on_resized` as one answer/execute sync funnel (`{shown, hidden, relayout}`, caller does hide/layout/bar — #3); session persists at quit only. `capacity::next_with_room` deleted — now `DisplaySet::next_with_room` with `max_tiles` stored on the set. `StateManager` keeps one narrow `active_workspace_idx()` accessor (19 call sites); the other former delegates are deleted. `bootstrap::assemble(display_infos, primary_id, settings, session)` is the single pure assembly for workspaces/displays/gaps (Q2). Bar stays primary-only with per-output active markers; `autostart` runs once per global Workspace regardless of hint (#4).

**LayoutWriter** — The deepened layout-write module (`state/layout_writer.rs`) owning the `LayoutWriteCache` maps plus the two funnels that reason about them: `plan_writes` decides a layout pass, `sweep_displaced` decides the misplaced tick. Callers feed one cheap read per window and execute what comes back; no caller threads `window_rect` through `should_write` / `is_displaced` by hand. `StateManager::apply_layout` (in `state/mod.rs`) is thin orchestration: compute targets, read, execute plan, untrack the proven-gone via the normal destroyed path. Policy tests target the funnels directly with no harness.

**StateManager layout** — `pengwm-daemon/src/state/mod.rs` holds construction, config reload, hide/reveal, layout application, and menubar publish; `lifecycle.rs` owns the window/app lifecycle, `reconcile.rs` the tick path, `monitors_wake.rs` monitor handling and wake resync; `commands.rs` owns the `Command` dispatch (`impl StateManager`, test-called helpers are `pub(super)`); `layout_writer.rs` owns the `LayoutWriteCache` module — the four write-policy maps (`applied_rects`, `gone_since`, `pin_state`, `layout_fail_logged`) plus the funnel (`should_write` / `record_success` / `record_failure` → `AfterWrite::Keep|Untrack` / `invalidate` / `forget` / `clear_on_wake` / `seed_hidden` / `note_displaced` / `is_displaced` / `pin_backoff_active`). `StateManager` keeps one field and never touches the maps; destroy/terminate/wake collapse to one `forget` / `clear_on_wake` each; policy tests target the cache directly with no harness; `tests/` holds the suite by domain (`common` harness + `bootstrap_routing`, `lifecycle`, `commands`, `layout_cache_hide`, `wake`). Fields stay on `StateManager`; the other files only reorganize `impl` blocks.

**PinBackoff** — After `PIN_STRIKES` (3) consecutive `Pinned` writes at the same target, `LayoutWriteCache` skips that window's writes for `PIN_BACKOFF` (15s) and retries on the timer. The invariant that makes it work: **`invalidate` clears `applied_rects` but never `pin_state`.** Both callers (`note_displaced`, the misplaced sweep) are observing "not where we put it", which is the *expected* state of a pinned window — clearing the strikes there reset the count to 1 on every 2s tick, so `PIN_STRIKES` was unreachable and a busy app (Firefox after wake) took a full 3-attempt rewrite every 2s indefinitely. The pin is dropped only where it genuinely goes stale: changed target (`should_write`), landed write (`record_success`), destroy/terminate (`forget`), wake (`clear_on_wake`). `reconcile_misplaced_windows` consults `pin_backoff_active` *before* its AX read, so a backed-off window stops re-triggering a whole-workspace re-layout. Regression tests: `layout_cache::tests::invalidate_keeps_pin_evidence` (policy) and `state::tests::wake::pin_backoff_survives_the_misplaced_sweep` (tick path).

**WakeResync** — `DaemonEvent::SystemWoke` **arms** a resync; it does not run one. `NSWorkspaceDidWake` fires while the AX subsystem is still blacked out: `kAXWindows` comes back empty for live apps and every cached element is stale. Resyncing inline was worse than doing nothing — the polls meant to refresh `WindowElementCache` returned nothing so the stale refs survived, and the layout writes that followed all failed `kAXErrorInvalidUIElement`, which `refresh_element` could not heal either, so every live window reported `Gone` and started a 10s death timer. `on_tick` drives `drive_wake_resync`, which re-probes every `WAKE_PROBE_INTERVAL` (500ms) and commits only when a poll actually returns windows — the poll result *is* the AX liveness probe, and committing behind it is the whole mechanism. `WAKE_DEADLINE` (20s) forces a commit so a probe that can never succeed doesn't leave the resync armed forever. Both `DidWake` and `ScreensDidWake` fire per wake; the first arm wins. Tests: `wake_resync_waits_for_ax_instead_of_writing_through_stale_elements`, `wake_resync_commits_at_the_deadline_even_if_ax_never_answers`, `double_wake_notification_arms_once`; `TestAdapter::set_ax_blackout` simulates the blackout (empty polls, unreadable rects, windows still alive).

**Gone grace** — `gone_since` records the *first* miss, so it must be reset by any outcome proving the element answered (`Ok`, `Pinned`, `Drift` — the latter two carry a readback). Without that reset a window that missed once and then went minutes without a write (skipped because it was at target) was untracked by the very next single miss. `GONE_GRACE` is **10s by design** and must stay there: a v0.5.1 misfire ("layout tracking delay") shrank it to 1s, so any transient AX miss (post-wake blackout, busy Chromium/Gecko app) permanently untracked live windows — they fell out of tiling with no notification. Real closes never wait on the grace: they arrive via the destroyed notification immediately. Test: `live_readback_clears_a_stale_gone_grace`.

**AX messaging timeout** — Every AX call is a synchronous mach RPC into the target app's main thread, issued from the one thread that also pumps the CGEventTap (keybinds) and the AX observers. `ax_element::MESSAGING_TIMEOUT_SECS` (0.25) bounds the stall: `set_global_messaging_timeout` sets the process-wide default at startup, and `create_app_element` / `apply_messaging_timeout` apply it per element (app elements and the window elements from `kAXWindows` are separate refs). Every `AXUIElementCreateApplication` goes through `create_app_element` so no call site inherits the system default. A timed-out request surfaces as `kAXErrorCannotComplete` → `Transient` → retry on the next layout or sweep.

**WindowElementCache** — A `HashMap<WindowId, AXUIElementRef>` owned by the unified macOS adapter. Populated on `kAXWindowCreatedNotification` (caller does `CFRetain`), evicted on `kAXUIElementDestroyedNotification` (caller does `CFRelease`). Makes `set_window_rect` O(1) instead of O(n) and seals CFRef memory lifecycle. Maintains a reverse `WindowId → i32` pid map so `set_window_rect` and `close_window` do not require a pid parameter from callers. Every write runs one funnel — `resolve_element` (cache, else discover) → attempt → `refresh_after_stale` (re-poll, re-cache, re-register, else evict) → `evict_element`; hide stays position-only but shares the funnel, so only the attempt op differs. Resynced on `DaemonEvent::SystemWoke` (`NSWorkspaceDidWake`/`ScreensDidWake`): `StateManager::on_system_woke` refreshes display geometry, clears `applied_rects`, re-attaches observers + re-polls all pids (which re-inserts fresh refs), then re-layouts visible workspaces.

## Architecture Boundaries

**Pure/dirty split** — `pengwm-core` is pure Rust, no macOS deps, testable on any platform. `pengwm-daemon` holds all macOS FFI. The layout pipeline crosses this boundary: `Workspace.layout()` produces global-coordinate rects so `StateManager` can blindly pass them to `OsAdapter::set_window_rect` without monitor math. Drag-overlap hit-testing is the same shape: `layout::window_at_point(rects, x, y, exclude)` answers "which other window is under this point" so `StateManager` never does rect containment itself.

## Shared Daemon↔Menubar Contract

One definition, one consumer. The daemon spawns `pengwm-menubar` (NSStatusItem,
excluded from tiling by pid) and pushes the UI state to it:

- **`ipc::send_command`** — one command-socket client shared by the `pengwm` CLI
  and the menubar's workspace/quit menu items; `ipc::COMMAND_SOCKET_PATH` /
  `ipc::BAR_SOCKET_PATH` are the single socket-path constants (the
  `pengwm-bar.sock` name is legacy wire format). The daemon re-exports them
  (`ipc_server::DEFAULT_SOCKET_PATH`, `bar_server::BAR_SOCKET_PATH`). The
  client **bounds its wait for the daemon's reply (3s read timeout)** so a
  wedged daemon cannot freeze the menubar's main thread mid-Quit; a stalled
  reply surfaces as an `Err`, and the menubar's Quit falls back to
  bootout + killall so the whole app still dies.
- **`launchd.rs`** — LaunchAgent awareness on the daemon side. On current
  macOS `KeepAlive { SuccessfulExit = false }` respawns the job even after a
  clean exit 0 (observed live), so "quit stays quit" cannot live in the
  plist: the daemon deregisters the job (`bootout`) on every clean shutdown
  (`pengwm quit`, menubar Quit, SIGTERM via the ctrlc handler). `RunAtLoad`
  re-registers at the next login.
- **`BarMessage`** — `State(BarState)` (workspace names/window counts/active
  marker + split direction) and `Exit`. The push server (`bar_server.rs`)
  caches the last `State` and replays it to a reconnecting menubar; `Exit`
  tells the menubar to terminate immediately on a clean `pengwm quit`.
- The status bar (egui strip + `[bar]` config + `BarReserve` reservation
  machinery) was **removed** in v0.5.2 — the menubar icon is the only UI
  surface. The core keeps `BarPosition` / `bar_strip_rect` /
  `Workspace::set_reserved_rect` as tested pure vocabulary so a future
  screen-strip can be re-introduced without reinventing the geometry.

## Command Vocabulary

One `Command` type is the single vocabulary every surface feeds into `StateManager::on_command`:

- **`command::Command::parse_action(s)`** — the one action-string parser (kebab-case of the variant + args: `move-window-left`, `cycle-layout`, `workspace-3`). Lives in `pengwm-core` with the wire type so the keybind TOML surface can never drift from it. The keybind store calls it directly (no passthrough) and fails the whole load on unknown actions — `load_from` warns and falls back to defaults, same as the IPC loader path.
- **CLI** — clap subcommands map onto the same `Command` (`move-window left` → `Command::MoveWindow`). Names line up with the keybind strings (`swap-*` is gone).
- **Reply slot** — `DaemonEvent::Command(cmd, Option<Sender<DaemonResponse>>)`. `Some` only for the IPC client; keybinds and the config watcher send `None` and get no reply, so no throwaway response channel is allocated. `on_command` acks only when a slot is present.
- **PrefixKey** — The chord (default `alt-space`) that arms a ~1s window where the full `Command` vocabulary is reachable from short follow-ups; repeatable commands (resize, focus) fire again without re-arming. Purely additive — direct binds keep working.
