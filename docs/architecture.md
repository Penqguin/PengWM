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
                                                    .all_windows() .hide_windows
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
  └─ ws.all_windows()  →  Vec<WindowId>
        Window set to hide on workspace switch (StateManager batches them
        to OsAdapter::hide_windows).
```

The `Rect` values are in global coordinates — `StateManager` passes them
directly to `OsAdapter::set_window_rect` without further math.

### 2. OsAdapter (pengwm-daemon)

The trait seam between platform-independent state and macOS FFI, split in
two — window/display query + actuation vs observer lifecycle:

```rust
pub trait ObserverRegistry {
    fn attach_observer(&mut self, pid: i32);
    fn detach_observer(&mut self, pid: i32);
}
pub trait OsAdapter: ObserverRegistry {
    fn running_app_pids(&self) -> Vec<i32>;
    fn frontmost_pid(&self) -> Option<i32>;
    fn poll_windows_for_pid(&self, pid: i32) -> Vec<WindowId>;
    fn focused_window_for_pid(&self, pid: i32) -> Option<WindowId>;
    fn active_displays(&self) -> Vec<DisplayInfo>;
    fn primary_display_id(&self) -> u32;
    fn set_window_rect(&self, window_id: WindowId, rect: Rect) -> WriteOutcome;
    fn window_rect(&self, window_id: WindowId) -> Option<Rect>;
    fn window_kind(&self, window_id: WindowId) -> Option<WindowClass>;
    fn raise_window(&self, window_id: WindowId);
    fn close_window(&self, window_id: WindowId);
    fn hide_windows(&self, placements: &HashMap<WindowId, HidePlacement>);
    fn window_is_hidden(&self, window_id: WindowId) -> bool;
    fn app_bundle_id(&self, pid: i32) -> Option<String>;
    fn app_name(&self, pid: i32) -> Option<String>;
}
```

Layout writes return a typed `WriteOutcome` (`Ok`, `Pinned`, `Drift`, `Gone`,
`Transient`) — the writer classifies, the `LayoutWriteCache` matches. Hidden
windows park position-only (never resized, so no app reflow): `HidePlacement`
is either the bottom-edge strip of the window's own monitor or far offscreen.

Discovery classifies instead of dropping: every window element gets a typed
`WindowClass` (`Standard`, `Dialog`, `SystemDialog`, `Floating`, `Sheet`,
`Unknown`). `Standard` tiles; the dialog/floating classes become
workspace-bound **popups** — tracked on their workspace, never tiled, each
rendered by `Workspace::layout()` as a centered overlay (the shared
`centered_overlay_rect`, also behind magnify) and raised back on top on
switch-back. `Sheet`/`Unknown` are dropped at the gate exactly as the old
`AXStandardWindow`-only check did. Windows of `restricted_apps` bundles are
forced into the popup fate by routing.

Two implementations:

- **MacOsAdapter** (prod) — owns a `WindowElementCache` (`HashMap<WindowId,
  AXUIElementRef>`). Observer callbacks populate the cache on
  `WindowCreated` (`CFRetain` + insert) and evict on `WindowDestroyed`
  (remove + `CFRelease`). Hot-path operations like `set_window_rect` are
  O(1) cache lookups instead of O(n) AX element scans. `with_callback`
  (taking `Box<dyn Fn(DaemonEvent) + Send>`) is an inherent constructor,
  not part of the trait.
- **TestAdapter** (tests) — in-memory implementation sharing cells with a
  `TestHandle` (`inject_window`, `inject_window_kind`, `set_fault`,
  `displace`, `rect`, `raised`, …), no FFI required.

### 3. StateManager

Thin coordinator split by responsibility (`state/mod.rs` construction +
layout application, `lifecycle.rs` window/app lifecycle, `commands.rs`
`Command` dispatch, `monitors_wake.rs` monitor + wake handling,
`reconcile.rs` tick path, `layout_writer.rs` write policy):

```
event → StateManager → mutate Workspace → call .layout()
                                       → call OsAdapter methods
```

StateManager does not know about `NodeId`, `Arena`, or monitor geometry.
It orchestrates at the level of intents: "window created", "focus right",
"switch workspace". Narrow supporting modules own their state:
`DisplaySet` (workspace ↔ output assignment + routing), `WindowStore`
(wid↔pid↔hidden), `BarReserve` (bar geometry gate), `DragState`
(drag-to-swap gesture), `LayoutWriteCache` (write policy maps).

## Tree

Windows are stored in an ID-based arena tree (`HashMap<NodeId, Node>`).
Splits are n-ary (3+ children in one split direction). Redundant splits
(parent and child with the same direction) are automatically flattened.

## Workspace Pool (i3-style)

There is one global set of named workspaces, shared across all monitors.
Each workspace is assigned to exactly one monitor at a time and each
monitor shows exactly one workspace. `DisplaySet` owns the assignment
(`active: monitor → workspace`), an explicit focused-output cell, and the
routing policy; the `Vec<Workspace>` itself stays on `StateManager`.

- **Switch** (`workspace-N`, global 1-based config order): a switch to a
  workspace shown on another monitor **swaps** it onto the focused monitor.
- **Focus/move across monitors** (`focus-display-*`,
  `move-window-to-display-*`): focus always lands (bookkeeping-only when
  empty); moves always land (bypassing `max_tiles`); focus stays on move.
- **Monitor add/remove**: the new output shows the first hidden workspace
  (else pulls the first, swapping); removed outputs' workspaces move to
  primary. Both flow through one answer/execute funnel (`TopologySync`),
  so hide + layout + bar publish stay with `StateManager`.
- **New windows** route into their app's global workspace (no auto-pull);
  unlisted apps land on the focused workspace; overflow spills to the next
  global workspace with room (`max_tiles`, default 4).

Hidden workspaces are emulated — `StateManager` sends their `all_windows()`
to `OsAdapter::hide_windows`, which parks them position-only (bottom-edge
strip of their own monitor by default, far offscreen if configured).

## Data Flow

```
CLI:  pengwm focus left
        ──▶ clap parse (thin adapter)
        ──▶ Command::Focus { direction: Left }
        ──▶ serde_json::to_string
        ──▶ UnixStream::connect("/tmp/pengwm.sock")
        ──▶ write JSON bytes

Daemon: UnixListener::accept
        ──▶ thread::spawn
        ──▶ read bytes
        ──▶ serde_json::from_slice<Command>
        ──▶ mpsc::Sender::blocking_send(DaemonEvent::Command(cmd, Some(resp_tx)))

StateManager: recv DaemonEvent
        ──▶ on_command(cmd, reply_slot)
        ──▶ mutate Workspace tree
        ──▶ workspace.layout(gap_inner, gap_outer) → HashMap<WindowId, Rect>
        ──▶ LayoutWriteCache.plan_writes (one cheap read per window)
        ──▶ os.set_window_rect(window_id, rect) → WriteOutcome
        ──▶ if let Some(tx) = reply_slot { tx.send(Ack) }
```

Every command source feeds the same `Command` vocabulary. The CLI parses clap
subcommands; the keybind surface parses action strings (`Command::parse_action`)
for the TOML config. The reply slot is nullable — only the IPC client gets one.
Keybinds and the config watcher send `DaemonEvent::Command(cmd, None)`, so they
never allocate a throwaway response channel.

## Layout-Write Policy

Every `set_window_rect` returns a `WriteOutcome` and the `LayoutWriteCache`
decides what it means: skip-if-unchanged (no redundant AX writes), gone
grace (a window is untracked only after repeated misses — any answering
readback resets the timer), and pinned-write backoff (a window whose
readbacks stop converging, e.g. a busy Firefox, is left alone for 15s after
3 strikes instead of being rewritten every tick).

## Wake Resync

`SystemWoke` only **arms** a resync — AX answers nothing for seconds after
wake, and resyncing into the blackout would mark every live window gone.
`on_tick` re-probes every 500ms and commits (re-attach observers, refresh
display geometry, clear the write cache, re-tile) once a poll returns
windows, or at a 20s deadline regardless.

## Crate Layout

| Crate | Deps | Purpose |
|-------|------|---------|
| `pengwm-core` | serde, serde_json | Types, layout math, workspace logic |
| `pengwm-daemon` | core, tokio, clap, accessibility-sys, objc2 | Single `pengwm` binary: daemon, CLI parser, UDS sender/server |
| `pengwm-bar` | core, eframe/egui, serde, toml, objc2 (macOS) | Status bar: split icon + workspace pills |

## Bar

`pengwm-bar` is a lightweight eframe (egui) process that subscribes to a
second UDS at `/tmp/pengwm-bar.sock`. The daemon spawns it at startup (gated on
`[bar].enabled`, excluded from tiling by pid) and pushes newline-delimited JSON
`BarMessage`s over that socket.

```
pengwm-daemon                    pengwm-bar
  bar_server ── BarMessage ──▶ ─── subscribe() ──▶ egui repaint
  (caches last Show/Hide         (reconnect w/ backoff,
   + last State, replays         250ms → 2s)
   both on connect)              ── send_command(Command) ──▶ pengwm.sock
                                                                 └─▶ daemon
```

- `BarMessage::Show` / `Hide` drive `ViewportCommand::Visible`; `Reload`
  re-reads config + theme; `Exit` closes the window.
- `BarMessage::State` carries `workspaces`, `active_workspace`,
  `split_direction`, and the daemon-reserved `rect`; the bar positions itself
  with `OuterPosition`/`InnerSize` and switches workspaces on click by sending
  `Command::Workspace` back over the command socket.
- Themes are built-in TOML presets (tokyo-night default) with `[bar.colors]`
  overrides; corner radius auto-matches the macOS version.

