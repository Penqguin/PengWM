# PengWM 🐧

PengWM is a workspace-oriented, tiling window manager for macOS and Windows. It uses a Binary Space Partitioning (BSP) algorithm to manage windows and provides a modern UI built with Tauri and Svelte.

## Features

- **Tiling Window Management**: Automatic BSP layout for your windows.
- **Cross-Platform**: Native support for macOS and Windows.
- **Modern UI**: A sleek dashboard for managing your workspaces and windows.
- **New App Detection**: Automatically manages new applications as they are launched.
- **Window Filtering**: Intelligently ignores tooltips, popups, and non-standard windows.
- **Real-time Sync**: UI stays in sync with the window manager daemon via IPC.

## Installation

### Prerequisites

- **Rust**: [Install Rust](https://www.rust-lang.org/tools/install)
- **Node.js**: [Install Node.js](https://nodejs.org/)
- **pnpm**: `npm install -g pnpm`

### Setup

1. **Clone the repository**:
   ```bash
   git clone https://github.com/your-username/PengWM.git
   cd PengWM
   ```

2. **Install UI dependencies**:
   ```bash
   cd pengwm-ui
   pnpm install
   cd ..
   ```

3. **Build the project**:
   ```bash
   cargo build --release
   ```

## Usage

### Running the Daemon

The daemon is the core of PengWM. It handles window management logic and system events.

```bash
cargo run -p pengwm-daemon
```

*Note: On macOS, you will be prompted to grant **Accessibility permissions** to the terminal or the binary. This is required for PengWM to manage windows.*

### Running the UI

The UI provides a dashboard and configuration interface.

```bash
cd pengwm-ui
pnpm tauri dev
```

## Configuration

PengWM is configured via a `config.toml` file located in the project root (or the same directory as the binary).

```toml
# The maximum number of windows per workspace before they start to stack
max_tiles = 4

# The outer gap between windows and the screen edge (in pixels)
gap_outer = 10

# The inner gap between adjacent windows (in pixels)
gap_inner = 5
```

## Development

- **pengwm-daemon**: Core logic, BSP tree, and platform-specific backends (Rust/Tokio).
- **pengwm-ui**: Tauri application with a Svelte frontend.
- **IPC**: Communication between the daemon and UI via local sockets / named pipes.

## License

MIT
