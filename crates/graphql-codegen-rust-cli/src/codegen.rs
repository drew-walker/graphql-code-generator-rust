use std::path::Path;

use crate::config::CodegenContext;
use crate::load::load_documents_with_timing;
use crate::relay_optimize;
use crate::transitional_plugins;
use crate::utils::debugging::{debug_event, debug_timing};
use plugin_helpers::types::{Config, FileOutput, OutputConfig};
use plugin_helpers::utils::{merge_complex_plugin_output, merge_outputs};
use std::time::Instant;

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
        if output_config.preset.is_some() {
            if debug_timing_enabled {
                eprintln!("[codegen:debug] skipping preset output {filename}");
            }
            continue;
        }

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

        if has_plugin(&output_config, "flow") || has_plugin(&output_config, "flow-resolvers") {
            let flow_started = Instant::now();
            let content = transitional_plugins::flow::combined_output(&schema_input.introspection);
            debug_timing(
                debug_timing_enabled,
                format!("generate flow output for {filename}"),
                flow_started,
            );
            result.push(FileOutput {
                filename: filename.to_string(),
                content: Some(content),
                hooks: output_config.hooks,
            });
            debug_timing(
                debug_timing_enabled,
                format!("complete output {filename}"),
                output_started,
            );
            continue;
        }

        let mut merged = plugin_helpers::types::ComplexPluginOutput::default();
        for plugin_spec in &output_config.plugins {
            let Some(plugin_name) = plugin_spec.name() else {
                return ExecuteCodegenOutput {
                    result,
                    error: Some(anyhow::anyhow!(
                        "Invalid empty plugin config for output `{filename}`"
                    )),
                };
            };
            let plugin_started = Instant::now();
            debug_event(
                debug_timing_enabled,
                format!("starting plugin {plugin_name} for {filename}"),
            );
            match plugin_name {
                "add" => match transitional_plugins::add::plugin(plugin_spec.config()) {
                    Ok(out) => merge_complex_plugin_output(&mut merged, out),
                    Err(e) => {
                        return ExecuteCodegenOutput {
                            result,
                            error: Some(e),
                        };
                    }
                },
                "typescript" => {
                    let ts_config =
                        plugin_typescript::TypeScriptPluginConfig::from_output_config_map(
                            &output_config.config,
                        );
                    match plugin_typescript::plugin(&schema_input, &documents, &ts_config) {
                        Ok(out) => merge_complex_plugin_output(&mut merged, out),
                        Err(e) => {
                            return ExecuteCodegenOutput {
                                result,
                                error: Some(e),
                            };
                        }
                    }
                }
                "typescript-operations" => {
                    let ops_config: plugin_typescript_operations::TypeScriptDocumentsPluginConfig =
                        serde_json::from_value(serde_json::Value::Object(
                            output_config.config.clone(),
                        ))
                        .unwrap_or_default();
                    let mut ops_documents = documents.clone();
                    if ops_config.flatten_generated_types {
                        ops_documents = match relay_optimize::optimize_operations(
                            &schema_input,
                            &ops_documents,
                            ops_config.flatten_generated_types_include_fragments,
                        ) {
                            Ok(documents) => documents,
                            Err(e) => {
                                return ExecuteCodegenOutput {
                                    result,
                                    error: Some(e),
                                };
                            }
                        };
                    }
                    match plugin_typescript_operations::plugin(
                        &schema_input,
                        &ops_documents,
                        &ops_config,
                    ) {
                        Ok(out) => merge_complex_plugin_output(&mut merged, out),
                        Err(e) => {
                            return ExecuteCodegenOutput {
                                result,
                                error: Some(e),
                            };
                        }
                    }
                }
                "typed-document-node" => {
                    let tdn_config: plugin_typed_document_node::TypeScriptTypedDocumentNodesConfig =
                        serde_json::from_value(serde_json::Value::Object(
                            output_config.config.clone(),
                        ))
                        .unwrap_or_default();
                    match plugin_typed_document_node::plugin(&schema_input, &documents, &tdn_config)
                    {
                        Ok(out) => merge_complex_plugin_output(&mut merged, out),
                        Err(e) => {
                            return ExecuteCodegenOutput {
                                result,
                                error: Some(e),
                            };
                        }
                    }
                }
                "typescript-graphql-files-modules" => {
                    let out = transitional_plugins::graphql_files_modules::plugin(&documents);
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
                other => {
                    return ExecuteCodegenOutput {
                        result,
                        error: Some(anyhow::anyhow!(
                            "Unsupported plugin `{other}` for output `{filename}`"
                        )),
                    };
                }
            }
            debug_timing(
                debug_timing_enabled,
                format!("plugin {plugin_name} for {filename}"),
                plugin_started,
            );
        }

        let merge_started = Instant::now();
        debug_event(
            debug_timing_enabled,
            format!("starting merge/finalize output {filename}"),
        );
        let mut content = merge_outputs(&merged);
        if has_plugin(&output_config, "typescript-resolvers") {
            content = transitional_plugins::typescript_resolvers::finalize_merged_content(
                content,
                &schema_input.introspection,
                &output_config.config,
            );
        }
        debug_timing(
            debug_timing_enabled,
            format!("merge/finalize output {filename}"),
            merge_started,
        );
        result.push(FileOutput {
            filename: filename.to_string(),
            content: Some(content),
            hooks: output_config.hooks,
        });
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

fn has_plugin(output_config: &plugin_helpers::types::ConfiguredOutput, name: &str) -> bool {
    output_config
        .plugins
        .iter()
        .any(|plugin| plugin.name() == Some(name))
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
