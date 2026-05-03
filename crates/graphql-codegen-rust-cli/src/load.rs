//! Mirrors `graphql-code-generator/packages/graphql-codegen-cli/src/load.ts`
//! (`loadSchema` / `loadDocuments`). Schema from JSON, SDL (via Node), or JS; documents from globs
//! or inline GraphQL strings (same as upstream `loadDocuments` + `@graphql-tools/load`).

use anyhow::{Context as _, Result};
use globwalk::GlobWalkerBuilder;
use plugin_helpers::schema_input::SchemaGenerationInput;
use plugin_helpers::types::DocumentFile;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::utils::debugging::{debug_event, debug_timing, timing_enabled_from_env};

/// Loads a GraphQL schema from string pointers (paths relative to `cwd`), matching the
/// single-string-pointer case of TS `loadSchema` / `context.loadSchema`.
pub async fn load_schema_with_timing(
    cwd: &Path,
    pointers: &[String],
    timing_enabled: bool,
) -> Result<SchemaGenerationInput> {
    let timing_enabled = timing_enabled || timing_enabled_from_env();
    let started = Instant::now();
    debug_event(
        timing_enabled,
        format!("enter load_schema pointers={pointers:?}"),
    );
    if pointers.is_empty() {
        anyhow::bail!("load_schema: empty schema pointers");
    }

    let result = if pointers.len() > 1 || pointers.iter().any(|p| pointer_needs_glob_walk(p)) {
        load_schema_graphql_pointers(cwd, pointers, timing_enabled).await
    } else {
        let path = resolve_path(cwd, &pointers[0]);
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .unwrap_or_default();

        match ext.as_str() {
            "json" => load_introspection_json(&path, timing_enabled),
            "graphql" | "gql" | "graphqls" => {
                load_schema_graphql_file_loader(&path, timing_enabled).await
            }
            "js" | "cjs" | "mjs" => load_schema_js(&path, timing_enabled).await,
            _ => anyhow::bail!(
                "Unsupported schema file for {} (expected .json, .graphql, or .js)",
                path.display()
            ),
        }
    };

    debug_timing(
        timing_enabled,
        format!("load_schema pointers={pointers:?}"),
        started,
    );
    result
}

async fn load_schema_graphql_pointers(
    cwd: &Path,
    pointers: &[String],
    timing_enabled: bool,
) -> Result<SchemaGenerationInput> {
    let mut files = Vec::new();
    for pointer in pointers {
        let pointer = pointer.strip_prefix("./").unwrap_or(pointer);
        if pointer_needs_glob_walk(pointer) {
            debug_event(timing_enabled, format!("starting schema glob `{pointer}`"));
            let glob_started = Instant::now();
            let before = files.len();
            let walker = GlobWalkerBuilder::from_patterns(cwd, &[pointer])
                .follow_links(true)
                .file_type(globwalk::FileType::FILE)
                .build()
                .with_context(|| {
                    format!("failed to build glob walker for schema pointer `{pointer}`")
                })?;
            for entry in walker.into_iter().flatten() {
                let path = entry.path().to_path_buf();
                let ext = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.to_ascii_lowercase())
                    .unwrap_or_default();
                if matches!(ext.as_str(), "graphql" | "gql" | "graphqls") {
                    files.push(path);
                }
            }
            debug_timing(
                timing_enabled,
                format!(
                    "schema glob `{pointer}` matched {} files",
                    files.len().saturating_sub(before)
                ),
                glob_started,
            );
        } else {
            files.push(resolve_path(cwd, pointer));
        }
    }

    files.sort();
    files.dedup();
    if files.is_empty() {
        anyhow::bail!(
            "load_schema: no schema files matched pointers {:?}",
            pointers
        );
    }

    let mut sdl = String::new();
    for file in files {
        debug_event(
            timing_enabled,
            format!("starting schema file read {}", file.display()),
        );
        let read_started = Instant::now();
        let text = tokio::fs::read_to_string(&file)
            .await
            .with_context(|| format!("failed to read schema {}", file.display()))?;
        debug_timing(
            timing_enabled,
            format!("read schema file {}", file.display()),
            read_started,
        );
        if !sdl.is_empty() {
            sdl.push('\n');
        }
        sdl.push_str(&text);
    }
    load_schema_graphql_sdl(&sdl, cwd, timing_enabled).await
}

fn is_graphql_document(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    ext == "graphql" || ext == "gql"
}

fn is_code_document(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    matches!(
        ext.as_str(),
        "js" | "jsx" | "ts" | "tsx" | "mts" | "cts" | "mjs" | "cjs"
    )
}

fn pluck_graphql_sources(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(marker_start) = rest.find("/* GraphQL */") {
        let after_marker = &rest[marker_start + "/* GraphQL */".len()..];
        let Some(tick_start) = after_marker.find('`') else {
            break;
        };
        let after_tick = &after_marker[tick_start + 1..];
        let Some(tick_end) = after_tick.find('`') else {
            break;
        };
        out.push(after_tick[..tick_end].to_string());
        rest = &after_tick[tick_end + 1..];
    }
    out
}

/// When a `documents` entry is a raw GraphQL string (as in `graphql-code-generator`
/// `dev-test/codegen.ts` → `documents: ['query test { ... }']`), globs match no files. Upstream
/// `@graphql-tools/load` still parses these; we detect the same case after an empty glob walk.
/// True when the pointer needs a filesystem glob walk (`globwalk`).
///
/// Upstream passes `ignore` from `load.ts` into `@graphql-tools/load` as filesystem paths (see
/// `join(config.cwd, generatePath)`). Those are almost always **literal** output paths, not globs.
/// Feeding a literal through `GlobWalkerBuilder` is both slow (wide tree walks) and can diverge
/// from toolkit matching; we only glob-walk when the string contains glob metacharacters (same
/// idea as negated document globs like `!./foo/**/*.graphql`).
fn pointer_needs_glob_walk(pointer: &str) -> bool {
    pointer
        .chars()
        .any(|c| matches!(c, '*' | '?' | '[' | ']' | '{' | '}'))
}

fn add_excluded_path(set: &mut HashSet<PathBuf>, path: PathBuf) {
    match std::fs::canonicalize(&path) {
        Ok(c) => {
            set.insert(c);
        }
        Err(_) => {
            set.insert(path);
        }
    }
}

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
    timing_enabled: bool,
) -> Result<Vec<DocumentFile>> {
    let timing_enabled = timing_enabled || timing_enabled_from_env();
    let started = Instant::now();
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

        if pointer_might_be_inline_graphql(&pat_norm) {
            debug_event(
                timing_enabled,
                format!("{doc_type} treating pointer as inline GraphQL"),
            );
            inline_graphql.push(pat);
            continue;
        }

        debug_event(
            timing_enabled,
            format!("{doc_type} starting document glob `{pat_norm}`"),
        );
        let glob_started = Instant::now();
        let before = files.len();
        let walker = GlobWalkerBuilder::from_patterns(cwd, &[pat_norm.as_str()])
            .follow_links(true)
            .file_type(globwalk::FileType::FILE)
            .build()
            .with_context(|| format!("failed to build glob walker for document pointer `{pat}`"))?;

        let mut matched_file = false;
        for entry in walker.into_iter().flatten() {
            let path = entry.path().to_path_buf();
            if !is_graphql_document(&path) && !is_code_document(&path) {
                continue;
            }
            files.push(path);
            matched_file = true;
        }
        debug_timing(
            timing_enabled,
            format!(
                "{doc_type} document glob `{pat_norm}` matched {} candidate files",
                files.len().saturating_sub(before)
            ),
            glob_started,
        );

        if !matched_file && pointer_might_be_inline_graphql(&pat_norm) {
            inline_graphql.push(pat);
        }
    }

    // Apply excludes (best-effort). Mirrors `load.ts` → `ignore` passed to `loadDocuments` from
    // `graphql-codegen-cli` (concrete output paths + user `!` negations, which may be globs).
    if !exclude.is_empty() {
        let mut excluded: HashSet<PathBuf> = HashSet::new();
        for ex in exclude {
            let ex = ex.strip_prefix("./").unwrap_or(&ex).to_string();
            if pointer_needs_glob_walk(&ex) {
                debug_event(
                    timing_enabled,
                    format!("{doc_type} starting ignore glob `{ex}`"),
                );
                let exclude_started = Instant::now();
                let before = excluded.len();
                let walker = GlobWalkerBuilder::from_patterns(cwd, &[ex.as_str()])
                    .follow_links(true)
                    .file_type(globwalk::FileType::FILE)
                    .build()
                    .with_context(|| {
                        format!("failed to build glob walker for ignore pointer `{ex}`")
                    })?;
                for entry in walker.into_iter().flatten() {
                    add_excluded_path(&mut excluded, entry.path().to_path_buf());
                }
                debug_timing(
                    timing_enabled,
                    format!(
                        "{doc_type} ignore glob `{ex}` matched {} files",
                        excluded.len().saturating_sub(before)
                    ),
                    exclude_started,
                );
            } else {
                let candidate = cwd.join(&ex);
                if candidate.is_file() {
                    add_excluded_path(&mut excluded, candidate);
                }
            }
        }
        files.retain(|p| {
            let key = std::fs::canonicalize(p).unwrap_or_else(|_| p.clone());
            !excluded.contains(&key) && !excluded.contains(p)
        });
    }

    files.sort();
    files.dedup();

    let mut out: Vec<DocumentFile> = Vec::with_capacity(files.len() + inline_graphql.len());
    for path in files {
        debug_event(
            timing_enabled,
            format!("{doc_type} starting document read {}", path.display()),
        );
        let read_started = Instant::now();
        let text = tokio::fs::read_to_string(&path)
            .await
            .with_context(|| format!("failed to read document {}", path.display()))?;
        debug_timing(
            timing_enabled,
            format!("{doc_type} read document {}", path.display()),
            read_started,
        );

        let collect_started = Instant::now();
        debug_event(
            timing_enabled,
            format!(
                "{doc_type} starting GraphQL source collection {}",
                path.display()
            ),
        );
        let sources = if is_graphql_document(&path) {
            vec![text]
        } else {
            pluck_graphql_sources(&text)
        };
        debug_timing(
            timing_enabled,
            format!(
                "{doc_type} collect GraphQL sources from {} ({} sources)",
                path.display(),
                sources.len()
            ),
            collect_started,
        );

        for source in sources {
            // graphql-parser's AST lifetime is tied to the input buffer, so we promote the buffer
            // to `'static` for storage in `DocumentFile` (mirrors upstream's owned `DocumentNode`).
            let text: &'static str = Box::leak(source.into_boxed_str());

            debug_event(
                timing_enabled,
                format!("{doc_type} starting document parse {}", path.display()),
            );
            let parse_started = Instant::now();
            let document = graphql_parser::parse_query::<String>(text)
                .with_context(|| format!("failed to parse GraphQL document {}", path.display()))?;
            debug_timing(
                timing_enabled,
                format!("{doc_type} parse document {}", path.display()),
                parse_started,
            );

            out.push(DocumentFile {
                location: path.to_string_lossy().to_string(),
                document,
                r#type: Some(doc_type.to_string()),
            });
        }
    }

    for src in inline_graphql {
        let text: &'static str = Box::leak(src.into_boxed_str());
        debug_event(
            timing_enabled,
            format!("{doc_type} starting inline GraphQL parse"),
        );
        let parse_started = Instant::now();
        let document = graphql_parser::parse_query::<String>(text)
            .with_context(|| format!("failed to parse inline GraphQL document `{text}`"))?;
        debug_timing(
            timing_enabled,
            format!("{doc_type} parse inline GraphQL document"),
            parse_started,
        );
        out.push(DocumentFile {
            location: "<inline>".to_string(),
            document,
            r#type: Some(doc_type.to_string()),
        });
    }

    debug_timing(
        timing_enabled,
        format!(
            "{doc_type} load_documents_for_pointers produced {} documents",
            out.len()
        ),
        started,
    );
    Ok(out)
}

/// Loads GraphQL documents from pointers (paths or globs), mirroring the TS `loadDocuments` call site.
///
/// `ignore` is used to avoid loading generated outputs as inputs (upstream derives this from `generates`).
pub async fn load_documents_with_timing(
    cwd: &Path,
    pointers: &[String],
    external_pointers: &[String],
    ignore: &[String],
    timing_enabled: bool,
) -> Result<Vec<DocumentFile>> {
    let mut out = Vec::new();
    out.extend(
        load_documents_for_pointers(cwd, pointers, ignore, "standard", timing_enabled).await?,
    );
    out.extend(
        load_documents_for_pointers(cwd, external_pointers, ignore, "external", timing_enabled)
            .await?,
    );
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

fn load_introspection_json(path: &Path, timing_enabled: bool) -> Result<SchemaGenerationInput> {
    debug_event(
        timing_enabled,
        format!("starting schema JSON read {}", path.display()),
    );
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read schema JSON {}", path.display()))?;
    debug_event(
        timing_enabled,
        format!("starting schema JSON parse {}", path.display()),
    );
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
async fn load_schema_js(path: &Path, timing_enabled: bool) -> Result<SchemaGenerationInput> {
    let abs = path
        .canonicalize()
        .with_context(|| format!("schema file not found: {}", path.display()))?;

    debug_event(
        timing_enabled,
        format!("starting JS schema loader node process {}", abs.display()),
    );
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
    debug_event(
        timing_enabled,
        format!("starting JS schema loader JSON parse {}", abs.display()),
    );
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
async fn load_schema_graphql_file_loader(
    path: &Path,
    timing_enabled: bool,
) -> Result<SchemaGenerationInput> {
    let abs = path
        .canonicalize()
        .with_context(|| format!("schema file not found: {}", path.display()))?;
    debug_event(
        timing_enabled,
        format!("starting SDL schema read {}", abs.display()),
    );
    let sdl = tokio::fs::read_to_string(&abs)
        .await
        .with_context(|| format!("failed to read schema {}", abs.display()))?;
    load_schema_graphql_sdl(
        &sdl,
        abs.parent()
            .context("schema path has no parent directory")?,
        timing_enabled,
    )
    .await
}

async fn load_schema_graphql_sdl(
    sdl: &str,
    cwd: &Path,
    timing_enabled: bool,
) -> Result<SchemaGenerationInput> {
    debug_event(
        timing_enabled,
        format!(
            "starting SDL schema loader node process in {} ({} bytes)",
            cwd.display(),
            sdl.len()
        ),
    );
    let output = tokio::process::Command::new("node")
        .current_dir(cwd)
        .env("CODEGEN_SCHEMA_SDL", sdl)
        .arg("-e")
        .arg(GRAPHQL_FILE_LOADER_SCRIPT_CJS)
        .output()
        .await
        .context("failed to spawn `node` for SDL schema load — is Node installed?")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("failed to load SDL schema: {}", stderr.trim());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    debug_event(timing_enabled, "starting SDL schema loader JSON parse");
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
const { buildSchema, introspectionFromSchema } = require('graphql');

const absPath = process.env.CODEGEN_SCHEMA_PATH;
if (!absPath) {
  process.stderr.write('CODEGEN_SCHEMA_PATH is not set');
  process.exit(1);
}

const dir = path.dirname(absPath);
const req = createRequire(path.join(dir, '_.cjs'));
const mod = req(absPath);
let schema = mod.schema ?? mod.default?.schema ?? mod.default;
if (typeof schema === 'string') {
  schema = buildSchema(schema);
}
if (!schema || typeof schema.getTypeMap !== 'function') {
  process.stderr.write('Expected a GraphQLSchema or SDL string export (schema/default.schema/default) from ' + absPath);
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
const { buildSchema, introspectionFromSchema } = require('graphql');

const absPath = process.env.CODEGEN_SCHEMA_PATH;
let sdl = process.env.CODEGEN_SCHEMA_SDL || (absPath ? require('fs').readFileSync(absPath, 'utf8') : '');
if (!sdl) {
  process.stderr.write('CODEGEN_SCHEMA_SDL is not set');
  process.exit(1);
}

if (/^extend\s+type\s+/m.test(sdl)) {
  sdl = sdl
    .replace(/^extend\s+type\s+/gm, 'type ')
    .replace(/^extend\s+schema[\s\S]*?(?=\n\s*(type|interface|enum|scalar|union|input)\s)/m, '');
}

let schema;
let intro;
try {
  schema = buildSchema(sdl, { assumeValidSDL: true });
  intro = introspectionFromSchema(schema);
} catch (err) {
  if (!String(err && err.message || err).includes('Query root type must be provided')) {
    throw err;
  }
  schema = buildSchema(`${sdl}\n\ntype Query { _empty: String }\nschema { query: Query }`, { assumeValidSDL: true });
  intro = introspectionFromSchema(schema);
  intro.__schema.queryType = null;
  intro.__schema.types = intro.__schema.types.filter(type => type.name !== 'Query');
}
process.stdout.write(JSON.stringify({ __schema: intro.__schema }));
"#;

#[cfg(test)]
mod tests {
    use super::pointer_might_be_inline_graphql;

    #[test]
    fn pointer_might_be_inline_graphql_dev_test_query_string() {
        assert!(pointer_might_be_inline_graphql(
            "query test { testArr1 testArr2 testArr3 }"
        ));
    }

    #[test]
    fn pointer_might_be_inline_graphql_mutation_and_fragment() {
        assert!(pointer_might_be_inline_graphql("mutation M { f }"));
        assert!(pointer_might_be_inline_graphql("fragment F on T { x }"));
    }

    #[test]
    fn pointer_might_be_inline_graphql_rejects_glob() {
        assert!(!pointer_might_be_inline_graphql("./dev-test/**/*.graphql"));
    }
}
