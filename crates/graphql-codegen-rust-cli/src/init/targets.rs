use std::fs;

use anyhow::Context;
use serde_json::Value;

use super::types::PossibleTargets;

pub async fn guess_targets() -> anyhow::Result<PossibleTargets> {
    tokio::task::spawn_blocking(guess_targets_sync)
        .await
        .context("join guess_targets task")?
}

fn guess_targets_sync() -> anyhow::Result<PossibleTargets> {
    let cwd = std::env::current_dir().context("resolve current directory")?;
    let path = cwd.join("package.json");
    let raw = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let pkg: Value = serde_json::from_str(&raw).context("parse package.json as JSON")?;

    let dependencies = dependency_names(&pkg);

    fn has(deps: &[String], name: &str) -> bool {
        deps.iter().any(|d| d == name)
    }

    Ok(PossibleTargets {
        angular: has(&dependencies, "@angular/core"),
        react: has(&dependencies, "react"),
        stencil: has(&dependencies, "@stencil/core"),
        vue: has(&dependencies, "vue") || has(&dependencies, "nuxt"),
        browser: false,
        node: false,
        typescript: has(&dependencies, "typescript"),
        flow: has(&dependencies, "flow"),
        graphql_request: has(&dependencies, "graphql-request"),
    })
}

fn dependency_names(pkg: &Value) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();
    for section in ["dependencies", "devDependencies"] {
        if let Some(obj) = pkg.get(section).and_then(|v| v.as_object()) {
            keys.extend(obj.keys().cloned());
        }
    }
    keys.sort();
    keys.dedup();
    keys
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dependency_names_merges_sections() {
        let v: Value = serde_json::json!({
            "dependencies": { "react": "18", "typescript": "5" },
            "devDependencies": { "react": "18", "vitest": "1" }
        });
        let names = dependency_names(&v);
        assert!(names.contains(&"react".to_string()));
        assert!(names.contains(&"typescript".to_string()));
        assert!(names.contains(&"vitest".to_string()));
    }
}
