use anyhow::Result;
use serde_json::Value;

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
        self.execute_hooks("afterStart", &[], None).await?;
        Ok(())
    }

    pub async fn before_all_file_write(&self, filenames: Vec<String>) -> Result<()> {
        self.execute_hooks("beforeAllFileWrite", &filenames, None)
            .await?;
        Ok(())
    }

    #[allow(dead_code)]
    pub async fn on_watch_triggered(&self, event: &str, path: &str) -> Result<()> {
        // Wired when watch mode is implemented (mirrors upstream watcher.ts calling this hook).
        self.execute_hooks(
            "onWatchTriggered",
            &[event.to_string(), path.to_string()],
            None,
        )
        .await?;
        Ok(())
    }

    pub async fn on_error(&self, error: &str) -> Result<()> {
        self.execute_hooks("onError", &[error.to_string()], None)
            .await?;
        Ok(())
    }

    pub async fn before_one_file_write(
        &self,
        absolute_path: &str,
        content: String,
    ) -> Result<String> {
        // Upstream supports "alter hooks" (function hooks) that can return a new content string.
        // Our Rust config currently only supports string hooks (shell commands), so content is
        // unchanged unless/until function hooks are supported.
        let result = self
            .execute_hooks(
                "beforeOneFileWrite",
                &[absolute_path.to_string()],
                Some(content.clone()),
            )
            .await?;
        Ok(result.unwrap_or(content))
    }

    pub async fn after_one_file_write(&self, filename: &str) -> Result<()> {
        self.execute_hooks("afterOneFileWrite", &[filename.to_string()], None)
            .await?;
        Ok(())
    }

    pub async fn after_all_file_write(&self, filenames: Vec<String>) -> Result<()> {
        self.execute_hooks("afterAllFileWrite", &filenames, None)
            .await?;
        Ok(())
    }

    pub async fn before_done(&self) -> Result<()> {
        self.execute_hooks("beforeDone", &[], None).await?;
        Ok(())
    }

    async fn execute_hooks(
        &self,
        key: &str,
        args: &[String],
        initial_state: Option<String>,
    ) -> Result<Option<String>> {
        let commands = self.get_commands(key);
        for command in commands {
            self.exec_shell_command(&command, args).await?;
        }
        Ok(initial_state)
    }

    fn get_commands(&self, key: &str) -> Vec<String> {
        match self._config.extra.get(key) {
            Some(Value::String(s)) => vec![s.clone()],
            Some(Value::Array(values)) => values
                .iter()
                .filter_map(|v| v.as_str().map(ToOwned::to_owned))
                .collect(),
            _ => vec![],
        }
    }

    async fn exec_shell_command(&self, command: &str, args: &[String]) -> Result<()> {
        let cwd = std::env::current_dir()?;
        let quoted_args = args
            .iter()
            .map(|a| shell_quote(a))
            .collect::<Vec<_>>()
            .join(" ");
        let full_command = if quoted_args.is_empty() {
            command.to_string()
        } else {
            format!("{command} {quoted_args}")
        };

        let mut path_entries = std::env::var_os("PATH")
            .map(|v| std::env::split_paths(&v).collect::<Vec<_>>())
            .unwrap_or_default();
        path_entries.push(cwd.join("node_modules/.bin"));
        let joined_path = std::env::join_paths(path_entries)?;

        let output = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(&full_command)
            .env("PATH", joined_path)
            .output()
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Hook command failed: `{full_command}`\n{}", stderr.trim());
        }

        Ok(())
    }
}

fn shell_quote(input: &str) -> String {
    let escaped = input.replace('\'', "'\"'\"'");
    format!("'{escaped}'")
}
