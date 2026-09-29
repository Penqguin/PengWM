# PengWM — Domain Glossary

## Layout

**LayoutPolicy** — The strategy used to arrange windows within a workspace.
- *Tiling* — Windows are arranged in splits according to the tree structure.
- *Monocle* — Only the focused window is shown fullscreen (minus gaps); all others are positioned off-screen.

**LayoutPreset** — One of five named tmux-style arrangements (`even-horizontal`, `even-vertical`, `main-horizontal`, `main-vertical`, `tiled`) applied via `Workspace::apply_preset`, which rewrites the tree into a canonical shape with weighted ratios. The `main-*` presets honor `main-ratio` (default 0.6).

**ResizePane** — Growing or shrinking the focused window by shifting the split ratios around it one step (5%, clamped at a 10% minimum pane share). Manual sizes survive structural edits; only a preset re-equalizes.
_Avoid_: pane (tmux word for window)

**Workspace** — An independent window tree on a single monitor. The deepened interface exposes:
- `layout(gap_inner, gap_outer) -> HashMap<WindowId, Rect>` — single method that computes global-coordinate rects for every window. Uses stored monitor geometry internally. Checks `monocle` flag; if set, produces one fullscreen rect + offscreen rects for siblings.
- `apply_split_direction(direction)` — the split intent: re-orients the focused Split container, or — when a Window is focused — pends the direction for the next window added. The "only a Split container re-orients" invariant lives here with the tree.
- Hiding — no workspace method: `StateManager::hide_workspace` sends `all_windows()` to `OsAdapter::hide_windows` (batch offscreen). The workspace owns *which* windows; the adapter owns *how* to hide.
- Tree internals (`root`, `arena`, `monitor_origin`, `monitor_size`) are private — `focused_node` and `monocle` remain public for daemon integration tests.
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

**WindowStore** — The single owner of wid↔pid↔hidden state owned by `StateManager`. Owns the pid maps plus five hidden methods (`hide` / `reveal` / `reveal_all` / `is_hidden` / `pending_for_reconcile` / `should_reconcile`) — the seam runs between state and trees: `hide` finds + remembers the workspace index, the caller removes from the tree and re-layouts; `reveal_all` drains remembered `(window, index)` pairs, the caller routes + retiles. `HiddenTracker` is private to the store; the predicate seam (`Fn(WindowId)->bool`) stays.

**HiddenTracker** — Private detail of `WindowStore` that remembers where each hidden/minimized window came from (`HashMap<WindowId, usize>` + `last_reconcile`). Exposes `hide_window` / `take_hidden` / `pending_for_reconcile(is_hidden)` so reconcile is testable via a `Fn(WindowId)->bool` predicate without `as_any_mut` downcast to `TestAdapter`.

**DragState** — The drag-to-swap gesture state owned by `StateManager` (`drag_window`, `overlap_target`, `overlap_start`, `last_move`). `on_moved` calls `layout::window_at_point` to update the overlap target; `on_tick` returns `DragTickAction::Swap | SnapBack | None` so `StateManager` owns `apply_layout` and workspace mutation. Keeps `last_layout_rects` borrowed from `StateManager` to avoid duplicating the rect map.

**BarReserve** — The bar reservation state owned by `StateManager` (`BarConfig`, `visible`, `spawned`). `reserved_rect(os)` is gated on `visible && spawned` (no phantom gap); `apply_reservation(workspaces, os)` pushes the strip rect into primary workspaces. `toggle()` and `on_reload(new_config)` return `ToggleAction` / `ReloadAction` so `StateManager` owns `BarSender` and `apply_layout`.

**DisplaySet** — The display ↔ workspace registry + routing policy owned by `StateManager` (`active: BTreeMap<u32, usize>` + `entries: Vec<WorkspaceEntry>` + `max_tiles`). `BTreeMap` makes iteration deterministic (no `HashMap` random fallback in `active_workspace_idx`). Owns `active` (which flat workspace is visible per monitor), the named-entry set, and the routing policy (`next_with_room`, `target_with_room`, `routed_workspace_idx`, `configured_workspace_name_for_pid`, `active_workspace_idx`, `display_in_direction`, `resolve_workspace`, `workspaces_on`, `is_visible`, `visible_or_first`) so per-monitor workspace resolution, visibility and overflow have locality in one module; `Vec<Workspace>` stays on `StateManager` and is borrowed per call. Exposes `init_workspaces`, `on_added`, `on_removed`, `on_resized` for monitor lifecycle. `capacity::next_with_room` deleted — now `DisplaySet::next_with_room` with `max_tiles` stored on the set. `StateManager` keeps one narrow `active_workspace_idx()` accessor (19 call sites); the other former delegates are deleted. `bootstrap::assemble(display_infos, primary_id, settings, session)` is the single pure assembly for workspaces/displays/gaps (Q2).

**StateManager layout** — `pengwm-daemon/src/state/mod.rs` holds construction, config reload, hide/reveal, and bar publish; `lifecycle.rs` owns the window/app lifecycle, `reconcile.rs` the tick path, `monitors_wake.rs` monitor handling and wake resync; `commands.rs` owns the `Command` dispatch (`impl StateManager`, test-called helpers are `pub(super)`); `layout_cache.rs` owns the `LayoutWriteCache` module — the four write-policy maps (`applied_rects`, `gone_since`, `pin_state`, `layout_fail_logged`) plus the funnel (`should_write` / `record_success` / `record_failure` → `AfterWrite::Keep|Untrack` / `invalidate` / `forget` / `clear_on_wake` / `seed_hidden` / `note_displaced` / `is_displaced` / `pin_backoff_active`). `StateManager` keeps one field and never touches the maps; destroy/terminate/wake collapse to one `forget` / `clear_on_wake` each; policy tests target the cache directly with no harness; `tests/` holds the suite by domain (`common` harness + `bootstrap_routing`, `lifecycle`, `commands`, `bar`, `layout_cache_hide`, `wake`). Fields stay on `StateManager`; the other files only reorganize `impl` blocks.

**PinBackoff** — After `PIN_STRIKES` (3) consecutive `Pinned` writes at the same target, `LayoutWriteCache` skips that window's writes for `PIN_BACKOFF` (15s) and retries on the timer. The invariant that makes it work: **`invalidate` clears `applied_rects` but never `pin_state`.** Both callers (`note_displaced`, the misplaced sweep) are observing "not where we put it", which is the *expected* state of a pinned window — clearing the strikes there reset the count to 1 on every 2s tick, so `PIN_STRIKES` was unreachable and a busy app (Firefox after wake) took a full 3-attempt rewrite every 2s indefinitely. The pin is dropped only where it genuinely goes stale: changed target (`should_write`), landed write (`record_success`), destroy/terminate (`forget`), wake (`clear_on_wake`). `reconcile_misplaced_windows` consults `pin_backoff_active` *before* its AX read, so a backed-off window stops re-triggering a whole-workspace re-layout. Regression tests: `layout_cache::tests::invalidate_keeps_pin_evidence` (policy) and `state::tests::wake::pin_backoff_survives_the_misplaced_sweep` (tick path).

**WindowElementCache** — A `HashMap<WindowId, AXUIElementRef>` owned by the unified macOS adapter. Populated on `kAXWindowCreatedNotification` (caller does `CFRetain`), evicted on `kAXUIElementDestroyedNotification` (caller does `CFRelease`). Makes `set_window_rect` O(1) instead of O(n) and seals CFRef memory lifecycle. Maintains a reverse `WindowId → i32` pid map so `set_window_rect` and `close_window` do not require a pid parameter from callers. Resynced on `DaemonEvent::SystemWoke` (`NSWorkspaceDidWake`/`ScreensDidWake`): `StateManager::on_system_woke` refreshes display geometry, clears `applied_rects`, re-attaches observers + re-polls all pids (which re-inserts fresh refs), then re-layouts visible workspaces.

## Architecture Boundaries

**Pure/dirty split** — `pengwm-core` is pure Rust, no macOS deps, testable on any platform. `pengwm-daemon` holds all macOS FFI. The layout pipeline crosses this boundary: `Workspace.layout()` produces global-coordinate rects so `StateManager` can blindly pass them to `OsAdapter::set_window_rect` without monitor math. Drag-overlap hit-testing is the same shape: `layout::window_at_point(rects, x, y, exclude)` answers "which other window is under this point" so `StateManager` never does rect containment itself.

## Shared Daemon↔Bar Contract

One definition, two consumers. The `[bar]` wire contract lives in `pengwm-core` so the daemon (geometry + spawn gate) and `pengwm-bar` (rendering) can never drift:

- **`config::BarConfig`** — the single `[bar]` table definition with one set of defaults (Top/32, the daemon's old winning defaults). The daemon's `Settings.bar` and the bar's own config table are both this type; both read the same file via `config::config_file_path()`. Bar-only presentation fields (`theme`, `colors`, `corner_radius`) ride along.
- **`ipc::send_command`** — one command-socket client shared by the `pengwm` CLI (`main.rs`) and the bar's click-to-switch handler; `ipc::COMMAND_SOCKET_PATH` / `ipc::BAR_SOCKET_PATH` are the single socket-path constants. The daemon re-exports them (`ipc_server::DEFAULT_SOCKET_PATH`, `bar_server::BAR_SOCKET_PATH`) for tests and callers.
- **`layout::bar_strip_rect(origin, size, position, thickness)`** — one answer for the strip rect on an edge. `StateManager::bar_reserved_rect` (daemon reservation) and `BarApp::desired_geometry` (bar self-positioning) both call it, so the two processes agree on geometry even when the daemon hasn't pushed a `State.rect` yet.
- **Reservation is gated on spawn, not on config alone** — `bar_reserved_rect` returns `None` unless `bar_visible && bar_spawned`, and `bar_visible` itself starts as `config.visible && bar_spawned`. A bar that never spawned (default `enabled = false`, or flipped on via a runtime reload) reserves no strip, so no phantom gap appears on the edge. `ToggleBar` is a no-op when nothing is running.
- **`BarApp::desired_geometry` falls back to the physical monitor** — before the first `State.rect` push, the bar computes the strip against `ViewportInfo::monitor_size` (global origin `0,0`) rather than its own `viewport_rect()`, which would be self-referential (a freshly-created window is centered, so bottom/right positions land mid-screen).
- **The bar window is fully transparent** — `BarApp` overrides `eframe::App::clear_color` to `Color32::TRANSPARENT`; eframe's default clear is a semi-transparent dark slab that painted the whole window square and hid the rounded `CornerRadius` fill behind it.

## Command Vocabulary

One `Command` type is the single vocabulary every surface feeds into `StateManager::on_command`:

- **`command::Command::parse_action(s)`** — the one action-string parser (kebab-case of the variant + args: `move-window-left`, `set-layout-tile`, `workspace-3`). Lives in `pengwm-core` with the wire type so the keybind TOML surface can never drift from it. `config/keybinds.rs::parse_action` is a thin passthrough.
- **CLI** — clap subcommands map onto the same `Command` (`move-window left` → `Command::MoveWindow`). Names line up with the keybind strings (`swap-*` is gone).
- **Reply slot** — `DaemonEvent::Command(cmd, Option<Sender<DaemonResponse>>)`. `Some` only for the IPC client; keybinds and the config watcher send `None` and get no reply, so no throwaway response channel is allocated. `on_command` acks only when a slot is present.
- **PrefixKey** — The chord (default `alt-space`) that arms a ~1s window where the full `Command` vocabulary is reachable from short follow-ups; repeatable commands (resize, focus) fire again without re-arming. Purely additive — direct binds keep working.
