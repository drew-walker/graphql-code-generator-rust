use clap::Subcommand;

use crate::init::init;

#[derive(Subcommand)]
pub enum Command {
    Init,
}

pub async fn run_cli(cmd: Option<Command>) -> anyhow::Result<i32> {
    // This is normally where ensureGraphQlPackage would be called, but it's not necessary in Rust.

    if let Some(Command::Init) = cmd {
        init().await?;
        return Ok(0);
    }

    println!("Code generation not implemented yet");
    Ok(0)
}
