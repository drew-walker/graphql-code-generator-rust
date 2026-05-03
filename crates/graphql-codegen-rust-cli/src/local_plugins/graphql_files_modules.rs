use std::path::Path;

use graphql_parser::query::{Definition, OperationDefinition};
use plugin_helpers::types::{ComplexPluginOutput, DocumentFile};

pub fn plugin(documents: &[DocumentFile]) -> ComplexPluginOutput {
    let mut module_order: Vec<String> = Vec::new();
    let mut exports_by_module: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    let mut documents = documents.iter().collect::<Vec<_>>();
    documents.sort_by(|a, b| a.location.cmp(&b.location));

    for document_file in documents {
        let Some(filename) = Path::new(&document_file.location)
            .file_name()
            .and_then(|s| s.to_str())
        else {
            continue;
        };
        let module_name = format!("*/{filename}");
        if !exports_by_module.contains_key(&module_name) {
            module_order.push(module_name.clone());
        }

        let exports = exports_by_module.entry(module_name).or_default();
        for definition in &document_file.document.definitions {
            let name = match definition {
                Definition::Fragment(fragment) => Some(fragment.name.as_str()),
                Definition::Operation(operation) => match operation {
                    OperationDefinition::Query(query) => query.name.as_deref(),
                    OperationDefinition::Mutation(mutation) => mutation.name.as_deref(),
                    OperationDefinition::Subscription(subscription) => subscription.name.as_deref(),
                    OperationDefinition::SelectionSet(_) => None,
                },
            };
            if let Some(name) = name
                && !exports.iter().any(|existing| existing == name)
            {
                exports.push(name.to_string());
            }
        }
    }

    let mut blocks = Vec::new();
    for module_name in module_order {
        let exports = exports_by_module.remove(&module_name).unwrap_or_default();
        let mut block = format!(
            "declare module '{module_name}' {{\n  import {{ DocumentNode }} from 'graphql';\n  const defaultDocument: DocumentNode;"
        );
        for export_name in exports {
            block.push_str(&format!("\n  export const {export_name}: DocumentNode;"));
        }
        block.push_str("\n\n  export default defaultDocument;\n}");
        blocks.push(block);
    }

    ComplexPluginOutput {
        content: blocks.join("\n\n"),
        prepend: vec![],
        append: vec![],
    }
}
