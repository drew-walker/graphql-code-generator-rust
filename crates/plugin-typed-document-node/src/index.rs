//! Port of `packages/plugins/typescript/typed-document-node/src/index.ts` (minimal).

use anyhow::Result;
use plugin_helpers::schema_input::SchemaGenerationInput;
use plugin_helpers::types::{ComplexPluginOutput, DocumentFile};

use crate::config::TypeScriptTypedDocumentNodesConfig;
use crate::visitor::TypeScriptDocumentNodesVisitor;

pub fn plugin(
    _schema: &SchemaGenerationInput,
    documents: &[DocumentFile],
    config: &TypeScriptTypedDocumentNodesConfig,
) -> Result<ComplexPluginOutput> {
    let visitor = TypeScriptDocumentNodesVisitor::new(config, documents);
    visitor.generate()
}
