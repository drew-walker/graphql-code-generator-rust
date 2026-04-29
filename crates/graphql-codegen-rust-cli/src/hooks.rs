use anyhow::Result;

use plugin_helpers::types::HooksConfig;

#[derive(Debug, Clone, Default)]
pub struct LifecycleHooks {
    _config: HooksConfig,
}

pub fn lifecycle_hooks(config: HooksConfig) -> LifecycleHooks {
    LifecycleHooks { _config: config }
}

impl LifecycleHooks {
    pub async fn after_start(&self) -> Result<()> {
        Ok(())
    }

    pub async fn before_all_file_write(&self, _filenames: Vec<String>) -> Result<()> {
        Ok(())
    }

    pub async fn before_one_file_write(
        &self,
        _absolute_path: &str,
        content: String,
    ) -> Result<String> {
        Ok(content)
    }

    pub async fn after_one_file_write(&self, _filename: &str) -> Result<()> {
        Ok(())
    }

    pub async fn after_all_file_write(&self, _filenames: Vec<String>) -> Result<()> {
        Ok(())
    }

    pub async fn before_done(&self) -> Result<()> {
        Ok(())
    }
}
