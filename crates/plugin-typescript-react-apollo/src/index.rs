use anyhow::Result;
use graphql_parser::query::{Definition, FragmentDefinition, OperationDefinition};
use plugin_helpers::schema_input::SchemaGenerationInput;
use plugin_helpers::types::{ComplexPluginOutput, DocumentFile};
use visitor_plugin_common::client_side_base_visitor::{
    build_document_with_dependent_fragments, collect_fragments, merge_documents,
    print_document_json,
};

use crate::config::TypeScriptReactApolloConfig;

pub fn plugin(
    _schema: &SchemaGenerationInput,
    documents: &[DocumentFile],
    _config: &TypeScriptReactApolloConfig,
) -> Result<ComplexPluginOutput> {
    let merged = merge_documents(documents);
    let fragments = collect_fragments(&merged);

    let mut content = String::new();
    content.push_str("import type { TypedDocumentNode as DocumentNode } from '@graphql-typed-document-node/core';\n");
    content.push_str("import * as Apollo from '@apollo/client/react';\n");
    content.push('\n');

    for def in &merged.definitions {
        match def {
            Definition::Operation(op) => {
                if let Some(block) = emit_operation_block(op, &fragments) {
                    content.push_str(&block);
                    content.push_str("\n\n");
                }
            }
            Definition::Fragment(fragment) => {
                content.push_str(&emit_fragment_block(fragment, &fragments));
                content.push_str("\n\n");
            }
        }
    }

    Ok(ComplexPluginOutput {
        content: content.trim_end().to_string(),
        prepend: vec![],
        append: vec![],
    })
}

fn emit_operation_block(
    op: &OperationDefinition<'static, String>,
    fragments: &std::collections::HashMap<String, FragmentDefinition<'static, String>>,
) -> Option<String> {
    let (operation_kind, operation_name) = match op {
        OperationDefinition::Query(query) => ("Query", query.name.as_deref()?),
        OperationDefinition::Mutation(mutation) => ("Mutation", mutation.name.as_deref()?),
        OperationDefinition::Subscription(subscription) => {
            ("Subscription", subscription.name.as_deref()?)
        }
        OperationDefinition::SelectionSet(_) => return None,
    };

    let pascal_name = to_pascal_case(operation_name);
    let document_name = format!("{pascal_name}Document");
    let result_type = format!("{pascal_name}{operation_kind}");
    let variables_type = format!("{result_type}Variables");
    let doc = build_document_with_dependent_fragments(Definition::Operation(op.clone()), fragments);
    let document_json = print_document_json(&doc);

    let mut out = String::new();
    out.push_str(&format!(
        "export const {document_name} = {document_json} as unknown as DocumentNode<{result_type}, {variables_type}>;\n"
    ));
    out.push_str("const defaultOptions = {} as const;\n");

    match op {
        OperationDefinition::Query(_) => {
            out.push_str(&format!(
                "export function use{pascal_name}Query(baseOptions: Apollo.QueryHookOptions<{result_type}, {variables_type}>) {{\n  const options = {{...defaultOptions, ...baseOptions}}\n  return Apollo.useQuery<{result_type}, {variables_type}>({document_name}, options);\n}}\n"
            ));
            out.push_str(&format!(
                "export function use{pascal_name}LazyQuery(baseOptions?: Apollo.LazyQueryHookOptions<{result_type}, {variables_type}>) {{\n  const options = {{...defaultOptions, ...baseOptions}}\n  return Apollo.useLazyQuery<{result_type}, {variables_type}>({document_name}, options);\n}}\n"
            ));
            out.push_str(&format!(
                "export function use{pascal_name}SuspenseQuery(baseOptions?: Apollo.SkipToken | Apollo.SuspenseQueryHookOptions<{result_type}, {variables_type}>) {{\n  const options = baseOptions === Apollo.skipToken ? baseOptions : {{...defaultOptions, ...baseOptions}}\n  return Apollo.useSuspenseQuery<{result_type}, {variables_type}>({document_name}, options);\n}}\n"
            ));
            out.push_str(&format!("export type {pascal_name}QueryHookResult = ReturnType<typeof use{pascal_name}Query>;\n"));
            out.push_str(&format!("export type {pascal_name}LazyQueryHookResult = ReturnType<typeof use{pascal_name}LazyQuery>;\n"));
            out.push_str(&format!("export type {pascal_name}SuspenseQueryHookResult = ReturnType<typeof use{pascal_name}SuspenseQuery>;\n"));
            out.push_str(&format!("export type {pascal_name}QueryResult = Apollo.QueryResult<{result_type}, {variables_type}>;\n"));
        }
        OperationDefinition::Mutation(_) => {
            out.push_str(&format!("export type {pascal_name}MutationFn = Apollo.MutationFunction<{result_type}, {variables_type}>;\n"));
            out.push_str(&format!(
                "export function use{pascal_name}Mutation(baseOptions?: Apollo.MutationHookOptions<{result_type}, {variables_type}>) {{\n  const options = {{...defaultOptions, ...baseOptions}}\n  return Apollo.useMutation<{result_type}, {variables_type}>({document_name}, options);\n}}\n"
            ));
            out.push_str(&format!("export type {pascal_name}MutationHookResult = ReturnType<typeof use{pascal_name}Mutation>;\n"));
            out.push_str(&format!(
                "export type {pascal_name}MutationResult = Apollo.MutationResult<{result_type}>;\n"
            ));
            out.push_str(&format!("export type {pascal_name}MutationOptions = Apollo.MutationFunctionOptions<{result_type}, {variables_type}>;\n"));
        }
        OperationDefinition::Subscription(_) => {
            out.push_str(&format!(
                "export function use{pascal_name}Subscription(baseOptions: Apollo.SubscriptionHookOptions<{result_type}, {variables_type}>) {{\n  const options = {{...defaultOptions, ...baseOptions}}\n  return Apollo.useSubscription<{result_type}, {variables_type}>({document_name}, options);\n}}\n"
            ));
            out.push_str(&format!("export type {pascal_name}SubscriptionHookResult = ReturnType<typeof use{pascal_name}Subscription>;\n"));
            out.push_str(&format!("export type {pascal_name}SubscriptionResult = Apollo.SubscriptionResult<{result_type}>;\n"));
        }
        OperationDefinition::SelectionSet(_) => {}
    }

    Some(out)
}

fn emit_fragment_block(
    fragment: &FragmentDefinition<'static, String>,
    fragments: &std::collections::HashMap<String, FragmentDefinition<'static, String>>,
) -> String {
    let fragment_name = fragment.name.as_str();
    let type_name = format!("{fragment_name}Fragment");
    let document_name = format!("{fragment_name}FragmentDoc");
    let doc =
        build_document_with_dependent_fragments(Definition::Fragment(fragment.clone()), fragments);
    let document_json = print_document_json(&doc);

    format!(
        "export const {document_name} = {document_json} as unknown as DocumentNode<{type_name}, unknown>;"
    )
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
