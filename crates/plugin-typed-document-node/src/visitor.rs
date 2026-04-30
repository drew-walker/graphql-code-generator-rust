//! Port of `packages/plugins/typescript/typed-document-node/src/visitor.ts` (thin wrapper).

use anyhow::Result;
use graphql_parser::Pos;
use graphql_parser::query::{Definition, OperationDefinition, Selection, SelectionSet};
use plugin_helpers::types::ComplexPluginOutput;
use visitor_plugin_common::client_side_base_visitor::{
    build_document_with_dependent_fragments, collect_fragments, merge_documents,
    order_fragments_with_roots, print_document_json,
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
        let mut out = String::new();

        if let Some(path) = &self.config.import_operation_types_from {
            out.push_str(&format!("import * as Types from '{path}';\n"));
        }

        // Upstream uses `documentNodeImport: '@graphql-typed-document-node/core#TypedDocumentNode'`
        // and aliases it to `DocumentNode`.
        out.push_str("import { TypedDocumentNode as DocumentNode } from '@graphql-typed-document-node/core';\n\n");

        // Emit document constants. This plugin relies on `typescript` + `typescript-operations`
        // running in the same output file to provide the referenced types.
        let merged = merge_documents(self.documents);

        // Upstream visitor emits fragments first, in dependency order.
        let fragments = collect_fragments(&merged);
        let fragment_roots: Vec<String> = merged
            .definitions
            .iter()
            .filter_map(|d| match d {
                Definition::Fragment(f) => Some(f.name.clone()),
                _ => None,
            })
            .collect();

        for name in order_fragments_with_roots(&fragment_roots, &fragments) {
            if let Some(f) = fragments.get(&name) {
                out.push_str(&self.emit_fragment_doc(f, &fragments));
                out.push('\n');
            }
        }
        // Then operations.
        for def in &merged.definitions {
            if let Definition::Operation(op) = def
                && let Some(s) = self.emit_operation_doc(op, &fragments)
            {
                out.push_str(&s);
                out.push('\n');
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
        let mut doc =
            build_document_with_dependent_fragments(Definition::Fragment(f.clone()), fragments);
        if self.config.add_typename_to_selection_sets {
            add_typename_to_document(&mut doc);
        }
        let js = print_document_json(&doc);
        let type_prefix = if self.config.import_operation_types_from.is_some() {
            "Types."
        } else {
            ""
        };
        format!(
            "export const {const_name} = {js} as unknown as DocumentNode<{type_prefix}{type_name}, unknown>;"
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
        let mut doc =
            build_document_with_dependent_fragments(Definition::Operation(op.clone()), fragments);
        if self.config.add_typename_to_selection_sets {
            add_typename_to_document(&mut doc);
        }
        let js = print_document_json(&doc);
        let type_prefix = if self.config.import_operation_types_from.is_some() {
            "Types."
        } else {
            ""
        };
        Some(format!(
            "export const {const_name} = {js} as unknown as DocumentNode<{type_prefix}{result_type}, {type_prefix}{vars_type}>;"
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

fn add_typename_to_document(doc: &mut graphql_parser::query::Document<'static, String>) {
    for def in &mut doc.definitions {
        match def {
            Definition::Fragment(f) => {
                add_typename_to_selection_set(&mut f.selection_set, false);
            }
            Definition::Operation(op) => match op {
                OperationDefinition::Query(q) => {
                    add_typename_to_selection_set(&mut q.selection_set, true);
                }
                OperationDefinition::Mutation(m) => {
                    add_typename_to_selection_set(&mut m.selection_set, true);
                }
                OperationDefinition::Subscription(s) => {
                    add_typename_to_selection_set(&mut s.selection_set, true);
                }
                OperationDefinition::SelectionSet(ss) => {
                    add_typename_to_selection_set(ss, true);
                }
            },
        }
    }
}

fn add_typename_to_selection_set(ss: &mut SelectionSet<'static, String>, is_operation_root: bool) {
    if !is_operation_root && !selection_set_has_typename(ss) {
        ss.items
            .push(Selection::Field(graphql_parser::query::Field {
                position: Pos { line: 0, column: 0 },
                alias: None,
                name: "__typename".to_string(),
                arguments: vec![],
                directives: vec![],
                selection_set: SelectionSet {
                    span: ss.span,
                    items: vec![],
                },
            }));
    }

    for sel in &mut ss.items {
        match sel {
            Selection::Field(f) => add_typename_to_selection_set(&mut f.selection_set, false),
            Selection::InlineFragment(inline) => {
                add_typename_to_selection_set(&mut inline.selection_set, false)
            }
            Selection::FragmentSpread(_) => {}
        }
    }
}

fn selection_set_has_typename(ss: &SelectionSet<'static, String>) -> bool {
    ss.items.iter().any(|sel| match sel {
        Selection::Field(f) => f.name == "__typename" || f.name.starts_with("__"),
        _ => false,
    })
}
