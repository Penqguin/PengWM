# PengWM — Domain Glossary

## Layout

**LayoutPolicy** — The strategy used to arrange windows within a workspace.
- *Tiling* — Windows are arranged in splits according to the tree structure.
- *Monocle* — Only the focused window is shown fullscreen (minus gaps); all others are positioned off-screen.

**Workspace** — An independent window tree on a single monitor. The deepened interface exposes:
- `layout(gap_inner, gap_outer) -> HashMap<WindowId, Rect>` — single method that computes global-coordinate rects for every window. Uses stored monitor geometry internally. Checks `monocle` flag; if set, produces one fullscreen rect + offscreen rects for siblings.
- `apply_split_direction(direction)` — the split intent: re-orients the focused Split container, or — when a Window is focused — pends the direction for the next window added. The "only a Split container re-orients" invariant lives here with the tree.
- Hiding — no workspace method: `StateManager::hide_workspace` sends `all_windows()` to `OsAdapter::hide_windows` (batch offscreen). The workspace owns *which* windows; the adapter owns *how* to hide.
- Tree internals (`root`, `arena`, `monitor_origin`, `monitor_size`) are private — `focused_node` and `monocle` remain public for daemon integration tests.

## Platform Abstraction

**OsAdapter** — The trait seam between platform-independent state logic and macOS-specific FFI. Two implementations: `MacOsAdapter` (prod) and `TestAdapter` (tests). The narrowed interface:

```rust
pub trait OsAdapter {
    fn running_app_pids(&self) -> Vec<i32>;
    fn frontmost_pid(&self) -> Option<i32>;
    fn poll_windows_for_pid(&mut self, pid: i32) -> Vec<WindowId>;  // inserts into cache
    fn focused_window_for_pid(&self, pid: i32) -> Option<WindowId>;
    fn active_displays(&self) -> Vec<DisplayInfo>;
    fn primary_display_id(&self) -> u32;
    fn set_window_rect(&mut self, window_id: WindowId, rect: Rect) -> anyhow::Result<()>;  // no pid
    fn close_window(&mut self, window_id: WindowId);  // no pid
    fn hide_windows(&mut self, window_ids: &[WindowId]);  // batch offscreen
    fn window_is_hidden(&self, window_id: WindowId) -> bool;  // kAXMinimized/kAXHidden (window + app); drives the periodic reconcile
    fn attach_observer(&mut self, pid: i32);
    fn detach_observer(&mut self, pid: i32);
    fn with_callback(callback: Box<dyn Fn(DaemonEvent) + Send>) -> Self;  // replaces mpsc hardcode
}
```

The observer side of the seam accepts `Box<dyn Fn(DaemonEvent) + Send>` (callback, not hardcoded mpsc sender).

Hidden/minimized windows are detected two ways: per-window `kAXWindowMiniaturizedNotification` / app-level `kAXApplicationHiddenNotification` fire immediately, and a ~1s `on_tick` reconcile queries `window_is_hidden` per tracked window as a fallback for missed notifications. Both untile the window (like a close) while keeping pid tracking so `on_window_shown` can retile it where it came from.

**WindowElementCache** — A `HashMap<WindowId, AXUIElementRef>` owned by the unified macOS adapter. Populated on `kAXWindowCreatedNotification` (caller does `CFRetain`), evicted on `kAXUIElementDestroyedNotification` (caller does `CFRelease`). Makes `set_window_rect` O(1) instead of O(n) and seals CFRef memory lifecycle. Maintains a reverse `WindowId → i32` pid map so `set_window_rect` and `close_window` do not require a pid parameter from callers.

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
