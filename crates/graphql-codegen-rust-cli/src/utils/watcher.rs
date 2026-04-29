use std::future::Future;
use std::pin::Pin;

use crate::config::CodegenContext;
use plugin_helpers::types::FileOutput;

pub struct Watcher {
    pub running_watcher: Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send>>,
    _phantom: (),
}

pub fn create_watcher(
    _context: CodegenContext,
    _write_output: impl Fn(
        Vec<FileOutput>,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<FileOutput>>> + Send>>
    + Send
    + Sync
    + 'static,
) -> Watcher {
    // TODO: real file watching.
    // For now, return a future that never resolves.
    Watcher {
        running_watcher: Box::pin(std::future::pending()),
        _phantom: (),
    }
}
