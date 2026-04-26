use anyhow::Result;
use clap::Parser;

mod cli;
mod init;
mod utils;

use cli::{Command, run_cli};

#[derive(Parser)]
#[command(author, version, about)]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    match run_cli(args.command).await {
        Ok(code) => {
            std::process::exit(code);
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }
}
