# Architecture

## Design

- **Pure/dirty split:** The layout engine (`pengwm-core`) is pure Rust with
  no macOS dependencies — unit-testable on any platform. macOS FFI lives
  entirely in `pengwm-daemon`.
- **IPC:** CLI commands are sent over a Unix Domain Socket at `/tmp/pengwm.sock`
  as JSON serialized `Command` enums.
- **Single-threaded state:** The event loop (CFRunLoop + mpsc) dispatches all
  macOS notifications, CLI commands, and keybinds on one thread.

## Event Flow

```
macOS Events ─┐
CLI Commands ─┤── mpsc::channel ──▶ StateManager ──▶ Layout Engine ──▶ AXUIElement
Keybinds ─────┘
```

## Tree

Windows are stored in an ID-based arena tree (`HashMap<NodeId, Node>`).
Splits are n-ary (3+ children in one split direction). Redundant splits
(parent and child with the same direction) are automatically flattened.

## Workspace Emulation

Workspaces are emulated — each workspace is an independent tree on the same
monitor. When a workspace is hidden, all its windows are moved to
`x: -9999` (off-screen).

## Data Flow

```
CLI:  pengwm focus left ──▶ clap parse ──▶ Command::Focus { direction: Left }
                                         ──▶ serde_json::to_string
                                         ──▶ UnixStream::connect("/tmp/pengwm.sock")
                                         ──▶ write JSON bytes

Daemon: UnixListener::accept ──▶ thread::spawn ──▶ read bytes
                                                ──▶ serde_json::from_slice<Command>
                                                ──▶ mpsc::Sender::blocking_send(DaemonEvent::Command(cmd, resp_tx))

StateManager: recv DaemonEvent ──▶ on_command(cmd, resp_tx)
                                ──▶ mutate tree
                                ──▶ apply_layout (reposition windows via AX)
                                ──▶ resp_tx.send(Ack)
```

## Crate Layout

| Crate | Deps | Purpose |
|-------|------|---------|
| `pengwm-core` | serde, serde_json | Types, layout math, workspace logic |
| `pengwm-daemon` | core, tokio, accessibility-sys, objc2 | Event loop, FFI, state, IPC server |
| `pengwm-cli` | core, clap, serde_json | CLI argument parsing, UDS sender |
