//! Port of `packages/plugins/typescript/typescript/src/index.ts`.

use anyhow::Result;
use plugin_helpers::schema_input::SchemaGenerationInput;
use plugin_helpers::types::{ComplexPluginOutput, DocumentFile};

use crate::config::TypeScriptPluginConfig;
use crate::introspection_visitor::TsIntrospectionVisitor;
use crate::visitor::TsVisitor;

/// Merges `prepend` + `content` the way `@graphql-codegen/core` does before writing a file.
pub fn merge_plugin_output(output: &ComplexPluginOutput) -> String {
    let prepend = output.prepend.join("\n");
    if prepend.is_empty() {
        return output.content.clone();
    }
    format!("{prepend}\n{}", output.content)
}

/// Mirrors `export const plugin: PluginFunction<...>` from `index.ts`.
///
/// `documents` matches the TS arity; pass `&[]` until document loading is ported.
pub fn plugin(
    schema: &SchemaGenerationInput,
    _documents: &[DocumentFile],
    config: &TypeScriptPluginConfig,
) -> Result<ComplexPluginOutput> {
    let visitor = TsVisitor::new(schema, config);

    let mut prepend: Vec<String> = Vec::new();
    prepend.extend(visitor.get_enums_imports());
    prepend.extend(visitor.get_directive_argument_and_input_field_mappings_imports());
    prepend.extend(visitor.get_scalars_imports());
    prepend.extend(visitor.get_wrapper_definitions());
    prepend.retain(|s| !s.is_empty());

    let scalars = visitor.scalars_definition();
    let introspection_visitor = TsIntrospectionVisitor::new(schema, config);
    let definitions = introspection_visitor.build_definitions_from_introspection()?;

    let content = [scalars, definitions]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n");

    Ok(ComplexPluginOutput {
        content,
        prepend,
        append: vec![],
    })
}
