//! Mirrors the `optimizeOperations` call used by
//! `packages/plugins/typescript/operations/src/index.ts` when
//! `config.flattenGeneratedTypes` is true.
//!
//! Upstream delegates to `@graphql-tools/relay-operation-optimizer`. This is a focused Rust
//! implementation of the behavior needed by the current `dev-test` fixtures: inline fragment
//! spreads into operations, recursively merge duplicate field selections, and drop fragment
//! definitions unless `flattenGeneratedTypesIncludeFragments` is enabled.

use std::collections::{HashMap, HashSet};

use anyhow::Result;
use graphql_parser::query::{
    Definition, Document, Field, FragmentDefinition, OperationDefinition, Selection, SelectionSet,
    TypeCondition,
};
use plugin_helpers::schema_input::SchemaGenerationInput;
use plugin_helpers::types::DocumentFile;
use serde_json::Value;

pub fn optimize_operations(
    schema: &SchemaGenerationInput,
    documents: &[DocumentFile],
    include_fragments: bool,
) -> Result<Vec<DocumentFile>> {
    let schema = SchemaInfo::new(schema);
    let fragments = collect_fragments(documents);
    documents
        .iter()
        .map(|document_file| {
            let document = optimize_document(
                &schema,
                &document_file.document,
                &fragments,
                include_fragments,
            )?;
            Ok(DocumentFile {
                location: document_file.location.clone(),
                document,
                r#type: document_file.r#type.clone(),
            })
        })
        .collect()
}

fn collect_fragments(
    documents: &[DocumentFile],
) -> HashMap<String, FragmentDefinition<'static, String>> {
    let mut fragments = HashMap::new();
    for document_file in documents {
        for definition in &document_file.document.definitions {
            if let Definition::Fragment(fragment) = definition {
                fragments.insert(fragment.name.clone(), fragment.clone());
            }
        }
    }
    fragments
}

fn optimize_document(
    schema: &SchemaInfo<'_>,
    document: &Document<'static, String>,
    fragments: &HashMap<String, FragmentDefinition<'static, String>>,
    include_fragments: bool,
) -> Result<Document<'static, String>> {
    let mut definitions = Vec::new();
    for definition in &document.definitions {
        match definition {
            Definition::Operation(operation) => {
                definitions.push(Definition::Operation(optimize_operation(
                    schema, operation, fragments,
                )?));
            }
            Definition::Fragment(fragment) if include_fragments => {
                let mut fragment = fragment.clone();
                fragment.selection_set = optimize_selection_set(
                    schema,
                    Some(type_condition_name(&fragment.type_condition)),
                    &fragment.selection_set,
                    fragments,
                    &mut HashSet::new(),
                    true,
                )?;
                definitions.push(Definition::Fragment(fragment));
            }
            Definition::Fragment(_) => {}
        }
    }
    Ok(Document { definitions })
}

fn optimize_operation(
    schema: &SchemaInfo<'_>,
    operation: &OperationDefinition<'static, String>,
    fragments: &HashMap<String, FragmentDefinition<'static, String>>,
) -> Result<OperationDefinition<'static, String>> {
    Ok(match operation {
        OperationDefinition::SelectionSet(selection_set) => {
            OperationDefinition::SelectionSet(optimize_selection_set(
                schema,
                schema.query_type.as_deref(),
                selection_set,
                fragments,
                &mut HashSet::new(),
                false,
            )?)
        }
        OperationDefinition::Query(query) => {
            let mut query = query.clone();
            query.selection_set = optimize_selection_set(
                schema,
                schema.query_type.as_deref(),
                &query.selection_set,
                fragments,
                &mut HashSet::new(),
                false,
            )?;
            OperationDefinition::Query(query)
        }
        OperationDefinition::Mutation(mutation) => {
            let mut mutation = mutation.clone();
            mutation.selection_set = optimize_selection_set(
                schema,
                schema.mutation_type.as_deref(),
                &mutation.selection_set,
                fragments,
                &mut HashSet::new(),
                false,
            )?;
            OperationDefinition::Mutation(mutation)
        }
        OperationDefinition::Subscription(subscription) => {
            let mut subscription = subscription.clone();
            subscription.selection_set = optimize_selection_set(
                schema,
                schema.subscription_type.as_deref(),
                &subscription.selection_set,
                fragments,
                &mut HashSet::new(),
                false,
            )?;
            OperationDefinition::Subscription(subscription)
        }
    })
}

fn optimize_selection_set(
    schema: &SchemaInfo<'_>,
    parent_type: Option<&str>,
    selection_set: &SelectionSet<'static, String>,
    fragments: &HashMap<String, FragmentDefinition<'static, String>>,
    seen_fragments: &mut HashSet<String>,
    in_fragment: bool,
) -> Result<SelectionSet<'static, String>> {
    let mut expanded = Vec::new();
    for selection in &selection_set.items {
        match selection {
            Selection::Field(field) => {
                let mut field = field.clone();
                if !field.selection_set.items.is_empty() {
                    let field_parent_type =
                        parent_type.and_then(|parent| schema.field_named_type(parent, &field.name));
                    field.selection_set = optimize_selection_set(
                        schema,
                        field_parent_type.as_deref(),
                        &field.selection_set,
                        fragments,
                        seen_fragments,
                        in_fragment,
                    )?;
                }
                expanded.push(Selection::Field(field));
            }
            Selection::FragmentSpread(spread) => {
                let Some(fragment) = fragments.get(&spread.fragment_name) else {
                    expanded.push(selection.clone());
                    continue;
                };
                if !seen_fragments.insert(spread.fragment_name.clone()) {
                    continue;
                }
                let optimized = optimize_selection_set(
                    schema,
                    Some(type_condition_name(&fragment.type_condition)),
                    &fragment.selection_set,
                    fragments,
                    seen_fragments,
                    true,
                )?;
                expanded.extend(optimized.items);
                seen_fragments.remove(&spread.fragment_name);
            }
            Selection::InlineFragment(inline_fragment) => {
                let mut inline_fragment = inline_fragment.clone();
                let inline_parent_type = inline_fragment
                    .type_condition
                    .as_ref()
                    .map(type_condition_name)
                    .or(parent_type);
                inline_fragment.selection_set = optimize_selection_set(
                    schema,
                    inline_parent_type,
                    &inline_fragment.selection_set,
                    fragments,
                    seen_fragments,
                    in_fragment,
                )?;
                if inline_parent_type == parent_type {
                    expanded.extend(inline_fragment.selection_set.items);
                } else {
                    expanded.push(Selection::InlineFragment(inline_fragment));
                }
            }
        }
    }

    let mut items = merge_duplicate_fields(expanded);
    if in_fragment {
        sort_scalar_fields_by_schema(schema, parent_type, &mut items);
    }

    Ok(SelectionSet {
        span: selection_set.span,
        items,
    })
}

fn merge_duplicate_fields(
    items: Vec<Selection<'static, String>>,
) -> Vec<Selection<'static, String>> {
    let mut merged: Vec<Selection<'static, String>> = Vec::new();
    let mut field_indexes: HashMap<String, usize> = HashMap::new();

    for item in items {
        let Selection::Field(field) = item else {
            merged.push(item);
            continue;
        };

        let key = response_key(&field);
        let Some(index) = field_indexes.get(&key).copied() else {
            field_indexes.insert(key, merged.len());
            merged.push(Selection::Field(field));
            continue;
        };

        let Selection::Field(existing) = &mut merged[index] else {
            continue;
        };
        if existing.selection_set.items.is_empty() {
            continue;
        }
        existing
            .selection_set
            .items
            .extend(field.selection_set.items.clone());
        existing.selection_set.items = merge_duplicate_fields(existing.selection_set.items.clone());
    }

    merged
}

fn response_key(field: &Field<'static, String>) -> String {
    field.alias.clone().unwrap_or_else(|| field.name.clone())
}

fn sort_scalar_fields_by_schema(
    schema: &SchemaInfo<'_>,
    parent_type: Option<&str>,
    items: &mut [Selection<'static, String>],
) {
    let Some(parent_type) = parent_type else {
        return;
    };
    if !items.iter().all(
        |selection| matches!(selection, Selection::Field(field) if field.selection_set.items.is_empty()),
    ) {
        return;
    }

    items.sort_by_key(|selection| match selection {
        Selection::Field(field) => schema
            .field_order_index(parent_type, &field.name)
            .unwrap_or(usize::MAX),
        _ => usize::MAX,
    });
}

fn type_condition_name<'a>(type_condition: &'a TypeCondition<'static, String>) -> &'a str {
    match type_condition {
        TypeCondition::On(name) => name,
    }
}

struct SchemaInfo<'a> {
    query_type: Option<String>,
    mutation_type: Option<String>,
    subscription_type: Option<String>,
    types_by_name: HashMap<String, &'a Value>,
}

impl<'a> SchemaInfo<'a> {
    fn new(schema: &'a SchemaGenerationInput) -> Self {
        let mut types_by_name = HashMap::new();
        if let Some(types) = schema.introspection.get("types").and_then(|t| t.as_array()) {
            for ty in types {
                if let Some(name) = ty.get("name").and_then(|n| n.as_str()) {
                    types_by_name.insert(name.to_string(), ty);
                }
            }
        }
        Self {
            query_type: root_type_name(&schema.introspection, "queryType"),
            mutation_type: root_type_name(&schema.introspection, "mutationType"),
            subscription_type: root_type_name(&schema.introspection, "subscriptionType"),
            types_by_name,
        }
    }

    fn field_named_type(&self, parent_type: &str, field_name: &str) -> Option<String> {
        let fields = self
            .types_by_name
            .get(parent_type)?
            .get("fields")?
            .as_array()?;
        for field in fields {
            if field.get("name").and_then(|n| n.as_str()) == Some(field_name) {
                return field.get("type").and_then(named_type_ref);
            }
        }
        None
    }

    fn field_order_index(&self, parent_type: &str, field_name: &str) -> Option<usize> {
        let fields = self
            .types_by_name
            .get(parent_type)?
            .get("fields")?
            .as_array()?;
        fields
            .iter()
            .position(|field| field.get("name").and_then(|n| n.as_str()) == Some(field_name))
    }
}

fn root_type_name(introspection: &Value, key: &str) -> Option<String> {
    introspection
        .get(key)?
        .get("name")?
        .as_str()
        .map(ToOwned::to_owned)
}

fn named_type_ref(type_ref: &Value) -> Option<String> {
    if let Some(name) = type_ref.get("name").and_then(|n| n.as_str()) {
        return Some(name.to_string());
    }
    type_ref.get("ofType").and_then(named_type_ref)
}
