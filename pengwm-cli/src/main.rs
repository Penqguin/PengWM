use clap::Parser;
use pengwm_core::command::Command;

mod cli;
mod ipc_client;

fn main() {
    let cli = cli::Cli::parse();
    let cmd: Command = cli.command.into();
    match ipc_client::send_command(&cmd) {
        Ok(response) => print!("{response}"),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}
