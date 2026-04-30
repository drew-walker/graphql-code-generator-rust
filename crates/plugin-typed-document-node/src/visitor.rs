//! Port of `packages/plugins/typescript/typed-document-node/src/visitor.ts` (thin wrapper).

use anyhow::Result;
use graphql_parser::query::{Definition, OperationDefinition};
use plugin_helpers::types::ComplexPluginOutput;
use visitor_plugin_common::client_side_base_visitor::{
    build_document_with_dependent_fragments, collect_fragments, merge_documents, order_fragments,
    print_document,
};

use crate::config::TypeScriptTypedDocumentNodesConfig;

pub struct TypeScriptDocumentNodesVisitor<'a> {
    config: &'a TypeScriptTypedDocumentNodesConfig,
    documents: &'a [plugin_helpers::types::DocumentFile],
}

impl<'a> TypeScriptDocumentNodesVisitor<'a> {
    pub fn new(
        config: &'a TypeScriptTypedDocumentNodesConfig,
        documents: &'a [plugin_helpers::types::DocumentFile],
    ) -> Self {
        Self { config, documents }
    }

    pub fn generate(&self) -> Result<ComplexPluginOutput> {
        let _ = self.config;

        let mut out = String::new();

        // Upstream uses `documentNodeImport: '@graphql-typed-document-node/core#TypedDocumentNode'`
        // and aliases it to `DocumentNode`.
        out.push_str("import { TypedDocumentNode as DocumentNode } from '@graphql-typed-document-node/core';\n\n");

        // Emit document constants. This plugin relies on `typescript` + `typescript-operations`
        // running in the same output file to provide the referenced types.
        let merged = merge_documents(self.documents);

        // Upstream visitor emits fragments first, in dependency order.
        let fragments = collect_fragments(&merged);
        for name in order_fragments(&fragments) {
            if let Some(f) = fragments.get(&name) {
                out.push_str(&self.emit_fragment_doc(f, &fragments));
                out.push('\n');
            }
        }
        // Then operations.
        for def in &merged.definitions {
            if let Definition::Operation(op) = def {
                if let Some(s) = self.emit_operation_doc(op, &fragments) {
                    out.push_str(&s);
                    out.push('\n');
                }
            }
        }

        Ok(ComplexPluginOutput {
            prepend: vec![],
            content: out.trim_end_matches('\n').to_string(),
            append: vec![],
        })
    }

    fn emit_fragment_doc(
        &self,
        f: &graphql_parser::query::FragmentDefinition<'static, String>,
        fragments: &std::collections::HashMap<
            String,
            graphql_parser::query::FragmentDefinition<'static, String>,
        >,
    ) -> String {
        let const_name = format!("{}FragmentDoc", f.name);
        let type_name = format!("{}Fragment", f.name);
        let doc =
            build_document_with_dependent_fragments(Definition::Fragment(f.clone()), fragments);
        let js = print_document(&doc);
        format!(
            "export const {const_name} = {js} as unknown as DocumentNode<{type_name}, unknown>;"
        )
    }

    fn emit_operation_doc(
        &self,
        op: &OperationDefinition<'static, String>,
        fragments: &std::collections::HashMap<
            String,
            graphql_parser::query::FragmentDefinition<'static, String>,
        >,
    ) -> Option<String> {
        let (op_kind, name) = match op {
            OperationDefinition::Query(q) => ("Query", q.name.as_deref()?),
            OperationDefinition::Mutation(m) => ("Mutation", m.name.as_deref()?),
            OperationDefinition::Subscription(s) => ("Subscription", s.name.as_deref()?),
            OperationDefinition::SelectionSet(_) => return None,
        };

        let const_name = format!("{}Document", to_pascal_case(name));
        let result_type = format!("{}{}", to_pascal_case(name), op_kind);
        let vars_type = format!("{result_type}Variables");
        let doc =
            build_document_with_dependent_fragments(Definition::Operation(op.clone()), fragments);
        let js = print_document(&doc);
        Some(format!(
            "export const {const_name} = {js} as unknown as DocumentNode<{result_type}, {vars_type}>;"
        ))
    }

    // (printing logic lives in `visitor-plugin-common`)
}

fn to_pascal_case(name: &str) -> String {
    let mut out = String::new();
    let mut upper_next = true;
    for ch in name.chars() {
        if ch == '_' || ch == '-' || ch == ' ' {
            upper_next = true;
            continue;
        }
        if upper_next {
            out.extend(ch.to_uppercase());
            upper_next = false;
        } else {
            out.push(ch);
        }
    }
    out
}
