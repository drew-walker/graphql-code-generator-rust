//! Port of `packages/plugins/typescript/typed-document-node/src/config.ts` (subset).

use serde::Deserialize;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TypeScriptTypedDocumentNodesConfig {
    pub flatten_generated_types: bool,
    pub add_typename_to_selection_sets: bool,
    pub import_operation_types_from: Option<String>,
}
