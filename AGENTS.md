# Repository Guidelines

## Project Structure

PengWM is a Rust workspace with three crates: `pengwm-core/` contains platform independent models and layout logic; `pengwm-daemon/` contains the daemon, CLI, and macOS Accessibility/Core Graphics integration; `pengwm-menubar/` contains the menu bar app. Unit tests live beside modules or in `tests/` integration suites. `docs/` holds user guides, architecture notes, and ADRs. `edge/` is the TypeScript Cloudflare distribution site and installer support.

## Build, Test, and Development

- `cargo build` builds the workspace in debug mode; use `cargo build --release` for distributable binaries.
- `cargo test` runs the Rust suite. Core logic tests run cross-platform; macOS integration tests require macOS.
- `cargo fmt` formats Rust code; `cargo clippy` runs Rust lints.
- On macOS, `RUST_LOG=debug ./target/debug/pengwm` starts a debug daemon. Run CLI commands such as `./target/debug/pengwm state` from another terminal.
- In `edge/`, `npm run dev` starts the local Cloudflare dev server and `npm run deploy` deploys it (requires configured Cloudflare access).

## Coding Style and Naming

Follow standard Rust formatting (`rustfmt`): four-space indentation, `snake_case` for modules/functions, and `UpperCamelCase` for types. Keep platform APIs in the daemon and platform independent behavior in `pengwm-core`. Add focused tests near the module they cover, or under the existing `tests/` directories. For TypeScript in `edge/`, follow the existing project configuration and use clear `camelCase` identifiers.

## Testing Guidance

Run `cargo test` after Rust changes and `cargo fmt --check` to verify formatting. Add regression tests for behavior changes, including pure core logic where possible. For changes that affect real window management, validate interactively on macOS with Accessibility permission; avoid treating a successful build as proof of correct window behavior.

## Commits and Pull Requests

Recent commits use short, descriptive subjects, often with a conventional prefix such as `chore:`; release changes use `chore: bump all crates to ...`. Keep commits focused and explain the behavior changed. Pull requests should describe the user-visible effect, list validation commands and results, link related issues or ADRs, and include screenshots or reproduction steps for visual or interactive changes. Call out macOS-only behavior and configuration changes.

## Configuration and Architecture

Runtime configuration lives at `~/.config/pengwm/config.toml` (or `$XDG_CONFIG_HOME/pengwm/config.toml`). Keep secrets and machine-specific settings out of the repository. Consult `docs/architecture.md` and relevant `docs/adr/` records before changing workspace, window identity, or display behavior; update documentation when user-facing commands or configuration change.
