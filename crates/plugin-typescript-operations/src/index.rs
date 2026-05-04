//! Port of `packages/plugins/typescript/operations/src/index.ts`.

use anyhow::Result;
use plugin_helpers::schema_input::SchemaGenerationInput;
use plugin_helpers::types::{ComplexPluginOutput, DocumentFile};

use crate::config::TypeScriptDocumentsPluginConfig;
use crate::visitor::TypeScriptDocumentsVisitor;

pub fn plugin(
    schema: &SchemaGenerationInput,
    documents: &[DocumentFile],
    config: &TypeScriptDocumentsPluginConfig,
) -> Result<ComplexPluginOutput> {
    let visitor = TypeScriptDocumentsVisitor::new(schema, config, documents);
    let mut out = visitor.generate()?;

    // Upstream adds `addOperationExport` consts by iterating all documents (standard + external).
    if config.add_operation_export {
        let mut export_consts: Vec<String> = Vec::new();
        for d in documents {
            for def in &d.document.definitions {
                match def {
                    graphql_parser::query::Definition::Operation(op) => {
                        let name = match op {
                            graphql_parser::query::OperationDefinition::Query(q) => {
                                q.name.as_deref()
                            }
                            graphql_parser::query::OperationDefinition::Mutation(m) => {
                                m.name.as_deref()
                            }
                            graphql_parser::query::OperationDefinition::Subscription(s) => {
                                s.name.as_deref()
                            }
                            graphql_parser::query::OperationDefinition::SelectionSet(_) => None,
                        };
                        if let Some(name) = name {
                            export_consts.push(format!(
                                "export declare const {name}: import(\"graphql\").DocumentNode;"
                            ));
                        }
                    }
                    graphql_parser::query::Definition::Fragment(f) => {
                        export_consts.push(format!(
                            "export declare const {}: import(\"graphql\").DocumentNode;",
                            f.name
                        ));
                    }
                }
            }
        }

        if !export_consts.is_empty() {
            if out.content.is_empty() {
                out.content = export_consts.join("\n");
            } else {
                out.content = [out.content, export_consts.join("\n")].join("\n");
            }
        }
    }

    // Upstream prepends imports + global declarations.
    out.prepend = [visitor.get_global_declarations(false), out.prepend].concat();

    Ok(out)
}
