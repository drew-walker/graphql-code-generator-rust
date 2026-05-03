//! Mirrors `~/Projects/graphql-code-generator/packages/graphql-codegen-cli/src/load.ts`
//! (`loadSchema` / `loadDocuments`). Schema from JSON, SDL (via Node), or JS; documents from globs
//! or inline GraphQL strings (same as upstream `loadDocuments` + `@graphql-tools/load`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use globwalk::GlobWalkerBuilder;
use plugin_helpers::schema_input::SchemaGenerationInput;
use plugin_helpers::types::DocumentFile;
use serde_json::Value;

/// Loads a GraphQL schema from string pointers (paths relative to `cwd`), matching the
/// single-string-pointer case of TS `loadSchema` / `context.loadSchema`.
pub async fn load_schema(cwd: &Path, pointers: &[String]) -> Result<SchemaGenerationInput> {
    if pointers.is_empty() {
        anyhow::bail!("load_schema: empty schema pointers");
    }
    if pointers.len() > 1 {
        anyhow::bail!(
            "load_schema: multiple schema pointers not supported yet (got {})",
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
        "graphql" | "gql" => load_schema_graphql_file_loader(&path).await,
        "js" | "cjs" | "mjs" => load_schema_js(&path).await,
        _ => anyhow::bail!(
            "Unsupported schema file for {} (expected .json, .graphql, or .js)",
            path.display()
        ),
    }
}

fn is_graphql_document(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    ext == "graphql" || ext == "gql"
}

/// When a `documents` entry is a raw GraphQL string (as in `~/Projects/graphql-code-generator`
/// `dev-test/codegen.ts` → `documents: ['query test { ... }']`), globs match no files. Upstream
/// `@graphql-tools/load` still parses these; we detect the same case after an empty glob walk.
fn pointer_might_be_inline_graphql(pointer: &str) -> bool {
    let s = pointer.trim_start_matches("./").trim_start();
    let Some(first) = s.split_whitespace().next() else {
        return false;
    };
    matches!(first, "query" | "mutation" | "subscription" | "fragment")
}

async fn load_documents_for_pointers(
    cwd: &Path,
    pointers: &[String],
    ignore: &[String],
    doc_type: &str,
) -> Result<Vec<DocumentFile>> {
    if pointers.is_empty() {
        return Ok(vec![]);
    }

    let mut include: Vec<String> = Vec::new();
    let mut exclude: Vec<String> = ignore.to_vec();

    for p in pointers {
        if let Some(rest) = p.strip_prefix('!') {
            exclude.push(rest.to_string());
        } else {
            include.push(p.clone());
        }
    }

    let mut files: Vec<PathBuf> = Vec::new();
    let mut inline_graphql: Vec<String> = Vec::new();
    for pat in include {
        let pat_norm = pat.strip_prefix("./").unwrap_or(&pat).to_string();

        let walker = GlobWalkerBuilder::from_patterns(cwd, &[pat_norm.as_str()])
            .follow_links(true)
            .file_type(globwalk::FileType::FILE)
            .build()
            .with_context(|| format!("failed to build glob walker for document pointer `{pat}`"))?;

        let mut matched_file = false;
        for entry in walker.into_iter().flatten() {
            let path = entry.path().to_path_buf();
            if !is_graphql_document(&path) {
                continue;
            }
            files.push(path);
            matched_file = true;
        }

        if !matched_file && pointer_might_be_inline_graphql(&pat_norm) {
            inline_graphql.push(pat);
        }
    }

    // Apply excludes (best-effort). Upstream derives an `ignore` list from generates outputs.
    if !exclude.is_empty() {
        let mut excluded: Vec<PathBuf> = Vec::new();
        for ex in exclude {
            let ex = ex.strip_prefix("./").unwrap_or(&ex).to_string();
            let walker = GlobWalkerBuilder::from_patterns(cwd, &[ex.as_str()])
                .follow_links(true)
                .file_type(globwalk::FileType::FILE)
                .build()
                .with_context(|| {
                    format!("failed to build glob walker for ignore pointer `{ex}`")
                })?;
            for entry in walker.into_iter().flatten() {
                excluded.push(entry.path().to_path_buf());
            }
        }
        excluded.sort();
        excluded.dedup();
        files.retain(|p| !excluded.contains(p));
    }

    files.sort();
    files.dedup();

    let mut out: Vec<DocumentFile> = Vec::with_capacity(files.len() + inline_graphql.len());
    for path in files {
        let text = tokio::fs::read_to_string(&path)
            .await
            .with_context(|| format!("failed to read document {}", path.display()))?;

        // graphql-parser's AST lifetime is tied to the input buffer, so we promote the buffer to
        // `'static` for storage in `DocumentFile` (mirrors upstream's owned `DocumentNode` behavior).
        let text: &'static str = Box::leak(text.into_boxed_str());

        let document = graphql_parser::parse_query::<String>(text)
            .with_context(|| format!("failed to parse GraphQL document {}", path.display()))?;

        out.push(DocumentFile {
            location: path.to_string_lossy().to_string(),
            document,
            r#type: Some(doc_type.to_string()),
        });
    }

    for src in inline_graphql {
        let text: &'static str = Box::leak(src.into_boxed_str());
        let document = graphql_parser::parse_query::<String>(text)
            .with_context(|| format!("failed to parse inline GraphQL document `{text}`"))?;
        out.push(DocumentFile {
            location: "<inline>".to_string(),
            document,
            r#type: Some(doc_type.to_string()),
        });
    }

    Ok(out)
}

/// Loads GraphQL documents from pointers (paths or globs), mirroring the TS `loadDocuments` call site.
///
/// `ignore` is used to avoid loading generated outputs as inputs (upstream derives this from `generates`).
pub async fn load_documents(
    cwd: &Path,
    pointers: &[String],
    external_pointers: &[String],
    ignore: &[String],
) -> Result<Vec<DocumentFile>> {
    let mut out = Vec::new();
    out.extend(load_documents_for_pointers(cwd, pointers, ignore, "standard").await?);
    out.extend(load_documents_for_pointers(cwd, external_pointers, ignore, "external").await?);
    Ok(out)
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
/// Requires `graphql` to be resolvable from the schema file’s execution context (`node_modules`).
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

/// Loads a `.graphql`/`.gql` schema file (mirrors upstream `GraphQLFileLoader` branch),
/// then returns `introspectionFromSchema(schema).__schema`.
///
/// Upstream reference:
/// - `packages/graphql-codegen-cli/src/load.ts` → `loadSchema()` → loaders include `new GraphQLFileLoader()`.
///
/// Implementation note: upstream uses `@graphql-tools/load` + `GraphQLFileLoader`. We shell out to
/// Node with `graphql`'s `buildSchema` for now.
async fn load_schema_graphql_file_loader(path: &Path) -> Result<SchemaGenerationInput> {
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
        .arg(GRAPHQL_FILE_LOADER_SCRIPT_CJS)
        .output()
        .await
        .context("failed to spawn `node` for SDL schema load — is Node installed?")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "failed to load SDL schema {}: {}",
            abs.display(),
            stderr.trim()
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: Value =
        serde_json::from_str(stdout.trim()).context("failed to parse SDL schema loader JSON")?;

    let introspection = parsed
        .get("__schema")
        .cloned()
        .context("SDL schema loader JSON missing `__schema`")?;

    Ok(SchemaGenerationInput {
        introspection,
        enum_internal_values: HashMap::new(),
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

/// CommonJS one-shot: reads `CODEGEN_SCHEMA_PATH`, reads SDL, builds schema, introspects, prints JSON.
const GRAPHQL_FILE_LOADER_SCRIPT_CJS: &str = r#"
const fs = require('fs');
const { buildSchema, introspectionFromSchema } = require('graphql');

const absPath = process.env.CODEGEN_SCHEMA_PATH;
if (!absPath) {
  process.stderr.write('CODEGEN_SCHEMA_PATH is not set');
  process.exit(1);
}

const sdl = fs.readFileSync(absPath, 'utf8');
const schema = buildSchema(sdl);
const intro = introspectionFromSchema(schema);
process.stdout.write(JSON.stringify({ __schema: intro.__schema }));
"#;
