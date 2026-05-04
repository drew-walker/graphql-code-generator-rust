use std::path::Path;
use std::time::Instant;

use crate::config::CodegenContext;
use crate::relay_optimize;
use crate::transitional_plugins;
use crate::utils::debugging::{debug_event, debug_timing};
use graphql_tools_load::load_documents_with_timing;
use plugin_fragment_matcher::FragmentMatcherConfig;
use plugin_helpers::types::DocumentFile;
use plugin_helpers::types::{ComplexPluginOutput, Config, FileOutput, OutputConfig, PluginSpec};
use plugin_helpers::utils::{merge_complex_plugin_output, merge_outputs};
use plugin_typed_document_node::TypeScriptTypedDocumentNodesConfig;
use plugin_typescript_react_apollo::TypeScriptReactApolloConfig;

#[derive(Debug)]
pub struct ExecuteCodegenOutput {
    pub result: Vec<FileOutput>,
    pub error: Option<anyhow::Error>,
}

pub async fn execute_codegen(context: &mut CodegenContext) -> ExecuteCodegenOutput {
    let config = context.get_config();
    let mut result: Vec<FileOutput> = Vec::new();
    let debug_timing_enabled = config.debug.unwrap_or(false)
        || config.verbose.unwrap_or(false)
        || context.flags.profile
        || std::env::var_os("CODEGEN_TIMING").is_some();

    let normalize_started = Instant::now();
    let generates = match normalize(&config) {
        Ok(g) => g,
        Err(error) => {
            return ExecuteCodegenOutput {
                result,
                error: Some(error),
            };
        }
    };
    debug_timing(
        debug_timing_enabled,
        format!("normalize config ({} outputs)", generates.len()),
        normalize_started,
    );

    // `packages/graphql-codegen-cli/src/load.ts` `loadDocuments`: `ignore` is built from
    // `Object.keys(config.generates)` but **skips** entries where `path.extname(generatePath) === ''`
    // (directory / preset roots, not a concrete output file).
    let ignore_documents: Vec<String> = config
        .generates
        .keys()
        .filter(|p| Path::new(p).extension().is_some())
        .cloned()
        .collect();

    for (filename, output_config) in generates {
        let output_started = Instant::now();
        debug_event(debug_timing_enabled, format!("starting output {filename}"));

        // TS executeCodegen per-output pipeline: load schema → load documents → generate.
        let mut schema_pointers: Vec<String> = Vec::new();
        schema_pointers.extend(config.schema.clone());
        schema_pointers.extend(output_config.schema.clone());

        let schema_started = Instant::now();
        debug_event(
            debug_timing_enabled,
            format!("starting schema load for {filename} ({schema_pointers:?})"),
        );
        let schema_input = match context
            .load_schema_with_timing(&schema_pointers, debug_timing_enabled)
            .await
        {
            Ok(s) => s,
            Err(e) => {
                return ExecuteCodegenOutput {
                    result,
                    error: Some(e),
                };
            }
        };
        debug_timing(
            debug_timing_enabled,
            format!("load schema for {filename} ({schema_pointers:?})"),
            schema_started,
        );

        let mut document_pointers: Vec<String> = Vec::new();
        document_pointers.extend(config.documents.clone());
        document_pointers.extend(output_config.documents.clone());

        let mut external_document_pointers: Vec<String> = Vec::new();
        external_document_pointers.extend(config.external_documents.clone());
        external_document_pointers.extend(output_config.external_documents.clone());

        let documents_started = Instant::now();
        debug_event(
            debug_timing_enabled,
            format!(
                "starting document load for {filename} (documents={document_pointers:?}, external={external_document_pointers:?})"
            ),
        );
        let documents = match load_documents_with_timing(
            &context.cwd,
            &document_pointers,
            &external_document_pointers,
            &ignore_documents,
            debug_timing_enabled,
        )
        .await
        {
            Ok(d) => d,
            Err(e) => {
                return ExecuteCodegenOutput {
                    result,
                    error: Some(e),
                };
            }
        };
        debug_timing(
            debug_timing_enabled,
            format!(
                "load documents for {filename} ({} standard/external docs)",
                documents.len()
            ),
            documents_started,
        );

        if output_config.preset.as_deref() == Some("near-operation-file") {
            let preset_started = Instant::now();
            let extension = output_config
                .preset_config
                .get("extension")
                .and_then(|value| value.as_str())
                .unwrap_or(".generated.ts");

            // Near-operation-file hot path: initialize JS host once with all documents,
            // then dispatch per-file plugin requests by document indices.
            match context.js_plugin_host(debug_timing_enabled).await {
                Ok(host) => {
                    if let Err(e) = host.init(&documents, &schema_input).await {
                        return ExecuteCodegenOutput {
                            result,
                            error: Some(e),
                        };
                    }
                }
                Err(e) => {
                    return ExecuteCodegenOutput {
                        result,
                        error: Some(e),
                    };
                }
            }

            // Build a map of fragment name -> document index for efficient lookup
            let mut fragment_index: std::collections::HashMap<String, usize> =
                std::collections::HashMap::new();
            for (doc_idx, doc) in documents.iter().enumerate() {
                for def in &doc.document.definitions {
                    if let graphql_parser::query::Definition::Fragment(frag) = def {
                        fragment_index.insert(frag.name.clone(), doc_idx);
                    }
                }
            }

            let spreads_by_doc: Vec<Vec<String>> = documents
                .iter()
                .map(|doc| collect_all_fragment_spreads(&doc.document))
                .collect();

            for (document_index, document) in documents.iter().enumerate() {
                let derived_filename =
                    near_operation_output_filename(&document.location, extension);
                let mut closure_seen = std::collections::HashSet::from([document_index]);
                let mut closure_vec: Vec<usize> = vec![document_index];
                let mut pending: Vec<String> = spreads_by_doc[document_index]
                    .iter()
                    .rev()
                    .cloned()
                    .collect();

                while let Some(spread_name) = pending.pop() {
                    if let Some(&frag_idx) = fragment_index.get(&spread_name)
                        && closure_seen.insert(frag_idx)
                    {
                        closure_vec.push(frag_idx);
                        for nested in spreads_by_doc[frag_idx].iter().rev() {
                            pending.push(nested.clone());
                        }
                    }
                }

                match execute_output(ExecuteOutputParams {
                    context,
                    config: &config,
                    output_base_path: &filename,
                    filename: &derived_filename,
                    output_config: &output_config,
                    schema_input: &schema_input,
                    documents: &documents,
                    result: &mut result,
                    debug_timing_enabled,
                    document_indices: Some(closure_vec),
                    root_document_index: Some(document_index),
                })
                .await
                {
                    Ok(()) => {}
                    Err(error) => {
                        return ExecuteCodegenOutput {
                            result,
                            error: Some(error),
                        };
                    }
                }
            }

            debug_timing(
                debug_timing_enabled,
                format!(
                    "expand preset output {filename} into {} files",
                    documents.len()
                ),
                preset_started,
            );
            debug_timing(
                debug_timing_enabled,
                format!("complete output {filename}"),
                output_started,
            );
            continue;
        }

        match execute_output(ExecuteOutputParams {
            context,
            config: &config,
            output_base_path: &filename,
            filename: &filename,
            output_config: &output_config,
            schema_input: &schema_input,
            documents: &documents,
            result: &mut result,
            debug_timing_enabled,
            document_indices: None,
            root_document_index: None,
        })
        .await
        {
            Ok(()) => {}
            Err(error) => {
                return ExecuteCodegenOutput {
                    result,
                    error: Some(error),
                };
            }
        }

        debug_timing(
            debug_timing_enabled,
            format!("complete output {filename}"),
            output_started,
        );
    }

    ExecuteCodegenOutput {
        result,
        error: None,
    }
}

fn near_operation_output_filename(document_location: &str, extension: &str) -> String {
    let path = Path::new(document_location);
    let parent = path.parent().unwrap_or_else(|| Path::new(""));
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("generated");
    parent
        .join(format!("{stem}{extension}"))
        .to_string_lossy()
        .to_string()
}

fn collect_all_fragment_spreads(document: &plugin_helpers::types::DocumentNode) -> Vec<String> {
    let mut spread_names = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for definition in &document.definitions {
        match definition {
            graphql_parser::query::Definition::Operation(op) => {
                let ss = match op {
                    graphql_parser::query::OperationDefinition::Query(q) => &q.selection_set,
                    graphql_parser::query::OperationDefinition::Mutation(m) => &m.selection_set,
                    graphql_parser::query::OperationDefinition::Subscription(s) => &s.selection_set,
                    graphql_parser::query::OperationDefinition::SelectionSet(ss) => ss,
                };
                collect_spreads_from_selection_set(ss, &mut seen, &mut spread_names);
            }
            graphql_parser::query::Definition::Fragment(frag) => {
                collect_spreads_from_selection_set(
                    &frag.selection_set,
                    &mut seen,
                    &mut spread_names,
                );
            }
        }
    }
    spread_names
}

fn collect_spreads_from_selection_set(
    ss: &graphql_parser::query::SelectionSet<'static, String>,
    seen: &mut std::collections::HashSet<String>,
    spread_names: &mut Vec<String>,
) {
    for sel in &ss.items {
        match sel {
            graphql_parser::query::Selection::FragmentSpread(spread) => {
                if seen.insert(spread.fragment_name.clone()) {
                    spread_names.push(spread.fragment_name.clone());
                }
            }
            graphql_parser::query::Selection::Field(field) => {
                collect_spreads_from_selection_set(&field.selection_set, seen, spread_names);
            }
            graphql_parser::query::Selection::InlineFragment(inline) => {
                collect_spreads_from_selection_set(&inline.selection_set, seen, spread_names);
            }
        }
    }
}

fn build_external_fragments_config(
    documents: &[DocumentFile],
    closure_indices: &[usize],
    root_document_index: usize,
) -> serde_json::Value {
    let mut external_fragments = Vec::new();

    for idx in closure_indices {
        if *idx == root_document_index {
            continue;
        }
        let Some(doc) = documents.get(*idx) else {
            continue;
        };

        for def in &doc.document.definitions {
            let graphql_parser::query::Definition::Fragment(fragment) = def else {
                continue;
            };

            let raw_sdl = std::fs::read_to_string(&doc.location).ok();

            let on_type = match &fragment.type_condition {
                graphql_parser::query::TypeCondition::On(type_name) => type_name.clone(),
            };

            external_fragments.push(serde_json::json!({
                "level": 0,
                "isExternal": true,
                "name": fragment.name,
                "onType": on_type,
                "node": serde_json::Value::Null,
                "location": doc.location,
                "rawSDL": raw_sdl,
            }));
        }
    }

    serde_json::Value::Array(external_fragments)
}

fn supports_single_root_near_operation(plugin_name: &str) -> bool {
    plugin_name == "typescript-operations" || plugin_name.contains("typescript-react-apollo")
}

fn native_plugin_requested(context: &CodegenContext, plugin_name: &str) -> bool {
    context.flags.native_plugins.iter().any(|name| {
        name.split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .any(|entry| entry == plugin_name)
    })
}

async fn execute_output(params: ExecuteOutputParams<'_>) -> Result<(), anyhow::Error> {
    let ExecuteOutputParams {
        context,
        config,
        output_base_path,
        filename,
        output_config,
        schema_input,
        documents,
        result,
        debug_timing_enabled,
        document_indices,
        root_document_index,
    } = params;

    if output_config.plugins.is_empty() {
        anyhow::bail!("No plugins configured for output `{filename}`");
    }

    let mut merged = plugin_helpers::types::ComplexPluginOutput::default();
    for plugin_spec in &output_config.plugins {
        let Some(plugin_name) = plugin_spec.name() else {
            anyhow::bail!("Invalid empty plugin config for output `{filename}`");
        };
        let closure_indices = document_indices.clone().unwrap_or_default();
        let preferred_indices = if supports_single_root_near_operation(plugin_name) {
            if let Some(root_idx) = root_document_index {
                vec![root_idx]
            } else {
                closure_indices.clone()
            }
        } else {
            closure_indices.clone()
        };
        let plugin_started = Instant::now();
        debug_event(
            debug_timing_enabled,
            format!("starting plugin {plugin_name} for {filename}"),
        );
        let js_config =
            merged_js_plugin_config(output_base_path, filename, output_config, plugin_spec);
        let mut js_config = js_config;

        if output_config.preset.as_deref() == Some("near-operation-file")
            && supports_single_root_near_operation(plugin_name)
            && let Some(root_idx) = root_document_index
            && closure_indices.len() > 1
            && !js_config.contains_key("externalFragments")
        {
            js_config.insert(
                "externalFragments".to_string(),
                build_external_fragments_config(documents, &closure_indices, root_idx),
            );
        }

        if native_plugin_requested(context, plugin_name) {
            let native_request_started = Instant::now();
            debug_event(
                debug_timing_enabled,
                format!("dispatching native plugin request {plugin_name} for {filename}"),
            );
            let out = run_native_plugin(
                plugin_name,
                plugin_spec,
                filename,
                output_config,
                schema_input,
                documents,
                &js_config,
            )?;
            debug_timing(
                debug_timing_enabled,
                format!("native plugin request {plugin_name} for {filename}"),
                native_request_started,
            );
            merge_complex_plugin_output(&mut merged, out);
            debug_timing(
                debug_timing_enabled,
                format!("plugin {plugin_name} for {filename}"),
                plugin_started,
            );
            continue;
        }

        let js_request_started = Instant::now();
        debug_event(
            debug_timing_enabled,
            format!("dispatching JS plugin request {plugin_name} for {filename}"),
        );

        let all_plugins: Vec<serde_json::Value> = output_config
            .plugins
            .iter()
            .map(plugin_spec_to_json)
            .collect();
        if debug_timing_enabled {
            if !preferred_indices.is_empty() {
                if preferred_indices.len() > 1 {
                    eprintln!(
                        "[codegen:debug] sending {} documents (indices mode) for {}",
                        preferred_indices.len(),
                        filename,
                    );
                }
            } else if documents.len() > 1 {
                eprintln!(
                    "[codegen:debug] sending {} documents for {}: {}",
                    documents.len(),
                    filename,
                    documents
                        .iter()
                        .map(|document| document.location.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
        }
        let host = context.js_plugin_host(debug_timing_enabled).await?;
        let plugin_context: serde_json::Map<String, serde_json::Value> = config
            .plugin_context
            .0
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();

        let plugin_documents = documents;
        let plugin_document_indices = preferred_indices.clone();

        let out = match host
            .run_plugin(crate::js_plugin_bridge::RunPluginParams {
                plugin_name,
                filename,
                all_plugins: all_plugins.clone(),
                plugin_config: js_config.clone(),
                output_config: js_config.clone(),
                plugin_context: plugin_context.clone(),
                schema: schema_input,
                documents: plugin_documents,
                document_indices: plugin_document_indices,
            })
            .await
        {
            Ok(out) => out,
            Err(error)
                if supports_single_root_near_operation(plugin_name)
                    && preferred_indices.len() == 1
                    && closure_indices.len() > 1
                    && (plugin_name == "typescript-operations"
                        || format!("{error:#}").contains("Node does not exist")) =>
            {
                host.run_plugin(crate::js_plugin_bridge::RunPluginParams {
                    plugin_name,
                    filename,
                    all_plugins,
                    plugin_config: js_config.clone(),
                    output_config: js_config,
                    plugin_context,
                    schema: schema_input,
                    documents,
                    document_indices: closure_indices,
                })
                .await
                .map_err(|retry_error| {
                    eprintln!(
                        "[codegen:debug] JS plugin failure for {plugin_name} on {filename}: {retry_error:#}"
                    );
                    retry_error
                })?
            }
            Err(error) => {
                eprintln!(
                    "[codegen:debug] JS plugin failure for {plugin_name} on {filename}: {error:#}"
                );
                return Err(error);
            }
        };

        debug_timing(
            debug_timing_enabled,
            format!("js host request {plugin_name} for {filename}"),
            js_request_started,
        );
        merge_complex_plugin_output(&mut merged, out);
        /*match plugin_name {
            "add" => match transitional_plugins::add::plugin(plugin_spec.config()) {
                Ok(out) => merge_complex_plugin_output(&mut merged, out),
                Err(e) => return Err(e),
            },
            "typescript" => {
                let ts_config = plugin_typescript::TypeScriptPluginConfig::from_output_config_map(
                    &output_config.config,
                );
                match plugin_typescript::plugin(schema_input, documents, &ts_config) {
                    Ok(out) => merge_complex_plugin_output(&mut merged, out),
                    Err(e) => return Err(e),
                }
            }
            "typescript-operations" => {
                let mut ops_config_map = output_config.config.clone();
                for (key, value) in &output_config.preset_config {
                    ops_config_map
                        .entry(key.clone())
                        .or_insert_with(|| value.clone());
                }
                let mut ops_config: plugin_typescript_operations::TypeScriptDocumentsPluginConfig =
                    serde_json::from_value(serde_json::Value::Object(ops_config_map))
                        .unwrap_or_default();
                if ops_config.import_operation_types_from.is_none() {
                    ops_config.import_operation_types_from = output_config
                        .preset_config
                        .get("baseTypesPath")
                        .and_then(|value| value.as_str())
                        .map(str::to_owned);
                }
                let mut ops_documents = documents.to_vec();
                if ops_config.flatten_generated_types {
                    ops_documents = relay_optimize::optimize_operations(
                        schema_input,
                        &ops_documents,
                        ops_config.flatten_generated_types_include_fragments,
                    )?;
                }
                match plugin_typescript_operations::plugin(schema_input, &ops_documents, &ops_config) {
                    Ok(out) => merge_complex_plugin_output(&mut merged, out),
                    Err(e) => return Err(e),
                }
            }
            "typed-document-node" => {
                let tdn_config: plugin_typed_document_node::TypeScriptTypedDocumentNodesConfig =
                    serde_json::from_value(serde_json::Value::Object(output_config.config.clone()))
                        .unwrap_or_default();
                match plugin_typed_document_node::plugin(schema_input, documents, &tdn_config) {
                    Ok(out) => merge_complex_plugin_output(&mut merged, out),
                    Err(e) => return Err(e),
                }
            }
            "typescript-react-apollo" => {
                let mut react_apollo_config_map = output_config.config.clone();
                for (key, value) in &output_config.preset_config {
                    react_apollo_config_map
                        .entry(key.clone())
                        .or_insert_with(|| value.clone());
                }
                let react_apollo_config: plugin_typescript_react_apollo::TypeScriptReactApolloConfig =
                    serde_json::from_value(serde_json::Value::Object(react_apollo_config_map))
                        .unwrap_or_default();
                match plugin_typescript_react_apollo::plugin(
                    schema_input,
                    documents,
                    &react_apollo_config,
                ) {
                    Ok(out) => merge_complex_plugin_output(&mut merged, out),
                    Err(e) => return Err(e),
                }
            }
            "fragment-matcher" => {
                let fragment_matcher_config: plugin_fragment_matcher::FragmentMatcherConfig =
                    serde_json::from_value(serde_json::Value::Object(output_config.config.clone()))
                        .unwrap_or_default();
                match plugin_fragment_matcher::plugin(
                    schema_input,
                    documents,
                    &fragment_matcher_config,
                    filename,
                ) {
                    Ok(out) => merge_complex_plugin_output(&mut merged, out),
                    Err(e) => return Err(e),
                }
            }
            "typescript-graphql-files-modules" => {
                let out = transitional_plugins::graphql_files_modules::plugin(documents);
                merge_complex_plugin_output(&mut merged, out);
            }
            "typescript-resolvers" => {
                let mut resolver_config = output_config.config.clone();
                if let Some(plugin_config) = plugin_spec.config()
                    && let Some(map) = plugin_config.as_object()
                {
                    for (key, value) in map {
                        resolver_config.insert(key.clone(), value.clone());
                    }
                }
                let out = transitional_plugins::typescript_resolvers::plugin(
                    &schema_input.introspection,
                    &resolver_config,
                );
                merge_complex_plugin_output(&mut merged, out);
            }
            other => unreachable!("bridge-first execution handles {other}"),
        }*/
        debug_timing(
            debug_timing_enabled,
            format!("plugin {plugin_name} for {filename}"),
            plugin_started,
        );
    }

    ensure_near_operation_types_import(output_base_path, filename, output_config, &mut merged);

    let merge_started = Instant::now();
    debug_event(
        debug_timing_enabled,
        format!("starting merge/finalize output {filename}"),
    );
    let content = merge_outputs(&merged);
    debug_timing(
        debug_timing_enabled,
        format!("merge/finalize output {filename}"),
        merge_started,
    );
    result.push(FileOutput {
        filename: filename.to_string(),
        content: Some(content),
        hooks: output_config.hooks.clone(),
    });
    Ok(())
}

fn run_native_plugin(
    plugin_name: &str,
    plugin_spec: &PluginSpec,
    filename: &str,
    output_config: &plugin_helpers::types::ConfiguredOutput,
    schema_input: &plugin_helpers::schema_input::SchemaGenerationInput,
    documents: &[DocumentFile],
    merged_config: &serde_json::Map<String, serde_json::Value>,
) -> Result<ComplexPluginOutput, anyhow::Error> {
    match plugin_name {
        "add" => transitional_plugins::add::plugin(plugin_spec.config()),
        "typescript" => {
            let ts_config =
                plugin_typescript::TypeScriptPluginConfig::from_output_config_map(merged_config);
            plugin_typescript::plugin(schema_input, documents, &ts_config)
        }
        "typescript-operations" => {
            let ops_config: plugin_typescript_operations::TypeScriptDocumentsPluginConfig =
                serde_json::from_value(serde_json::Value::Object(merged_config.clone()))
                    .unwrap_or_default();
            let mut ops_documents = documents.to_vec();
            if ops_config.flatten_generated_types {
                ops_documents = relay_optimize::optimize_operations(
                    schema_input,
                    &ops_documents,
                    ops_config.flatten_generated_types_include_fragments,
                )?;
            }
            plugin_typescript_operations::plugin(schema_input, &ops_documents, &ops_config)
        }
        "typed-document-node" => {
            let tdn_config: TypeScriptTypedDocumentNodesConfig =
                serde_json::from_value(serde_json::Value::Object(merged_config.clone()))
                    .unwrap_or_default();
            plugin_typed_document_node::plugin(schema_input, documents, &tdn_config)
        }
        "typescript-react-apollo" => {
            let react_apollo_config: TypeScriptReactApolloConfig =
                serde_json::from_value(serde_json::Value::Object(merged_config.clone()))
                    .unwrap_or_default();
            plugin_typescript_react_apollo::plugin(schema_input, documents, &react_apollo_config)
        }
        "fragment-matcher" => {
            let fragment_matcher_config: FragmentMatcherConfig =
                serde_json::from_value(serde_json::Value::Object(merged_config.clone()))
                    .unwrap_or_default();
            plugin_fragment_matcher::plugin(
                schema_input,
                documents,
                &fragment_matcher_config,
                filename,
            )
        }
        "typescript-graphql-files-modules" => Ok(
            transitional_plugins::graphql_files_modules::plugin(documents),
        ),
        "typescript-resolvers" => {
            let mut resolver_config = output_config.config.clone();
            if let Some(plugin_config) = plugin_spec.config()
                && let Some(map) = plugin_config.as_object()
            {
                for (key, value) in map {
                    resolver_config.insert(key.clone(), value.clone());
                }
            }
            let mut out = transitional_plugins::typescript_resolvers::plugin(
                &schema_input.introspection,
                &resolver_config,
            );
            out.content = transitional_plugins::typescript_resolvers::finalize_merged_content(
                out.content,
                &schema_input.introspection,
                &resolver_config,
            );
            Ok(out)
        }
        "flow" => Ok(ComplexPluginOutput {
            content: transitional_plugins::flow::combined_output(&schema_input.introspection),
            prepend: vec![],
            append: vec![],
        }),
        "flow-resolvers" => Ok(ComplexPluginOutput {
            content: transitional_plugins::flow::resolvers_output(&schema_input.introspection),
            prepend: vec![],
            append: vec![],
        }),
        other => anyhow::bail!(
            "Native plugin `{other}` is not available yet. Remove it from --native-plugins to use JS fallback."
        ),
    }
}

struct ExecuteOutputParams<'a> {
    context: &'a mut CodegenContext,
    config: &'a Config,
    output_base_path: &'a str,
    filename: &'a str,
    output_config: &'a plugin_helpers::types::ConfiguredOutput,
    schema_input: &'a plugin_helpers::schema_input::SchemaGenerationInput,
    documents: &'a [DocumentFile],
    result: &'a mut Vec<FileOutput>,
    debug_timing_enabled: bool,
    document_indices: Option<Vec<usize>>,
    root_document_index: Option<usize>,
}

fn merged_js_plugin_config(
    output_base_path: &str,
    filename: &str,
    output_config: &plugin_helpers::types::ConfiguredOutput,
    plugin_spec: &plugin_helpers::types::PluginSpec,
) -> serde_json::Map<String, serde_json::Value> {
    let mut merged = output_config.config.clone();
    for (key, value) in &output_config.preset_config {
        merged.entry(key.clone()).or_insert_with(|| value.clone());
    }
    if let Some(plugin_config) = plugin_spec.config()
        && let Some(map) = plugin_config.as_object()
    {
        for (key, value) in map {
            merged.insert(key.clone(), value.clone());
        }
    }

    let plugin_name = plugin_spec.name().unwrap_or_default();
    if plugin_name == "typescript-operations" {
        if output_config.preset.as_deref() == Some("near-operation-file")
            && !merged.contains_key("exportFragmentSpreadSubTypes")
        {
            merged.insert(
                "exportFragmentSpreadSubTypes".to_string(),
                serde_json::Value::Bool(true),
            );
        }

        let near_operation_base_types =
            near_operation_base_types_import_path(output_base_path, filename, output_config);

        if !merged.contains_key("importOperationTypesFrom")
            && let Some(import_path) = &near_operation_base_types
        {
            merged.insert(
                "importOperationTypesFrom".to_string(),
                serde_json::Value::String(import_path.clone()),
            );
        }

        if near_operation_base_types.is_some() && !merged.contains_key("namespacedImportName") {
            let namespace = output_config
                .preset_config
                .get("importTypesNamespace")
                .and_then(|value| value.as_str())
                .unwrap_or("Types");
            merged.insert(
                "namespacedImportName".to_string(),
                serde_json::Value::String(namespace.to_string()),
            );
        }
    }

    merged
}

fn ensure_near_operation_types_import(
    output_base_path: &str,
    filename: &str,
    output_config: &plugin_helpers::types::ConfiguredOutput,
    merged: &mut plugin_helpers::types::ComplexPluginOutput,
) {
    let Some(import_source) =
        near_operation_base_types_import_path(output_base_path, filename, output_config)
    else {
        return;
    };

    let namespace = output_config
        .preset_config
        .get("importTypesNamespace")
        .and_then(|value| value.as_str())
        .unwrap_or("Types");

    let namespace_marker = format!("{namespace}.");
    if !merged.content.contains(&namespace_marker) {
        return;
    }

    let use_type_imports = output_config
        .config
        .get("useTypeImports")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let import_type_line = format!("import type * as {namespace} from '{import_source}';");
    let import_line = format!("import * as {namespace} from '{import_source}';");

    let has_import = merged.prepend.iter().any(|line| {
        line.contains(&import_type_line)
            || line.contains(&import_line)
            || (line.contains(&format!("* as {namespace} from")) && line.contains(&import_source))
    }) || merged.content.contains(&import_type_line)
        || merged.content.contains(&import_line);

    if !has_import {
        let import_insert_index = merged
            .prepend
            .iter()
            .position(|line| line.starts_with("import"))
            .unwrap_or(merged.prepend.len());

        if use_type_imports {
            merged
                .prepend
                .insert(import_insert_index, format!("{import_type_line}\n"));
        } else {
            merged
                .prepend
                .insert(import_insert_index, format!("{import_line}\n"));
        }
    }
}

fn near_operation_base_types_import_path(
    output_base_path: &str,
    filename: &str,
    output_config: &plugin_helpers::types::ConfiguredOutput,
) -> Option<String> {
    if output_config.preset.as_deref() != Some("near-operation-file") {
        return None;
    }

    let base_types_path = output_config
        .preset_config
        .get("baseTypesPath")
        .and_then(|value| value.as_str())?;

    if let Some(path) = base_types_path.strip_prefix('~') {
        return Some(path.to_string());
    }

    let base_types_target = Path::new(output_base_path).join(base_types_path);
    let output_dir = Path::new(filename)
        .parent()
        .unwrap_or_else(|| Path::new(""));
    let relative =
        pathdiff::diff_paths(&base_types_target, output_dir).unwrap_or(base_types_target);
    let mut import_path = relative.to_string_lossy().replace('\\', "/");

    if !import_path.starts_with('.') {
        import_path = format!("./{import_path}");
    }

    Some(strip_import_code_extension(&import_path).to_string())
}

fn strip_import_code_extension(path: &str) -> &str {
    if let Some(stripped) = path.strip_suffix(".d.ts") {
        return stripped;
    }
    if let Some(stripped) = path.strip_suffix(".ts") {
        return stripped;
    }
    if let Some(stripped) = path.strip_suffix(".tsx") {
        return stripped;
    }
    if let Some(stripped) = path.strip_suffix(".js") {
        return stripped;
    }
    if let Some(stripped) = path.strip_suffix(".jsx") {
        return stripped;
    }
    path
}

fn plugin_spec_to_json(plugin_spec: &plugin_helpers::types::PluginSpec) -> serde_json::Value {
    match plugin_spec {
        plugin_helpers::types::PluginSpec::Name(name) => serde_json::Value::String(name.clone()),
        plugin_helpers::types::PluginSpec::Config(map) => serde_json::to_value(map)
            .unwrap_or_else(|_| serde_json::Value::Object(Default::default())),
    }
}

fn normalize(
    config: &Config,
) -> anyhow::Result<Vec<(String, plugin_helpers::types::ConfiguredOutput)>> {
    let generate_keys: Vec<_> = config.generates.keys().collect();

    if generate_keys.is_empty() {
        anyhow::bail!(
            r#"Invalid Codegen Configuration! \n
        Please make sure that your codegen config file contains the "generates" field, with a specification for the plugins you need.

        It should looks like that:

        schema:
          - my-schema.graphql
        generates:
          my-file.ts:
            - plugin1
            - plugin2
            - plugin3"#
        );
    }

    let mut out = Vec::new();
    for filename in generate_keys {
        let OutputConfig::Configured(configured) = &config.generates[filename];

        if configured.preset.is_none() && configured.plugins.is_empty() {
            anyhow::bail!(
                r#"Invalid Codegen Configuration! \n
          Please make sure that your codegen config file has defined plugins list for output "{filename}".

          It should looks like that:

          schema:
            - my-schema.graphql
          generates:
            my-file.ts:
              - plugin1
              - plugin2
              - plugin3
          "#
            );
        }

        out.push((filename.clone(), configured.clone()));
    }

    if config.schema.is_empty() && out.iter().any(|(_, output)| output.schema.is_empty()) {
        anyhow::bail!(
            r#"Invalid Codegen Configuration! \n
        Please make sure that your codegen config file contains either the "schema" field
        or every generated file has its own "schema" field.

        It should looks like that:
        schema:
          - my-schema.graphql

        or:
        generates:
          path/to/output:
            schema: my-schema.graphql
      "#
        );
    }

    Ok(out)
}
