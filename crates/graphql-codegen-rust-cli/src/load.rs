//! Mirrors `packages/graphql-codegen-cli/src/load.ts` — schema loading only for now.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use plugin_helpers::schema_input::SchemaGenerationInput;
use serde_json::Value;

/// Loads a GraphQL schema from string pointers (paths relative to `cwd`), matching the
/// single-string-pointer case of TS `loadSchema` / `context.loadSchema`.
pub async fn load_schema_for_pointers(
    cwd: &Path,
    pointers: &[String],
) -> Result<SchemaGenerationInput> {
    if pointers.is_empty() {
        anyhow::bail!("load_schema_for_pointers: empty schema pointers");
    }
    if pointers.len() > 1 {
        anyhow::bail!(
            "load_schema_for_pointers: multiple schema pointers not supported yet (got {})",
            pointers.len()
        );
    }

    let path = resolve_path(cwd, &pointers[0]);
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();

    match ext.as_str() {
        "json" => load_introspection_json(&path),
        "graphql" | "gql" => anyhow::bail!(
            "GraphQL SDL schema files are not supported yet: {} (use .json or .js for now)",
            path.display()
        ),
        "js" | "cjs" | "mjs" => load_schema_js(&path).await,
        _ => anyhow::bail!(
            "Unsupported schema file for {} (expected .json, .graphql, or .js)",
            path.display()
        ),
    }
}

fn resolve_path(cwd: &Path, p: &str) -> PathBuf {
    let path = Path::new(p);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

fn load_introspection_json(path: &Path) -> Result<SchemaGenerationInput> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read schema JSON {}", path.display()))?;
    let v: Value = serde_json::from_str(&text).context("failed to parse schema JSON")?;
    let introspection = extract_introspection_schema(&v).with_context(|| {
        format!(
            "schema JSON at {} must contain `__schema` or `data.__schema`",
            path.display()
        )
    })?;
    Ok(SchemaGenerationInput {
        introspection,
        enum_internal_values: HashMap::new(),
    })
}

fn extract_introspection_schema(v: &Value) -> Option<Value> {
    v.pointer("/__schema")
        .or_else(|| v.pointer("/data/__schema"))
        .cloned()
}

/// Loads a JS/CJS module that exports a `GraphQLSchema` (same shapes as `@graphql-tools/code-file-loader`).
/// Uses `graphql` from `node_modules` (install `graphql` under `dev-test/` for dogfooding).
async fn load_schema_js(path: &Path) -> Result<SchemaGenerationInput> {
    let abs = path
        .canonicalize()
        .with_context(|| format!("schema file not found: {}", path.display()))?;

    let output = tokio::process::Command::new("node")
        .current_dir(
            abs.parent()
                .context("schema path has no parent directory")?,
        )
        .env("CODEGEN_SCHEMA_PATH", abs.as_os_str())
        .arg("-e")
        .arg(SCHEMA_LOAD_SCRIPT_CJS)
        .output()
        .await
        .context("failed to spawn `node` for schema load — is Node installed?")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "failed to load JS schema {}: {}",
            abs.display(),
            stderr.trim()
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: Value =
        serde_json::from_str(stdout.trim()).context("failed to parse schema loader JSON")?;

    let introspection = parsed
        .get("__schema")
        .cloned()
        .context("schema loader JSON missing `__schema`")?;

    let enum_internal_values = parsed
        .get("enumInternalValues")
        .map(parse_enum_internal_values)
        .unwrap_or_default();

    Ok(SchemaGenerationInput {
        introspection,
        enum_internal_values,
    })
}

fn parse_enum_internal_values(v: &Value) -> HashMap<String, HashMap<String, String>> {
    let mut out = HashMap::new();
    let Some(obj) = v.as_object() else {
        return out;
    };
    for (type_name, inner) in obj {
        let mut m = HashMap::new();
        if let Some(im) = inner.as_object() {
            for (value_name, vv) in im {
                m.insert(value_name.clone(), json_to_display_string(vv));
            }
        }
        out.insert(type_name.clone(), m);
    }
    out
}

fn json_to_display_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "null".to_string(),
        _ => v.to_string(),
    }
}

/// CommonJS one-shot: reads `CODEGEN_SCHEMA_PATH`, requires it, introspects, prints JSON.
const SCHEMA_LOAD_SCRIPT_CJS: &str = r#"
const path = require('path');
const { createRequire } = require('module');
const { introspectionFromSchema } = require('graphql');

const absPath = process.env.CODEGEN_SCHEMA_PATH;
if (!absPath) {
  process.stderr.write('CODEGEN_SCHEMA_PATH is not set');
  process.exit(1);
}

const dir = path.dirname(absPath);
const req = createRequire(path.join(dir, '_.cjs'));
const mod = req(absPath);
const schema = mod.schema ?? mod.default?.schema ?? mod.default;
if (!schema || typeof schema.getTypeMap !== 'function') {
  process.stderr.write('Expected a GraphQLSchema export (schema/default.schema/default) from ' + absPath);
  process.exit(1);
}

const intro = introspectionFromSchema(schema);
const enumInternalValues = {};
const typeMap = schema.getTypeMap();
for (const typeName of Object.keys(typeMap)) {
  if (typeName.startsWith('__')) continue;
  const t = typeMap[typeName];
  if (!t || typeof t.getValues !== 'function') continue;
  const vals = {};
  for (const ev of t.getValues()) {
    vals[ev.name] = ev.value;
  }
  if (Object.keys(vals).length) {
    enumInternalValues[typeName] = vals;
  }
}

process.stdout.write(JSON.stringify({ __schema: intro.__schema, enumInternalValues }));
"#;
