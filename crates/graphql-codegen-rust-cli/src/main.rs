use anyhow::Result;
use clap::Parser;

mod cli;
mod codegen;
mod config;
mod generate_and_save;
mod hooks;
mod init;
mod load;
mod relay_optimize;
mod utils;

use cli::{Command, run_cli};
use config::CliFlags;

#[derive(Parser)]
#[command(author, version, about)]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,

    #[command(flatten)]
    flags: CliFlags,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    match run_cli(args.command, args.flags).await {
        Ok(code) => {
            std::process::exit(code);
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }
}
