use std::path::Path;
use std::process::Stdio;

use anyhow::Context as _;
use plugin_helpers::schema_input::SchemaGenerationInput;
use plugin_helpers::types::{ComplexPluginOutput, DocumentFile};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use visitor_plugin_common::client_side_base_visitor::print_document_json;

const JS_PLUGIN_HOST_SCRIPT: &str = r#"
import { createRequire } from 'node:module';
import { pathToFileURL } from 'node:url';
import path from 'node:path';
import readline from 'node:readline';

const cwd = process.cwd();
const requireFromCwd = createRequire(path.join(cwd, '__codegen_plugin_host__.cjs'));
const graphql = requireFromCwd('graphql');
const pluginCache = new Map();
let cachedSchema = null;

// Global state for near-operation optimization
let allDocuments = [];
let fragmentIndex = new Map();

function normalizeDocumentPayload(doc) {
  if (doc.rawSDL && typeof doc.rawSDL === 'string') {
    return {
      location: doc.location,
      document: graphql.parse(doc.rawSDL),
      rawSDL: undefined,
    };
  }
  return {
    location: doc.location,
    document: doc.document,
    rawSDL: undefined,
  };
}

function buildFragmentIndex(documents) {
  const index = new Map();
  for (let i = 0; i < documents.length; i++) {
    const doc = documents[i];
    if (doc.document && doc.document.definitions) {
      for (const def of doc.document.definitions) {
        if (def.kind === 'FragmentDefinition') {
          index.set(def.name.value, i);
        }
      }
    }
  }
  return index;
}

function collectFragmentSpreads(node) {
  const spreads = new Set();
  function visit(n) {
    if (!n) return;
    if (n.kind === 'FragmentSpread') {
      spreads.add(n.name.value);
    }
    if (n.selectionSet) {
      for (const sel of n.selectionSet.selections) {
        visit(sel);
      }
    }
    if (n.definitions) {
      for (const def of n.definitions) {
        visit(def);
      }
    }
  }
  visit(node);
  return Array.from(spreads);
}

function buildClosureIndices(rootIndex, fragmentIndex) {
  const closure = new Set([rootIndex]);
  const pending = collectFragmentSpreads(allDocuments[rootIndex].document);
  
  while (pending.length > 0) {
    const spreadName = pending.pop();
    const fragIdx = fragmentIndex.get(spreadName);
    if (fragIdx !== undefined && !closure.has(fragIdx)) {
      closure.add(fragIdx);
      pending.push(...collectFragmentSpreads(allDocuments[fragIdx].document));
    }
  }
  return Array.from(closure).sort((a, b) => a - b);
}

function normalizeOutput(result) {
  const normalizeTextList = (value) => {
    if (!Array.isArray(value)) {
      return [];
    }
    const out = [];
    for (const entry of value) {
      if (typeof entry === 'string') {
        out.push(entry);
        continue;
      }
      if (entry && typeof entry === 'object' && typeof entry.content === 'string') {
        out.push(entry.content);
      }
    }
    return out;
  };

  if (typeof result === 'string') {
    return { content: result, prepend: [], append: [] };
  }
  return {
    content: typeof result?.content === 'string' ? result.content : '',
    prepend: normalizeTextList(result?.prepend),
    append: normalizeTextList(result?.append),
  };
}

function hydrateExternalFragments(config) {
  if (!config || typeof config !== 'object') {
    return config;
  }
  const external = config.externalFragments;
  if (!Array.isArray(external)) {
    return config;
  }

  const hydrated = external.map((fragment) => {
    if (!fragment || typeof fragment !== 'object') {
      return fragment;
    }
    if (fragment.node && typeof fragment.node === 'object' && fragment.node.kind) {
      return fragment;
    }
    if (typeof fragment.rawSDL === 'string' && typeof fragment.name === 'string') {
      try {
        const parsed = graphql.parse(fragment.rawSDL);
        const node = parsed.definitions.find(
          (def) => def.kind === 'FragmentDefinition' && def.name?.value === fragment.name,
        );
        if (node) {
          return { ...fragment, node };
        }
      } catch {
        return fragment;
      }
    }
    return fragment;
  });

  return {
    ...config,
    externalFragments: hydrated,
  };
}

async function loadPlugin(pluginName) {
  if (pluginCache.has(pluginName)) {
    return pluginCache.get(pluginName);
  }
  const candidates = pluginName.startsWith('@') || pluginName.startsWith('.') || pluginName.startsWith('/')
    ? [pluginName]
    : [`@graphql-codegen/${pluginName}`, pluginName];
  let resolved;
  let lastError;
  for (const candidate of candidates) {
    try {
      resolved = requireFromCwd.resolve(candidate);
      break;
    } catch (error) {
      lastError = error;
    }
  }
  if (!resolved) {
    throw lastError ?? new Error(`Unable to resolve plugin ${pluginName}`);
  }
  const mod = await import(pathToFileURL(resolved).href);
  const plugin = mod.plugin ?? mod.default?.plugin ?? mod.default;
  if (typeof plugin !== 'function') {
    throw new Error(`Plugin ${pluginName} does not export a callable plugin function`);
  }
  pluginCache.set(pluginName, plugin);
  return plugin;
}

async function runPlugin(request) {
  const plugin = await loadPlugin(request.pluginName);
  const pluginConfig = hydrateExternalFragments(request.pluginConfig);
  const schema = cachedSchema
    ?? (request.schema && request.schema.introspection
      ? graphql.buildClientSchema({ __schema: request.schema.introspection })
      : null);
  if (!schema) {
    throw new Error('Schema is not initialized for plugin execution');
  }
  
  // Handle two cases: indices-based (near-operation-file) or direct documents (regular outputs)
  let documents;
  if (request.documentIndices && request.documentIndices.length > 0) {
    // Near-operation-file: use indices to look up documents from allDocuments
    documents = request.documentIndices.map(idx => ({
      location: allDocuments[idx].location,
      document: allDocuments[idx].document,
      rawSDL: undefined,
    }));
  } else if (request.documents && request.documents.length > 0) {
    // Regular outputs: documents sent directly
    documents = request.documents.map((doc) => normalizeDocumentPayload(doc));
  } else {
    // Fallback: no documents provided
    documents = [];
  }
  
  const result = await plugin(schema, documents, pluginConfig, {
    outputFile: request.filename,
    allPlugins: request.allPlugins,
    pluginContext: request.pluginContext ?? {},
    config: request.outputConfig,
  });
  return normalizeOutput(result);
}

const rl = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });
for await (const line of rl) {
  if (!line.trim()) {
    continue;
  }

  let request;
  try {
    request = JSON.parse(line);
    if (request.command === 'init') {
      allDocuments = (request.documents || []).map((doc) => normalizeDocumentPayload(doc));
      fragmentIndex = buildFragmentIndex(allDocuments);
      if (request.schema && request.schema.introspection) {
        cachedSchema = graphql.buildClientSchema({ __schema: request.schema.introspection });
      }
      process.stdout.write(`${JSON.stringify({ id: request.id, ok: true })}\n`);
      continue;
    }
    if (request.command === 'runPlugin') {
      const result = await runPlugin(request);
      process.stdout.write(`${JSON.stringify({ id: request.id, ok: true, result })}\n`);
      continue;
    }
    throw new Error(`Unsupported command: ${request.command}`);
  } catch (error) {
    process.stdout.write(`${JSON.stringify({
      id: request?.id ?? null,
      ok: false,
      error: error?.stack ?? String(error),
    })}\n`);
  }
}
"#;

#[derive(Debug)]
pub struct JsPluginHost {
    _child: Child,
    stdin: ChildStdin,
    stdout: Lines<BufReader<ChildStdout>>,
    next_id: u64,
}

pub struct RunPluginParams<'a> {
    pub plugin_name: &'a str,
    pub filename: &'a str,
    pub all_plugins: Vec<serde_json::Value>,
    pub plugin_config: serde_json::Map<String, serde_json::Value>,
    pub output_config: serde_json::Map<String, serde_json::Value>,
    pub plugin_context: serde_json::Map<String, serde_json::Value>,
    pub schema: &'a SchemaGenerationInput,
    pub documents: &'a [DocumentFile],
    pub document_indices: Vec<usize>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsPluginRequest<'a> {
    id: u64,
    command: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    plugin_name: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    filename: Option<&'a str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    all_plugins: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "serde_json::Map::is_empty")]
    plugin_config: serde_json::Map<String, serde_json::Value>,
    #[serde(skip_serializing_if = "serde_json::Map::is_empty")]
    output_config: serde_json::Map<String, serde_json::Value>,
    #[serde(skip_serializing_if = "serde_json::Map::is_empty")]
    plugin_context: serde_json::Map<String, serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    schema: Option<JsSchemaPayload<'a>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    documents: Vec<JsDocumentPayload>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    document_indices: Vec<usize>,
}

#[derive(Serialize)]
struct JsSchemaPayload<'a> {
    introspection: &'a serde_json::Value,
}

#[derive(Serialize)]
struct JsDocumentPayload {
    location: String,
    document: serde_json::Value,
    #[serde(rename = "rawSDL", skip_serializing_if = "Option::is_none")]
    raw_sdl: Option<String>,
}

#[derive(Deserialize)]
struct JsPluginResponse {
    id: Option<u64>,
    ok: bool,
    result: Option<JsPluginResult>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct JsPluginResult {
    content: String,
    prepend: Vec<String>,
    append: Vec<String>,
}

impl JsPluginHost {
    pub async fn spawn(cwd: &Path) -> anyhow::Result<Self> {
        let mut child = Command::new("node")
            .current_dir(cwd)
            .args([
                "--experimental-strip-types",
                "--input-type=module",
                "-e",
                JS_PLUGIN_HOST_SCRIPT,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .context("Failed to spawn persistent Node plugin host")?;

        let stdin = child
            .stdin
            .take()
            .context("Node plugin host missing stdin")?;
        let stdout = child
            .stdout
            .take()
            .context("Node plugin host missing stdout")?;

        Ok(Self {
            _child: child,
            stdin,
            stdout: BufReader::new(stdout).lines(),
            next_id: 1,
        })
    }

    pub async fn init(
        &mut self,
        documents: &[DocumentFile],
        schema: &SchemaGenerationInput,
    ) -> anyhow::Result<()> {
        let request_id = self.next_id;
        self.next_id += 1;

        let js_documents = serialize_documents(documents, true)?;
        let request = JsPluginRequest {
            id: request_id,
            command: "init",
            plugin_name: None,
            filename: None,
            all_plugins: vec![],
            plugin_config: Default::default(),
            output_config: Default::default(),
            plugin_context: Default::default(),
            schema: Some(JsSchemaPayload {
                introspection: &schema.introspection,
            }),
            documents: js_documents,
            document_indices: vec![],
        };

        let line = serde_json::to_string(&request)?;
        self.stdin.write_all(line.as_bytes()).await?;
        self.stdin.write_all(b"\n").await?;
        self.stdin.flush().await?;

        let response_line = self
            .stdout
            .next_line()
            .await?
            .context("Node plugin host exited before responding to init")?;
        let response: JsPluginResponse = serde_json::from_str(&response_line)
            .context("Failed to parse Node plugin host init response")?;

        if response.id != Some(request_id) {
            anyhow::bail!(
                "Node plugin host init response id mismatch: expected {request_id}, got {:?}",
                response.id
            );
        }

        if !response.ok {
            anyhow::bail!(
                "Node plugin host init failed: {}",
                response
                    .error
                    .unwrap_or_else(|| "unknown error".to_string())
            );
        }

        Ok(())
    }

    pub async fn run_plugin(
        &mut self,
        params: RunPluginParams<'_>,
    ) -> anyhow::Result<ComplexPluginOutput> {
        let RunPluginParams {
            plugin_name,
            filename,
            all_plugins,
            plugin_config,
            output_config,
            plugin_context,
            schema,
            documents,
            document_indices,
        } = params;

        let request_id = self.next_id;
        self.next_id += 1;

        let js_documents = if document_indices.is_empty() {
            serialize_documents(documents, false)?
        } else {
            vec![]
        };

        let request = JsPluginRequest {
            id: request_id,
            command: "runPlugin",
            plugin_name: Some(plugin_name),
            filename: Some(filename),
            all_plugins,
            plugin_config,
            output_config,
            plugin_context,
            schema: if document_indices.is_empty() {
                Some(JsSchemaPayload {
                    introspection: &schema.introspection,
                })
            } else {
                None
            },
            documents: js_documents,
            document_indices,
        };

        let line = serde_json::to_string(&request)?;
        self.stdin.write_all(line.as_bytes()).await?;
        self.stdin.write_all(b"\n").await?;
        self.stdin.flush().await?;

        let response_line = self
            .stdout
            .next_line()
            .await?
            .context("Node plugin host exited before responding")?;
        let response: JsPluginResponse = serde_json::from_str(&response_line)
            .context("Failed to parse Node plugin host response")?;

        if response.id != Some(request_id) {
            anyhow::bail!(
                "Node plugin host response id mismatch: expected {request_id}, got {:?}",
                response.id
            );
        }

        if !response.ok {
            anyhow::bail!(
                "JS plugin `{plugin_name}` failed for `{filename}`: {}",
                response
                    .error
                    .unwrap_or_else(|| "unknown error".to_string())
            );
        }

        let result = response
            .result
            .context("Node plugin host returned no result payload")?;
        Ok(ComplexPluginOutput {
            content: result.content,
            prepend: result.prepend,
            append: result.append,
        })
    }
}

fn serialize_documents(
    documents: &[DocumentFile],
    include_raw_sdl: bool,
) -> anyhow::Result<Vec<JsDocumentPayload>> {
    documents
        .iter()
        .map(|document| {
            let ast_json = print_document_json(&document.document);
            let raw_sdl = if include_raw_sdl {
                read_raw_sdl_if_graphql(&document.location)
            } else {
                None
            };
            Ok(JsDocumentPayload {
                location: document.location.clone(),
                document: serde_json::from_str(&ast_json)
                    .context("Failed to serialize document AST for JS plugin host")?,
                raw_sdl,
            })
        })
        .collect()
}

fn read_raw_sdl_if_graphql(location: &str) -> Option<String> {
    let path = Path::new(location);
    let extension = path.extension().and_then(|ext| ext.to_str())?;
    if extension != "graphql" && extension != "gql" && extension != "graphqls" {
        return None;
    }
    std::fs::read_to_string(path).ok()
}
