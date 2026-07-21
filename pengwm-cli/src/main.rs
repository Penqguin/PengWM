//! CLI client for PengWM.
//!
//! Connects to the daemon via Unix Domain Socket at /tmp/pengwm.sock,
//! sends a JSON-serialized DaemonCommand, and prints the response.

mod cli;

fn main() {
    //  parse CLI args via clap (cli::Cli)
    //  connect to UDS at /tmp/pengwm.sock
    //  serialize DaemonCommand
    //  send over socket
    //  read response
    //  print to stdout
    todo!("cli entry")
}
