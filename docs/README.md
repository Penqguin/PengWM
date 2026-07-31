# PengWM Documentation

- [Getting Started](getting-started.md)
- [Configuration](configuration.md)
- [Commands](commands.md)
- [Architecture](architecture.md)

## Overview

PengWM is a tiling window manager for macOS. It ships as a single `pengwm`
binary: run it with no arguments to start the daemon, or pass a subcommand
to control a running daemon over a Unix Domain Socket.

| Component | Description |
|-----------|-------------|
| `pengwm` | Single binary — daemon + CLI client, event loop, macOS FFI |
| `pengwm-core` | Shared library — layout engine, data types, IPC protocol |
