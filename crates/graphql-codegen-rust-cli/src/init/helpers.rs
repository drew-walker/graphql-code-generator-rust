use super::types::{CodegenConfig, InitAnswers, Tag};

use crate::utils::get_latest_version::get_latest_version;
use anyhow::Context;
use serde_json::Map;
use serde_json::Value;
use std::path::PathBuf;

fn json_to_ts_expr(v: &Value, indent: usize) -> String {
    match v {
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => serde_json::to_string(s).unwrap_or_else(|_| format!("{s:?}")),
        Value::Array(arr) => {
            if arr.is_empty() {
                return "[]".to_string();
            }
            let next = indent + 2;
            let inner = arr
                .iter()
                .map(|x| format!("{}{}", " ".repeat(next), json_to_ts_expr(x, next)))
                .collect::<Vec<_>>()
                .join(",\n");
            format!("[\n{inner}\n{}]", " ".repeat(indent))
        }
        Value::Object(map) => {
            if map.is_empty() {
                return "{}".to_string();
            }
            let next = indent + 2;
            let mut keys: Vec<_> = map.keys().collect();
            keys.sort();

            let inner = keys
                .into_iter()
                .map(|k| {
                    let key = if is_simple_ident(k) {
                        k.to_string()
                    } else {
                        serde_json::to_string(k).unwrap_or_else(|_| format!("{k:?}"))
                    };
                    let val = json_to_ts_expr(&map[k], next);
                    format!("{}{}: {}", " ".repeat(next), key, val)
                })
                .collect::<Vec<_>>()
                .join(",\n");
            format!("{{\n{inner}\n{}}}", " ".repeat(indent))
        }
    }
}

fn is_simple_ident(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

pub struct WriteConfigResult {
    pub relative_path: String,
    // pub full_path: String,
}

pub async fn write_config(
    answers: &InitAnswers,
    config: &CodegenConfig,
) -> anyhow::Result<WriteConfigResult> {
    tokio::task::spawn_blocking({
        let answers = answers.clone();
        let config = config.clone();
        move || write_config_sync(&answers, &config)
    })
    .await
    .context("join write_config task")?
}

fn write_config_sync(
    answers: &InitAnswers,
    config: &CodegenConfig,
) -> anyhow::Result<WriteConfigResult> {
    let ext = answers
        .config
        .to_ascii_lowercase()
        .split('.')
        .nth(1)
        .unwrap_or("")
        .to_string();

    let cwd = std::env::current_dir().context("resolve current directory")?;
    let full_path: PathBuf = cwd.join(&answers.config);

    let relative_path = pathdiff::diff_paths(&full_path, &cwd)
        .unwrap_or_else(|| full_path.clone())
        .to_string_lossy()
        .to_string();

    let content: String = if ext == "ts" {
        // TS used Babel to generate a JS/TS object expression.
        // Here we generate a TS object literal from serde_json::Value for similar output.
        let v = serde_json::to_value(config).context("serialize config to json value")?;
        let obj = json_to_ts_expr(&v, 0);

        format!(
            r#"
import type {{ CodegenConfig }} from '@graphql-codegen/cli';

const config: CodegenConfig = {obj}

export default config;
"#
        )
    } else if ext == "json" {
        serde_json::to_string(config).context("serialize config to json")?
    } else {
        serde_yaml::to_string(config).context("serialize config to yaml")?
    };

    std::fs::write(&full_path, content)
        .with_context(|| format!("write {}", full_path.display()))?;

    Ok(WriteConfigResult {
        relative_path,
        // full_path: full_path.to_string_lossy().to_string(),
    })
}

pub async fn write_package(answers: &InitAnswers, config_location: &str) -> anyhow::Result<()> {
    tokio::task::spawn_blocking({
        let answers = answers.clone();
        let config_location = config_location.to_string();
        move || write_package_sync(&answers, &config_location)
    })
    .await
    .context("join write_package task")?
}

fn write_package_sync(answers: &InitAnswers, config_location: &str) -> anyhow::Result<()> {
    let cwd = std::env::current_dir().context("resolve current directory")?;
    let pkg_path: PathBuf = cwd.join("package.json");

    let pkg_content = std::fs::read_to_string(&pkg_path)
        .with_context(|| format!("read {}", pkg_path.display()))?;

    let mut pkg: Value =
        serde_json::from_str(&pkg_content).context("parse package.json as JSON")?;

    let indent = detect_json_indent(&pkg_content);

    ensure_object(&mut pkg, "scripts");

    {
        let scripts = get_object_mut(&mut pkg, "scripts")?;
        scripts.insert(
            answers.script.clone(),
            Value::String(format!("graphql-codegen --config {}", config_location)),
        );
    }

    ensure_object(&mut pkg, "devDependencies");

    for plugin in answers.plugins.as_deref().unwrap_or(&[]) {
        let version = get_latest_version(&plugin.package)?;
        let dev = get_object_mut(&mut pkg, "devDependencies")?;
        dev.insert(plugin.package.clone(), Value::String(version));
    }

    if answers.introspection {
        let name = "@graphql-codegen/introspection";
        let version = get_latest_version(name)?;
        let dev = get_object_mut(&mut pkg, "devDependencies")?;
        dev.insert(name.to_string(), Value::String(version));
    }

    {
        let name = "@graphql-codegen/cli";
        let version = get_latest_version(name)?;
        let dev = get_object_mut(&mut pkg, "devDependencies")?;
        dev.insert(name.to_string(), Value::String(version));
    }

    if answers.targets.contains(&Tag::Client) {
        let name = "@graphql-codegen/client-preset";
        let version = get_latest_version(name)?;
        let dev = get_object_mut(&mut pkg, "devDependencies")?;
        dev.insert(name.to_string(), Value::String(version));
    }

    let json = to_json_pretty_with_indent(&pkg, indent)?;
    std::fs::write(&pkg_path, json).with_context(|| format!("write {}", pkg_path.display()))?;

    Ok(())
}

fn detect_json_indent(src: &str) -> Vec<u8> {
    // Very small analogue to `detect-indent`: find a line that starts with spaces then a quote.
    for line in src.lines() {
        let trimmed = line.trim_start_matches(' ');
        if trimmed.starts_with('"') {
            let n = line.len() - trimmed.len();
            if n > 0 {
                return vec![b' '; n];
            }
        }
    }
    vec![b' '; 2]
}

fn to_json_pretty_with_indent(v: &Value, indent: Vec<u8>) -> anyhow::Result<String> {
    use serde::Serialize;
    use serde_json::ser::{PrettyFormatter, Serializer};
    let mut out = Vec::new();
    let formatter = PrettyFormatter::with_indent(&indent);
    let mut ser = Serializer::with_formatter(&mut out, formatter);
    v.serialize(&mut ser).context("serialize package.json")?;
    String::from_utf8(out).context("package.json utf8")
}

fn ensure_object(root: &mut Value, key: &str) {
    if let Value::Object(map) = root {
        map.entry(key.to_string())
            .or_insert_with(|| Value::Object(Map::new()));
    }
}

fn get_object_mut<'a>(
    root: &'a mut Value,
    key: &str,
) -> anyhow::Result<&'a mut Map<String, Value>> {
    match root {
        Value::Object(map) => match map.get_mut(key) {
            Some(Value::Object(obj)) => Ok(obj),
            Some(_) => anyhow::bail!("{key} must be an object in package.json"),
            None => anyhow::bail!("{key} missing in package.json"),
        },
        _ => anyhow::bail!("package.json root must be an object"),
    }
}
