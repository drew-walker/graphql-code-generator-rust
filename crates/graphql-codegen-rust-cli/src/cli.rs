use clap::Subcommand;

use crate::config::{CliFlags, create_context};
use crate::generate_and_save::generate;
use crate::init::init;

#[derive(Subcommand)]
pub enum Command {
    Init,
}

pub async fn run_cli(cmd: Option<Command>, flags: CliFlags) -> anyhow::Result<i32> {
    // This is normally where ensureGraphQlPackage would be called, but it's not necessary in Rust.

    if let Some(Command::Init) = cmd {
        init().await?;
        return Ok(0);
    }

    let context = create_context(flags)?;
    generate(context, true).await?;
    // TODO: Check for checkMode and log if files are stale
    Ok(0)
}
