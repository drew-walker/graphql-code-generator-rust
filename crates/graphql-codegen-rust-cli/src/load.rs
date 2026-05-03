//! Mirrors `graphql-code-generator/packages/graphql-codegen-cli/src/load.ts`
//! (`loadSchema` / `loadDocuments`). Schema from JSON, SDL (via Node), or JS; documents from globs
//! or inline GraphQL strings (same as upstream `loadDocuments` + `@graphql-tools/load`).

use anyhow::{Context as _, Result};
use apollo_compiler::Schema as ApolloSchema;
use apollo_compiler::parser::Parser;
use globwalk::GlobWalkerBuilder;
use graphql_parser::schema::{
    Definition as SchemaDefinitionAst, Document as SchemaDocument, EnumType, EnumTypeExtension,
    Field, InputObjectType, InputObjectTypeExtension, InputValue, InterfaceType,
    InterfaceTypeExtension, ObjectType, ObjectTypeExtension, Type as SchemaType, TypeDefinition,
    TypeExtension, UnionType, UnionTypeExtension, Value as SchemaValue, parse_schema,
};
use plugin_helpers::schema_input::SchemaGenerationInput;
use plugin_helpers::types::DocumentFile;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};
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
            let globs = expand_glob_split_pointers(cwd, pointer);
            debug_event(
                timing_enabled,
                format!(
                    "starting schema glob `{pointer}` → {:?}",
                    globs
                        .iter()
                        .map(|g| (g.root.display().to_string(), g.pattern.clone()))
                        .collect::<Vec<_>>()
                ),
            );
            let glob_started = Instant::now();
            let before = files.len();
            for path in expand_globwalk_matches(&globs, "schema", true)? {
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

    let mut parts = Vec::new();
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
        parts.push((file, text));
    }
    load_schema_graphql_files(&parts, timing_enabled)
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
/// True when the pointer needs a filesystem glob expansion.
///
/// Upstream passes `ignore` from `load.ts` into `@graphql-tools/load` as filesystem paths (see
/// `join(config.cwd, generatePath)`). Those are almost always **literal** output paths, not globs.
/// `graphql-tools/packages/loaders/graphql-file` and `code-file` pass `globby([include, …ignores.map(v
/// => "!"+v)])`. When `ignore` is non-empty we mirror that with [`GlobWalkerBuilder::from_patterns`]
/// on `cwd` plus `!` negated patterns (see `buildIgnoreGlob` there).
fn pointer_needs_glob_walk(pointer: &str) -> bool {
    pointer
        .chars()
        .any(|c| matches!(c, '*' | '?' | '[' | ']' | '{' | '}'))
}

struct GlobPointer {
    root: PathBuf,
    pattern: String,
}

/// Micromatch-style `{a,b}` in the **last** path segment (e.g. `**/*.{graphql,gql}`) becomes
/// several [`GlobPointer`] values. Filesystem matching uses [`globwalk`] scoped to the literal
/// prefix before the first glob metacharacter (fast on large repos). Patterns without `**` also
/// use [`glob_path_matches_fixed_depth`] so `src/*.graphqls` does not match `src/__tests__/x`.
fn expand_glob_split_pointers(cwd: &Path, pointer: &str) -> Vec<GlobPointer> {
    let pointer = pointer.strip_prefix("./").unwrap_or(pointer);
    if let Some(alts) = expand_brace_alternatives_in_last_segment(pointer) {
        alts.into_iter()
            .map(|p| glob_root_and_pattern(cwd, &p))
            .collect()
    } else {
        vec![glob_root_and_pattern(cwd, pointer)]
    }
}

fn expand_brace_alternatives_in_last_segment(pointer: &str) -> Option<Vec<String>> {
    let last_start = pointer.rfind('/').map(|i| i + 1).unwrap_or(0);
    let last_seg = pointer.get(last_start..)?;
    let open = last_seg.find('{')?;
    let close = last_seg.rfind('}')?;
    if close <= open + 1 {
        return None;
    }
    let inner = &last_seg[open + 1..close];
    if inner.contains(['{', '}']) {
        return None;
    }
    let opts: Vec<_> = inner
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if opts.len() < 2 {
        return None;
    }
    let prefix = pointer.get(..last_start + open)?;
    let suffix = last_seg.get(close + 1..).unwrap_or("");
    Some(
        opts.into_iter()
            .map(|o| format!("{prefix}{o}{suffix}"))
            .collect(),
    )
}

fn glob_root_and_pattern(cwd: &Path, pointer: &str) -> GlobPointer {
    let pointer = pointer.strip_prefix("./").unwrap_or(pointer);
    let Some(first_glob_idx) = pointer
        .char_indices()
        .find_map(|(idx, c)| matches!(c, '*' | '?' | '[' | ']' | '{' | '}').then_some(idx))
    else {
        return GlobPointer {
            root: cwd.to_path_buf(),
            pattern: pointer.to_string(),
        };
    };

    let literal_prefix = &pointer[..first_glob_idx];
    let Some(separator_idx) = literal_prefix.rfind('/') else {
        return GlobPointer {
            root: cwd.to_path_buf(),
            pattern: pointer.to_string(),
        };
    };

    let (root_part, pattern) = if separator_idx == 0 && pointer.starts_with('/') {
        ("/", &pointer[1..])
    } else {
        (&pointer[..separator_idx], &pointer[separator_idx + 1..])
    };

    let root = if root_part.is_empty() {
        cwd.to_path_buf()
    } else {
        let root = Path::new(root_part);
        if root.is_absolute() {
            root.to_path_buf()
        } else {
            cwd.join(root)
        }
    };

    GlobPointer {
        root,
        pattern: pattern.to_string(),
    }
}

fn normalize_glob_entry_path(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

fn glob_fixed_relative_depth(pattern: &str) -> Option<usize> {
    if pattern.contains("**/") || pattern.starts_with("**") || pattern.ends_with("/**") {
        return None;
    }
    if pattern.contains("**") {
        return None;
    }
    let n = pattern.split('/').filter(|s| !s.is_empty()).count();
    (n > 0).then_some(n)
}

fn glob_path_matches_fixed_depth(root: &Path, full_path: &Path, pattern: &str) -> bool {
    let Some(expected) = glob_fixed_relative_depth(pattern) else {
        return true;
    };
    full_path
        .strip_prefix(root)
        .map(|rel| rel.components().count() == expected)
        .unwrap_or(false)
}

/// Walk `globs` with [`globwalk`]. `apply_fixed_depth_filter` is for schema pointers like
/// `src/*.graphqls` so `*` does not match across `/` (see [`glob_path_matches_fixed_depth`]). For
/// documents we pass `false`: patterns are almost always `**/…`, and depth filtering is inactive
/// then anyway.
///
/// We match **symlink** entries as well as regular files (`pnpm` / `node_modules` are mostly
/// symlinks; `FILE` alone dropped ~85% of matches). We do **not** follow directory symlinks
/// (`follow_links(false)`): `follow_links(true)` can thrash or appear hung on circular links and
/// is not what most tooling does for document discovery.
fn expand_globwalk_matches(
    globs: &[GlobPointer],
    ctx: &str,
    apply_fixed_depth_filter: bool,
) -> Result<Vec<PathBuf>> {
    let mut merged = Vec::new();
    for glob in globs {
        let walker = GlobWalkerBuilder::from_patterns(&glob.root, &[glob.pattern.as_str()])
            .follow_links(false)
            .file_type(globwalk::FileType::FILE | globwalk::FileType::SYMLINK)
            .build()
            .with_context(|| {
                format!(
                    "failed to build {ctx} glob walker (root={}, pattern={})",
                    glob.root.display(),
                    glob.pattern
                )
            })?;
        for entry in walker {
            let entry = entry
                .with_context(|| format!("{ctx} glob walk error under {}", glob.root.display()))?;
            let path = normalize_glob_entry_path(&glob.root, entry.path());
            if apply_fixed_depth_filter
                && !glob_path_matches_fixed_depth(&glob.root, &path, &glob.pattern)
            {
                continue;
            }
            if path.is_file() {
                merged.push(path);
            }
        }
    }
    merged.sort();
    merged.dedup();
    Ok(merged)
}

fn path_to_unix_slashes(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

fn unixify_glob_token(s: &str) -> String {
    s.strip_prefix("./").unwrap_or(s).replace('\\', "/")
}

/// Same as `buildIgnoreGlob(unixify(v))` in `graphql-tools/packages/loaders/graphql-file` / `code-file`.
fn graphql_tools_ignore_negation(cwd: &Path, ex: &str) -> String {
    let ex = ex.strip_prefix("./").unwrap_or(ex);
    let v = if pointer_needs_glob_walk(ex) {
        unixify_glob_token(ex)
    } else {
        path_to_unix_slashes(&resolve_path(cwd, ex))
    };
    format!("!{v}")
}

/// [`GlobWalkerBuilder::from_patterns`] on `cwd` with `[include, "!ignore1", …]` — toolkit `globby` shape.
fn expand_document_globs_graphql_tools_style(
    cwd: &Path,
    pat_norm: &str,
    ignore_negations: &[String],
    doc_type: &str,
) -> Result<Vec<PathBuf>> {
    let pat_norm = pat_norm.strip_prefix("./").unwrap_or(pat_norm);
    let include_variants: Vec<String> = match expand_brace_alternatives_in_last_segment(pat_norm) {
        Some(alts) => alts.into_iter().map(|p| unixify_glob_token(&p)).collect(),
        None => vec![unixify_glob_token(pat_norm)],
    };

    let mut merged = Vec::new();
    for inc in include_variants {
        let mut patterns: Vec<String> = Vec::with_capacity(1 + ignore_negations.len());
        patterns.push(inc);
        patterns.extend_from_slice(ignore_negations);

        let pat_refs: Vec<&str> = patterns.iter().map(|s| s.as_str()).collect();
        let walker = GlobWalkerBuilder::from_patterns(cwd, &pat_refs)
            .follow_links(false)
            .file_type(globwalk::FileType::FILE | globwalk::FileType::SYMLINK)
            .build()
            .with_context(|| {
                format!(
                    "failed to build {doc_type} glob walker (cwd={}, patterns={pat_refs:?})",
                    cwd.display()
                )
            })?;

        for entry in walker {
            let entry = entry
                .with_context(|| format!("{doc_type} glob walk error under {}", cwd.display()))?;
            let path = normalize_glob_entry_path(cwd, entry.path());
            if path.is_file() {
                merged.push(path);
            }
        }
    }
    merged.sort();
    merged.dedup();
    Ok(merged)
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

    let ignore_negations: Vec<String> = exclude
        .iter()
        .map(|e| graphql_tools_ignore_negation(cwd, e.strip_prefix("./").unwrap_or(e)))
        .collect();

    let use_toolkit_glob_merge = !ignore_negations.is_empty();

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

        if !pointer_needs_glob_walk(&pat_norm) {
            if use_toolkit_glob_merge {
                debug_event(
                    timing_enabled,
                    format!(
                        "{doc_type} literal document pointer `{pat_norm}` with {} toolkit ignore pattern(s)",
                        ignore_negations.len()
                    ),
                );
                let glob_started = Instant::now();
                let before = files.len();
                let mut matched_file = false;
                for path in expand_document_globs_graphql_tools_style(
                    cwd,
                    &pat_norm,
                    &ignore_negations,
                    doc_type,
                )? {
                    if !is_graphql_document(&path) && !is_code_document(&path) {
                        continue;
                    }
                    files.push(path);
                    matched_file = true;
                }
                debug_timing(
                    timing_enabled,
                    format!(
                        "{doc_type} literal `{pat_norm}` matched {} candidate file(s)",
                        files.len().saturating_sub(before)
                    ),
                    glob_started,
                );
                if !matched_file && pointer_might_be_inline_graphql(&pat_norm) {
                    inline_graphql.push(pat);
                }
            } else {
                let candidate = resolve_path(cwd, &pat_norm);
                debug_event(
                    timing_enabled,
                    format!(
                        "{doc_type} checking literal document pointer {}",
                        candidate.display()
                    ),
                );
                if candidate.is_file()
                    && (is_graphql_document(&candidate) || is_code_document(&candidate))
                {
                    files.push(candidate);
                }
            }
            continue;
        }

        debug_event(
            timing_enabled,
            format!("{doc_type} preparing document glob `{pat_norm}`"),
        );
        let glob_started = Instant::now();
        let before = files.len();
        let mut matched_file = false;

        if use_toolkit_glob_merge {
            debug_event(
                timing_enabled,
                format!(
                    "{doc_type} document glob `{pat_norm}` merged with {} toolkit ignore pattern(s)",
                    ignore_negations.len()
                ),
            );
            for path in expand_document_globs_graphql_tools_style(
                cwd,
                &pat_norm,
                &ignore_negations,
                doc_type,
            )? {
                if !is_graphql_document(&path) && !is_code_document(&path) {
                    continue;
                }
                files.push(path);
                matched_file = true;
            }
        } else {
            let globs = expand_glob_split_pointers(cwd, &pat_norm);
            debug_event(
                timing_enabled,
                format!(
                    "{doc_type} starting document glob `{pat_norm}` → {:?}",
                    globs
                        .iter()
                        .map(|g| (g.root.display().to_string(), g.pattern.clone()))
                        .collect::<Vec<_>>()
                ),
            );
            for path in expand_globwalk_matches(&globs, doc_type, false)? {
                if !is_graphql_document(&path) && !is_code_document(&path) {
                    continue;
                }
                files.push(path);
                matched_file = true;
            }
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

    let cwd = abs
        .parent()
        .context("schema path has no parent directory")?;
    debug_event(
        timing_enabled,
        format!("starting JS schema loader node process {}", abs.display()),
    );
    let output = tokio::process::Command::new("node")
        .current_dir(cwd)
        .env("CODEGEN_SCHEMA_PATH", abs.as_os_str())
        .arg("-e")
        .arg(SCHEMA_LOAD_SCRIPT_CJS)
        .output()
        .await
        .with_context(|| {
            format!(
                "failed to spawn `node` for JS schema load in {}. PATH={}",
                cwd.display(),
                std::env::var("PATH").unwrap_or_else(|_| "<unset>".to_string())
            )
        })?;

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
/// Implementation note: upstream uses `@graphql-tools/load` + `GraphQLFileLoader`; the Rust port
/// builds the introspection shape natively from the SDL AST.
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
    load_schema_graphql_sdl(&sdl, timing_enabled)
}

fn load_schema_graphql_sdl(sdl: &str, timing_enabled: bool) -> Result<SchemaGenerationInput> {
    debug_event(
        timing_enabled,
        format!(
            "starting apollo-compiler SDL schema parse ({} bytes)",
            sdl.len()
        ),
    );
    let normalized_sdl = normalize_orphan_schema_extensions(&strip_schema_extensions(sdl));
    let apollo_schema = ApolloSchema::parse(&normalized_sdl, "schema.graphql")
        .map_err(|errors| anyhow::anyhow!("failed to parse SDL schema:\n{errors}"))?;
    schema_generation_input_from_apollo_schema(apollo_schema)
}

/// Loads SDL from multiple files using apollo-compiler’s schema builder (one parse per path).
/// Concatenating files into one string and calling [`ApolloSchema::parse`] makes the compiler
/// treat duplicate type names as collisions in a single document; upstream `@graphql-tools/load`
/// merges across files instead.
///
/// We do **not** run [`normalize_orphan_schema_extensions_pass2`] here: that pass promotes
/// orphan `extend type Foo` to `type Foo` when `Foo` is missing from a global name set. Across
/// multiple files, `Foo` is often defined in a later file while an earlier file only extends it;
/// promoting the extension first produces a second `type Foo` when the real definition is parsed,
/// which apollo rejects (`Query` defined multiple times). [`Parser::parse_into_schema_builder`]
/// already keeps orphan extensions until the base type appears.
fn load_schema_graphql_files(
    files: &[(PathBuf, String)],
    timing_enabled: bool,
) -> Result<SchemaGenerationInput> {
    if files.is_empty() {
        anyhow::bail!("load_schema_graphql_files: empty file list");
    }
    if files.len() == 1 {
        return load_schema_graphql_sdl(&files[0].1, timing_enabled);
    }
    let total_bytes: usize = files.iter().map(|(_, s)| s.len()).sum();
    debug_event(
        timing_enabled,
        format!(
            "starting apollo-compiler multi-file SDL schema parse ({} files, {} bytes)",
            files.len(),
            total_bytes
        ),
    );

    let mut builder = ApolloSchema::builder();
    let mut parser = Parser::new();
    for (path, raw) in files {
        let stripped = strip_schema_extensions(raw);
        parser.parse_into_schema_builder(stripped, path, &mut builder);
    }

    let apollo_schema = builder
        .build()
        .map_err(|errors| anyhow::anyhow!("failed to parse SDL schema:\n{errors}"))?;
    schema_generation_input_from_apollo_schema(apollo_schema)
}

fn schema_generation_input_from_apollo_schema(
    apollo_schema: ApolloSchema,
) -> Result<SchemaGenerationInput> {
    let merged_sdl = apollo_schema.serialize().to_string();
    let document = parse_schema::<String>(&merged_sdl)
        .context("failed to parse SDL schema")?
        .into_static();
    Ok(SchemaGenerationInput {
        introspection: schema_document_to_introspection(&document),
        enum_internal_values: HashMap::new(),
    })
}

#[derive(Debug, Clone, Default)]
struct SdlType {
    kind: &'static str,
    description: Option<String>,
    fields: Vec<Field<'static, String>>,
    input_fields: Vec<InputValue<'static, String>>,
    interfaces: Vec<String>,
    enum_values: Vec<graphql_parser::schema::EnumValue<'static, String>>,
    possible_types: Vec<String>,
}

fn strip_schema_extensions(sdl: &str) -> String {
    let mut out = String::new();
    let mut skipping_extend_schema = false;
    for line in sdl.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("extend schema") {
            skipping_extend_schema = true;
            continue;
        }
        if skipping_extend_schema {
            if trimmed.is_empty() || trimmed.starts_with('@') {
                continue;
            }
            skipping_extend_schema = false;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn collect_schema_definition_type_names(sdl: &str) -> HashSet<String> {
    let mut defined = HashSet::new();
    for line in sdl.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("extend ") {
            continue;
        }
        if let Some(name) = type_name_from_definition_line(trimmed) {
            defined.insert(name.to_string());
        }
    }
    defined
}

/// Second pass of orphan-extension normalization: promotes `extend type Foo` to `type Foo` when
/// `Foo` is not in `defined`, then inserts `Foo`. Callers seed `defined` appropriately; for
/// multi-file loads, seed with all base definitions across files, then reuse the same `defined`
/// across files in pointer order so promotions in an earlier file affect later files.
fn normalize_orphan_schema_extensions_pass2(sdl: &str, defined: &mut HashSet<String>) -> String {
    let mut out = String::new();
    for line in sdl.lines() {
        let trimmed = line.trim_start();
        let leading = &line[..line.len() - trimmed.len()];
        if let Some(rest) = trimmed.strip_prefix("extend ")
            && let Some(name) = type_name_from_definition_line(rest)
            && !defined.contains(name)
        {
            defined.insert(name.to_string());
            out.push_str(leading);
            out.push_str(rest);
            out.push('\n');
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn normalize_orphan_schema_extensions(sdl: &str) -> String {
    let mut defined = collect_schema_definition_type_names(sdl);
    normalize_orphan_schema_extensions_pass2(sdl, &mut defined)
}

fn type_name_from_definition_line(line: &str) -> Option<&str> {
    for keyword in ["type", "interface", "input", "enum", "union", "scalar"] {
        if let Some(rest) = line.strip_prefix(keyword) {
            let rest = rest.trim_start();
            let name = rest
                .split(|c: char| c.is_whitespace() || matches!(c, '@' | '=' | '{'))
                .next()
                .filter(|name| !name.is_empty())?;
            return Some(name);
        }
    }
    None
}

fn schema_document_to_introspection(document: &SchemaDocument<'static, String>) -> Value {
    let mut types: BTreeMap<String, SdlType> = BTreeMap::new();
    add_builtin_scalars(&mut types);

    let mut query_type: Option<String> = None;
    let mut mutation_type: Option<String> = None;
    let mut subscription_type: Option<String> = None;

    for definition in &document.definitions {
        match definition {
            SchemaDefinitionAst::SchemaDefinition(schema) => {
                query_type = schema.query.clone();
                mutation_type = schema.mutation.clone();
                subscription_type = schema.subscription.clone();
            }
            SchemaDefinitionAst::TypeDefinition(definition) => {
                add_type_definition(&mut types, definition);
            }
            SchemaDefinitionAst::TypeExtension(extension) => {
                add_type_extension(&mut types, extension);
            }
            SchemaDefinitionAst::DirectiveDefinition(_) => {}
        }
    }

    if query_type.is_none() && types.contains_key("Query") {
        query_type = Some("Query".to_string());
    }
    if mutation_type.is_none() && types.contains_key("Mutation") {
        mutation_type = Some("Mutation".to_string());
    }
    if subscription_type.is_none() && types.contains_key("Subscription") {
        subscription_type = Some("Subscription".to_string());
    }

    add_interface_possible_types(&mut types);

    let type_values = types
        .iter()
        .map(|(name, ty)| sdl_type_to_introspection(name, ty, &types))
        .collect::<Vec<_>>();

    json!({
        "queryType": query_type.map(|name| json!({ "name": name })).unwrap_or(Value::Null),
        "mutationType": mutation_type.map(|name| json!({ "name": name })).unwrap_or(Value::Null),
        "subscriptionType": subscription_type.map(|name| json!({ "name": name })).unwrap_or(Value::Null),
        "types": type_values,
        "directives": [],
    })
}

fn add_builtin_scalars(types: &mut BTreeMap<String, SdlType>) {
    for name in ["String", "Boolean", "Int", "Float", "ID"] {
        types.insert(
            name.to_string(),
            SdlType {
                kind: "SCALAR",
                ..Default::default()
            },
        );
    }
}

fn add_type_definition(
    types: &mut BTreeMap<String, SdlType>,
    definition: &TypeDefinition<'static, String>,
) {
    match definition {
        TypeDefinition::Scalar(scalar) => {
            types.insert(
                scalar.name.clone(),
                SdlType {
                    kind: "SCALAR",
                    description: scalar.description.clone(),
                    ..Default::default()
                },
            );
        }
        TypeDefinition::Object(object) => set_object_type(types, object),
        TypeDefinition::Interface(interface) => set_interface_type(types, interface),
        TypeDefinition::Union(union) => set_union_type(types, union),
        TypeDefinition::Enum(enum_type) => set_enum_type(types, enum_type),
        TypeDefinition::InputObject(input) => set_input_type(types, input),
    }
}

fn add_type_extension(
    types: &mut BTreeMap<String, SdlType>,
    extension: &TypeExtension<'static, String>,
) {
    match extension {
        TypeExtension::Object(object) => extend_object_type(types, object),
        TypeExtension::Interface(interface) => extend_interface_type(types, interface),
        TypeExtension::Union(union) => extend_union_type(types, union),
        TypeExtension::Enum(enum_type) => extend_enum_type(types, enum_type),
        TypeExtension::InputObject(input) => extend_input_type(types, input),
        TypeExtension::Scalar(scalar) => {
            types.entry(scalar.name.clone()).or_insert_with(|| SdlType {
                kind: "SCALAR",
                ..Default::default()
            });
        }
    }
}

fn set_object_type(types: &mut BTreeMap<String, SdlType>, object: &ObjectType<'static, String>) {
    types.insert(
        object.name.clone(),
        SdlType {
            kind: "OBJECT",
            description: object.description.clone(),
            fields: object.fields.clone(),
            interfaces: object.implements_interfaces.clone(),
            ..Default::default()
        },
    );
}

fn extend_object_type(
    types: &mut BTreeMap<String, SdlType>,
    object: &ObjectTypeExtension<'static, String>,
) {
    let entry = types.entry(object.name.clone()).or_insert_with(|| SdlType {
        kind: "OBJECT",
        ..Default::default()
    });
    entry.fields.extend(object.fields.clone());
    merge_names(&mut entry.interfaces, &object.implements_interfaces);
}

fn set_interface_type(
    types: &mut BTreeMap<String, SdlType>,
    interface: &InterfaceType<'static, String>,
) {
    types.insert(
        interface.name.clone(),
        SdlType {
            kind: "INTERFACE",
            description: interface.description.clone(),
            fields: interface.fields.clone(),
            interfaces: interface.implements_interfaces.clone(),
            ..Default::default()
        },
    );
}

fn extend_interface_type(
    types: &mut BTreeMap<String, SdlType>,
    interface: &InterfaceTypeExtension<'static, String>,
) {
    let entry = types
        .entry(interface.name.clone())
        .or_insert_with(|| SdlType {
            kind: "INTERFACE",
            ..Default::default()
        });
    entry.fields.extend(interface.fields.clone());
    merge_names(&mut entry.interfaces, &interface.implements_interfaces);
}

fn set_union_type(types: &mut BTreeMap<String, SdlType>, union: &UnionType<'static, String>) {
    types.insert(
        union.name.clone(),
        SdlType {
            kind: "UNION",
            description: union.description.clone(),
            possible_types: union.types.clone(),
            ..Default::default()
        },
    );
}

fn extend_union_type(
    types: &mut BTreeMap<String, SdlType>,
    union: &UnionTypeExtension<'static, String>,
) {
    let entry = types.entry(union.name.clone()).or_insert_with(|| SdlType {
        kind: "UNION",
        ..Default::default()
    });
    merge_names(&mut entry.possible_types, &union.types);
}

fn set_enum_type(types: &mut BTreeMap<String, SdlType>, enum_type: &EnumType<'static, String>) {
    types.insert(
        enum_type.name.clone(),
        SdlType {
            kind: "ENUM",
            description: enum_type.description.clone(),
            enum_values: enum_type.values.clone(),
            ..Default::default()
        },
    );
}

fn extend_enum_type(
    types: &mut BTreeMap<String, SdlType>,
    enum_type: &EnumTypeExtension<'static, String>,
) {
    let entry = types
        .entry(enum_type.name.clone())
        .or_insert_with(|| SdlType {
            kind: "ENUM",
            ..Default::default()
        });
    entry.enum_values.extend(enum_type.values.clone());
}

fn set_input_type(types: &mut BTreeMap<String, SdlType>, input: &InputObjectType<'static, String>) {
    types.insert(
        input.name.clone(),
        SdlType {
            kind: "INPUT_OBJECT",
            description: input.description.clone(),
            input_fields: input.fields.clone(),
            ..Default::default()
        },
    );
}

fn extend_input_type(
    types: &mut BTreeMap<String, SdlType>,
    input: &InputObjectTypeExtension<'static, String>,
) {
    let entry = types.entry(input.name.clone()).or_insert_with(|| SdlType {
        kind: "INPUT_OBJECT",
        ..Default::default()
    });
    entry.input_fields.extend(input.fields.clone());
}

fn add_interface_possible_types(types: &mut BTreeMap<String, SdlType>) {
    let implementors = types
        .iter()
        .filter(|(_, ty)| ty.kind == "OBJECT")
        .flat_map(|(name, ty)| {
            ty.interfaces
                .iter()
                .map(move |interface| (interface.clone(), name.clone()))
        })
        .collect::<Vec<_>>();

    for (interface, object_name) in implementors {
        if let Some(ty) = types.get_mut(&interface) {
            merge_names(&mut ty.possible_types, &[object_name]);
        }
    }
}

fn sdl_type_to_introspection(name: &str, ty: &SdlType, types: &BTreeMap<String, SdlType>) -> Value {
    json!({
        "kind": ty.kind,
        "name": name,
        "description": ty.description,
        "fields": if matches!(ty.kind, "OBJECT" | "INTERFACE") {
            Value::Array(ty.fields.iter().map(|field| field_to_introspection(field, types)).collect())
        } else {
            Value::Null
        },
        "inputFields": if ty.kind == "INPUT_OBJECT" {
            Value::Array(ty.input_fields.iter().map(|field| input_value_to_introspection(field, types)).collect())
        } else {
            Value::Null
        },
        "interfaces": if matches!(ty.kind, "OBJECT" | "INTERFACE") {
            Value::Array(ty.interfaces.iter().map(|name| named_type_ref(name, types)).collect())
        } else {
            Value::Null
        },
        "enumValues": if ty.kind == "ENUM" {
            Value::Array(ty.enum_values.iter().map(enum_value_to_introspection).collect())
        } else {
            Value::Null
        },
        "possibleTypes": if matches!(ty.kind, "INTERFACE" | "UNION") {
            Value::Array(ty.possible_types.iter().map(|name| named_type_ref(name, types)).collect())
        } else {
            Value::Null
        },
    })
}

fn field_to_introspection(
    field: &Field<'static, String>,
    types: &BTreeMap<String, SdlType>,
) -> Value {
    json!({
        "name": field.name,
        "description": field.description,
        "args": field.arguments.iter().map(|arg| input_value_to_introspection(arg, types)).collect::<Vec<_>>(),
        "type": schema_type_ref(&field.field_type, types),
        "isDeprecated": deprecated_reason(&field.directives).is_some(),
        "deprecationReason": deprecated_reason(&field.directives),
    })
}

fn input_value_to_introspection(
    value: &InputValue<'static, String>,
    types: &BTreeMap<String, SdlType>,
) -> Value {
    json!({
        "name": value.name,
        "description": value.description,
        "type": schema_type_ref(&value.value_type, types),
        "defaultValue": value.default_value.as_ref().map(schema_value_to_string),
        "isDeprecated": deprecated_reason(&value.directives).is_some(),
        "deprecationReason": deprecated_reason(&value.directives),
    })
}

fn enum_value_to_introspection(
    value: &graphql_parser::schema::EnumValue<'static, String>,
) -> Value {
    json!({
        "name": value.name,
        "description": value.description,
        "isDeprecated": deprecated_reason(&value.directives).is_some(),
        "deprecationReason": deprecated_reason(&value.directives),
    })
}

fn schema_type_ref(t: &SchemaType<'static, String>, types: &BTreeMap<String, SdlType>) -> Value {
    match t {
        SchemaType::NamedType(name) => named_type_ref(name, types),
        SchemaType::ListType(inner) => json!({
            "kind": "LIST",
            "name": Value::Null,
            "ofType": schema_type_ref(inner, types),
        }),
        SchemaType::NonNullType(inner) => json!({
            "kind": "NON_NULL",
            "name": Value::Null,
            "ofType": schema_type_ref(inner, types),
        }),
    }
}

fn named_type_ref(name: &str, types: &BTreeMap<String, SdlType>) -> Value {
    json!({
        "kind": types.get(name).map(|ty| ty.kind).unwrap_or("SCALAR"),
        "name": name,
        "ofType": Value::Null,
    })
}

fn deprecated_reason(
    directives: &[graphql_parser::schema::Directive<'static, String>],
) -> Option<String> {
    directives
        .iter()
        .find(|directive| directive.name == "deprecated")
        .map(|directive| {
            directive
                .arguments
                .iter()
                .find_map(|(name, value)| {
                    (name == "reason").then(|| match value {
                        SchemaValue::String(reason) => reason.clone(),
                        _ => schema_value_to_string(value),
                    })
                })
                .unwrap_or_else(|| "No longer supported".to_string())
        })
}

fn schema_value_to_string(value: &SchemaValue<'static, String>) -> String {
    match value {
        SchemaValue::Variable(name) => format!("${name}"),
        SchemaValue::Int(value) => value.as_i64().unwrap_or_default().to_string(),
        SchemaValue::Float(value) => value.to_string(),
        SchemaValue::String(value) => format!("\"{}\"", value.replace('"', "\\\"")),
        SchemaValue::Boolean(value) => value.to_string(),
        SchemaValue::Null => "null".to_string(),
        SchemaValue::Enum(value) => value.clone(),
        SchemaValue::List(values) => format!(
            "[{}]",
            values
                .iter()
                .map(schema_value_to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        SchemaValue::Object(values) => format!(
            "{{{}}}",
            values
                .iter()
                .map(|(name, value)| format!("{name}: {}", schema_value_to_string(value)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn merge_names(target: &mut Vec<String>, names: &[String]) {
    for name in names {
        if !target.contains(name) {
            target.push(name.clone());
        }
    }
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

#[cfg(test)]
mod tests {
    use super::{
        expand_brace_alternatives_in_last_segment, expand_glob_split_pointers,
        expand_globwalk_matches, glob_fixed_relative_depth, glob_path_matches_fixed_depth,
        glob_root_and_pattern, load_schema_graphql_files, normalize_orphan_schema_extensions,
        pointer_might_be_inline_graphql,
    };
    use std::path::{Path, PathBuf};

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

    #[test]
    fn normalize_orphan_schema_extensions_promotes_first_extension() {
        let normalized = normalize_orphan_schema_extensions(
            r#"
extend type Query {
  users: [User]
}

extend type User {
  id: ID!
}

extend type User {
  name: String
}
"#,
        );

        assert!(normalized.contains("type Query {\n  users: [User]\n}"));
        assert!(normalized.contains("type User {\n  id: ID!\n}"));
        assert!(normalized.contains("extend type User {\n  name: String\n}"));
    }

    #[test]
    fn glob_root_and_pattern_splits_relative_glob_at_literal_prefix() {
        let g = glob_root_and_pattern(Path::new("/repo"), "src/**/*.graphql");
        assert_eq!(g.root, Path::new("/repo/src"));
        assert_eq!(g.pattern, "**/*.graphql");
    }

    #[test]
    fn expand_brace_splits_micromatch_style_last_segment() {
        assert_eq!(
            expand_brace_alternatives_in_last_segment("a/**/*.{graphql,gql}"),
            Some(vec!["a/**/*.graphql".to_string(), "a/**/*.gql".to_string(),])
        );
    }

    #[test]
    fn glob_fixed_relative_depth_star_suffix_is_one_level() {
        assert_eq!(glob_fixed_relative_depth("*.graphqls"), Some(1));
        assert_eq!(glob_fixed_relative_depth("src/*.graphqls"), Some(2));
        assert_eq!(glob_fixed_relative_depth("**/a.graphql"), None);
    }

    #[test]
    fn globwalk_single_star_does_not_match_nested_dir() {
        use std::fs;
        let dir = std::env::temp_dir().join(format!("graphql_codegen_glob_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("__tests__")).unwrap();
        fs::write(dir.join("root.graphqls"), "scalar S").unwrap();
        fs::write(dir.join("__tests__/nested.graphqls"), "scalar T").unwrap();
        let globs = expand_glob_split_pointers(&dir, "*.graphqls");
        let paths = expand_globwalk_matches(&globs, "test", true).unwrap();
        assert_eq!(paths.len(), 1);
        assert!(paths[0].ends_with("root.graphqls"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn document_glob_merged_negations_like_graphql_file_loader() {
        use std::fs;
        let dir = std::env::temp_dir().join(format!("graphql_codegen_neg_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("keep.graphql"), "query K { x }").unwrap();
        fs::write(dir.join("drop.query.graphql"), "query D { x }").unwrap();
        fs::write(dir.join("keep2.graphql"), "query K2 { x }").unwrap();
        let neg = super::graphql_tools_ignore_negation(&dir, "*.query.graphql");
        let paths =
            super::expand_document_globs_graphql_tools_style(&dir, "*.graphql", &[neg], "test")
                .unwrap();
        assert_eq!(paths.len(), 2);
        let names: Vec<String> = paths
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&"keep.graphql".to_string()));
        assert!(names.contains(&"keep2.graphql".to_string()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn glob_path_matches_fixed_depth_rejects_nested_for_single_star() {
        let root = Path::new("/repo/packages/graphql/src");
        assert!(glob_path_matches_fixed_depth(
            root,
            Path::new("/repo/packages/graphql/src/schema.graphqls"),
            "*.graphqls"
        ));
        assert!(!glob_path_matches_fixed_depth(
            root,
            Path::new("/repo/packages/graphql/src/__tests__/mockSchema.graphqls"),
            "*.graphqls"
        ));
    }

    /// Concatenating these into one SDL makes apollo-compiler reject a duplicate `AccountStatus`
    /// definition; loading as separate files merges `extend enum` into the base enum.
    #[test]
    fn load_schema_graphql_files_merges_enum_extension_across_files() {
        let files = vec![
            (
                PathBuf::from("schema.graphql"),
                "type Query { _: Boolean }\nenum AccountStatus { OPEN }\n".to_string(),
            ),
            (
                PathBuf::from("extensions.graphql"),
                "extend enum AccountStatus { CLOSED }\n".to_string(),
            ),
        ];
        let out = load_schema_graphql_files(&files, false).expect("multi-file schema");
        let types = out.introspection["types"].as_array().expect("types array");
        let account_status = types
            .iter()
            .find(|t| t["name"] == "AccountStatus")
            .expect("AccountStatus type");
        let names: Vec<_> = account_status["enumValues"]
            .as_array()
            .expect("enumValues")
            .iter()
            .map(|v| v["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"OPEN"));
        assert!(names.contains(&"CLOSED"));
    }

    /// Orphan `extend type Query` in an earlier file must not be promoted to `type Query` when the
    /// base `type Query` lives in a later file (would duplicate `Query` in apollo-compiler).
    #[test]
    fn load_schema_graphql_files_extend_root_type_before_base_definition() {
        let files = vec![
            (
                PathBuf::from("extensions.graphql"),
                "extend type Query { ext: String }\n".to_string(),
            ),
            (
                PathBuf::from("root.graphql"),
                "type Query { root: Int }\n".to_string(),
            ),
        ];
        let out = load_schema_graphql_files(&files, false).expect("merged schema");
        let types = out.introspection["types"].as_array().expect("types");
        let query = types
            .iter()
            .find(|t| t["name"] == "Query")
            .expect("Query type");
        let names: Vec<_> = query["fields"]
            .as_array()
            .expect("fields")
            .iter()
            .map(|f| f["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"ext"));
        assert!(names.contains(&"root"));
    }
}
