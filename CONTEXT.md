# PengWM — Domain Glossary

## Layout

**LayoutPolicy** — The strategy used to arrange windows within a workspace.
- *Tiling* — Windows are arranged in splits according to the tree structure.
- *Monocle* — Only the focused window is shown fullscreen (minus gaps); all others are positioned off-screen.

**Workspace** — An independent window tree on a single monitor. The deepened interface exposes:
- `layout(gap_inner, gap_outer) -> HashMap<WindowId, Rect>` — single method that computes global-coordinate rects for every window. Uses stored monitor geometry internally. Checks `monocle` flag; if set, produces one fullscreen rect + offscreen rects for siblings.
- `hide() -> HashMap<WindowId, Rect>` — returns offscreen rects for all windows, used when switching workspaces.
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
    fn attach_observer(&mut self, pid: i32);
    fn detach_observer(&mut self, pid: i32);
    fn with_callback(callback: Box<dyn Fn(DaemonEvent) + Send>) -> Self;  // replaces mpsc hardcode
}
```

The observer side of the seam accepts `Box<dyn Fn(DaemonEvent) + Send>` (callback, not hardcoded mpsc sender).

**WindowElementCache** — A `HashMap<WindowId, AXUIElementRef>` owned by the unified macOS adapter. Populated on `kAXWindowCreatedNotification` (caller does `CFRetain`), evicted on `kAXUIElementDestroyedNotification` (caller does `CFRelease`). Makes `set_window_rect` O(1) instead of O(n) and seals CFRef memory lifecycle. Maintains a reverse `WindowId → i32` pid map so `set_window_rect` and `close_window` do not require a pid parameter from callers.

## Architecture Boundaries

**Pure/dirty split** — `pengwm-core` is pure Rust, no macOS deps, testable on any platform. `pengwm-daemon` holds all macOS FFI. The layout pipeline crosses this boundary: `Workspace.layout()` produces global-coordinate rects so `StateManager` can blindly pass them to `OsAdapter::set_window_rect` without monitor math.
