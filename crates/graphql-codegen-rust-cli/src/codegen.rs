use crate::config::CodegenContext;
use plugin_helpers::types::{Config, FileOutput, OutputConfig};

#[derive(Debug)]
pub struct ExecuteCodegenOutput {
    pub result: Vec<FileOutput>,
    pub error: Option<anyhow::Error>,
}

pub async fn execute_codegen(context: &mut CodegenContext) -> ExecuteCodegenOutput {
    let config = context.get_config();
    let mut result: Vec<FileOutput> = Vec::new();

    let generates = match normalize(&config) {
        Ok(g) => g,
        Err(error) => {
            return ExecuteCodegenOutput {
                result,
                error: Some(error),
            };
        }
    };

    for (filename, output_config) in generates {
        // TS executeCodegen per-output pipeline: load schema → load documents → generate.
        let mut schema_pointers: Vec<String> = Vec::new();
        schema_pointers.extend(config.schema.clone());
        schema_pointers.extend(output_config.schema.clone());

        let schema_input = match context.load_schema(&schema_pointers).await {
            Ok(s) => s,
            Err(e) => {
                return ExecuteCodegenOutput {
                    result,
                    error: Some(e),
                };
            }
        };

        // TODO: load documents (root + output), presets, plugin map, documentTransforms — TS parity.
        let _ = (&config.documents, &output_config.documents);

        let mut content = String::new();
        for plugin_name in &output_config.plugins {
            match plugin_name.as_str() {
                "typescript" => {
                    let ts_config =
                        plugin_typescript::TypeScriptPluginConfig::from_output_config_map(
                            &output_config.config,
                        );
                    match plugin_typescript::plugin(&schema_input, &[], &ts_config) {
                        Ok(out) => content = plugin_typescript::merge_plugin_output(&out),
                        Err(e) => {
                            return ExecuteCodegenOutput {
                                result,
                                error: Some(e),
                            };
                        }
                    }
                }
                other => {
                    return ExecuteCodegenOutput {
                        result,
                        error: Some(anyhow::anyhow!(
                            "Unsupported plugin `{other}` for output `{filename}` (only `typescript` is wired)"
                        )),
                    };
                }
            }
        }

        result.push(FileOutput {
            filename: filename.to_string(),
            content: Some(content),
            hooks: output_config.hooks,
        });
    }

    ExecuteCodegenOutput {
        result,
        error: None,
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
