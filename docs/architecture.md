# Architecture

## Design

- **Pure/dirty split:** `pengwm-core` is pure Rust with no macOS dependencies
  — unit-testable on any platform. macOS FFI lives entirely in `pengwm-daemon`.
- **IPC:** CLI commands are sent over a Unix Domain Socket at `/tmp/pengwm.sock`
  as JSON serialized `Command` enums.
- **Single-threaded state:** The event loop (CFRunLoop + mpsc) dispatches all
  macOS notifications, CLI commands, and keybinds on one thread.

## Event Flow

```
macOS Events ─┐
CLI Commands ─┤── mpsc::channel ──▶ StateManager ──▶ Workspace    ──▶ OsAdapter
Keybinds ─────┘                                    .layout()      .set_window_rect
                                                    .hide()        .hide_windows
```

## Layered Interface

### 1. Workspace (pengwm-core)

Each `Workspace` owns an arena tree of windows. The tree structure is an
implementation detail — external code calls high-level methods:

```
StateManager
  │
  ├─ ws.layout(gap_inner, gap_outer)  →  HashMap<WindowId, Rect>
  │     Internally: applies outer gap, walks tree, adds gaps between
  │     siblings, converts to global coordinates. Monocle mode produces
  │     one fullscreen rect + offscreen rects for siblings.
  │
  └─ ws.hide()  →  HashMap<WindowId, Rect>
        Offscreen rects for all windows (workspace switch).
```

The `Rect` values are in global coordinates — `StateManager` passes them
directly to `OsAdapter::set_window_rect` without further math.

### 2. OsAdapter (pengwm-daemon)

The trait seam between platform-independent state and macOS FFI:

```rust
pub trait OsAdapter {
    fn running_app_pids(&self) -> Vec<i32>;
    fn frontmost_pid(&self) -> Option<i32>;
    fn poll_windows_for_pid(&mut self, pid: i32) -> Vec<WindowId>;
    fn focused_window_for_pid(&self, pid: i32) -> Option<WindowId>;
    fn active_displays(&self) -> Vec<DisplayInfo>;
    fn primary_display_id(&self) -> u32;
    fn set_window_rect(&mut self, window_id: WindowId, rect: Rect) -> anyhow::Result<()>;
    fn close_window(&mut self, window_id: WindowId);
    fn hide_windows(&mut self, window_ids: &[WindowId]);
    fn attach_observer(&mut self, pid: i32);
    fn detach_observer(&mut self, pid: i32);
    fn with_callback(callback: Box<dyn Fn(DaemonEvent) + Send>) -> Self;
}
```

Two implementations:

- **MacOsAdapter** (prod) — owns a `WindowElementCache` (`HashMap<WindowId,
  (AXUIElementRef, i32)>`). Observer callbacks populate the cache on
  `WindowCreated` (`CFRetain` + insert) and evict on `WindowDestroyed`
  (remove + `CFRelease`). Hot-path operations like `set_window_rect` are
  O(1) cache lookups instead of O(n) AX element scans.
- **TestAdapter** (`#[cfg(test)]`) — in-memory HashMap implementation,
  no FFI required.

### 3. StateManager

Thin coordinator:

```
event → StateManager → mutate Workspace → call .layout() / .hide()
                                       → call OsAdapter methods
```

StateManager does not know about `NodeId`, `Arena`, or monitor geometry.
It orchestrates at the level of intents: "window created", "focus right",
"switch workspace".

## Tree

Windows are stored in an ID-based arena tree (`HashMap<NodeId, Node>`).
Splits are n-ary (3+ children in one split direction). Redundant splits
(parent and child with the same direction) are automatically flattened.

## Workspace Emulation

Workspaces are emulated — each workspace is an independent tree on the same
monitor. When a workspace is hidden, `Workspace::hide()` returns offscreen
rects (x: -9999) for all its windows.

## Data Flow

```
CLI:  pengwm focus left
        ──▶ clap parse
        ──▶ Command::Focus { direction: Left }
        ──▶ serde_json::to_string
        ──▶ UnixStream::connect("/tmp/pengwm.sock")
        ──▶ write JSON bytes

Daemon: UnixListener::accept
        ──▶ thread::spawn
        ──▶ read bytes
        ──▶ serde_json::from_slice<Command>
        ──▶ mpsc::Sender::blocking_send(DaemonEvent::Command(cmd, resp_tx))

StateManager: recv DaemonEvent
        ──▶ on_command(cmd, resp_tx)
        ──▶ mutate Workspace tree
        ──▶ workspace.layout(gap_inner, gap_outer) → HashMap<WindowId, Rect>
        ──▶ os.set_window_rect(window_id, rect) for each entry
        ──▶ resp_tx.send(Ack)
```

## Crate Layout

| Crate | Deps | Purpose |
|-------|------|---------|
| `pengwm-core` | serde, serde_json | Types, layout math, workspace logic |
| `pengwm-daemon` | core, tokio, accessibility-sys, objc2 | Event loop, FFI, state, IPC server |
| `pengwm-cli` | core, clap, serde_json | CLI argument parsing, UDS sender |
