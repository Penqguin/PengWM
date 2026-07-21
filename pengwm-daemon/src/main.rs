//! Entry point for the PengWM daemon.
//!
//! 1. Check accessibility permissions (AXIsProcessTrusted).
//! 2. Initialize logging (env_logger).
//! 3. Build the event_loop::EventLoop and run it.
//!
//! The daemon never exits under normal operation — it listens for macOS events
//! and CLI commands indefinitely.

mod event_loop;
mod state;
mod uds;

pub mod macos;
pub mod config;

fn main() {
    //  init env_logger
    //  check accessibility permissions; if not trusted, print instructions and exit
    //  build EventLoop
    //  event_loop.run()
    todo!("daemon entry point")
}
