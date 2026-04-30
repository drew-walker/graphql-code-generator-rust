use clap::Subcommand;

use crate::config::{CliFlags, create_context};
use crate::generate_and_save::generate;
use crate::hooks::lifecycle_hooks;
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

    let mut context = create_context(flags).await?;
    let hooks_config = context.get_config().hooks.clone();

    match generate(context, true).await {
        Ok(()) => {
            // TODO: Check for checkMode and log if files are stale
            Ok(0)
        }
        Err(e) => {
            lifecycle_hooks(hooks_config)
                .on_error(&e.to_string())
                .await?;
            Ok(1)
        }
    }
}
