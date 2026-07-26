# PengWM Documentation

- [Getting Started](getting-started.md)
- [Configuration](configuration.md)
- [Commands](commands.md)
- [Architecture](architecture.md)

## Overview

PengWM is a tiling window manager for macOS. It runs as a background daemon
and is controlled via a CLI client over a Unix Domain Socket.

| Component | Description |
|-----------|-------------|
| `pengwm-daemon` | Background process — event loop, state tree, macOS FFI |
| `pengwm-cli` | Command-line client — sends commands via UDS |
| `pengwm-core` | Shared library — layout engine, data types, IPC protocol |
