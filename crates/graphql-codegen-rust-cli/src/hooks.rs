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
        self.execute_hooks("afterStart", &[]).await?;
        Ok(())
    }

    pub async fn before_all_file_write(&self, filenames: Vec<String>) -> Result<()> {
        self.execute_hooks("beforeAllFileWrite", &filenames).await?;
        Ok(())
    }

    pub async fn before_one_file_write(
        &self,
        absolute_path: &str,
        content: String,
    ) -> Result<String> {
        self.execute_hooks("beforeOneFileWrite", &[absolute_path.to_string()])
            .await?;
        Ok(content)
    }

    pub async fn after_one_file_write(&self, filename: &str) -> Result<()> {
        self.execute_hooks("afterOneFileWrite", &[filename.to_string()])
            .await?;
        Ok(())
    }

    pub async fn after_all_file_write(&self, filenames: Vec<String>) -> Result<()> {
        self.execute_hooks("afterAllFileWrite", &filenames).await?;
        Ok(())
    }

    pub async fn before_done(&self) -> Result<()> {
        self.execute_hooks("beforeDone", &[]).await?;
        Ok(())
    }

    async fn execute_hooks(&self, key: &str, args: &[String]) -> Result<()> {
        let commands = self.get_commands(key);
        for command in commands {
            self.exec_shell_command(&command, args).await?;
        }
        Ok(())
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
