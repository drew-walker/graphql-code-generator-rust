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
        // TODO: implement schema loading, document loading, and plugin execution
        // mirroring the per-output task pipeline in TS executeCodegen.
        let content = match std::fs::read_to_string(context.cwd.join(&filename)) {
            Ok(c) => c,
            Err(e) => {
                return ExecuteCodegenOutput {
                    result,
                    error: Some(anyhow::anyhow!(
                        "Failed to read output for '{}': {}",
                        context.cwd.join(&filename).display(),
                        e
                    )),
                };
            }
        };

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
