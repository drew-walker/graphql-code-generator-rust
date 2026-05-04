//! Minimal port of `ClientSideBaseVisitor`-adjacent utilities from upstream.
//!
//! Upstream reference:
//! - `packages/plugins/other/visitor-plugin-common/src/client-side-base-visitor.ts`
//! - `packages/plugins/typescript/typed-document-node/src/visitor.ts`
//!
//! This module focuses narrowly on printing GraphQL query ASTs as TS `DocumentNode` object literals
//! (the shape used by the `typed-document-node` plugin fixtures).

use std::collections::{HashMap, HashSet};

use anyhow::Result;
use graphql_parser::query::{
    Definition, Document, Field, FragmentDefinition, InlineFragment, OperationDefinition,
    Selection, SelectionSet, Type, TypeCondition, Value, VariableDefinition,
};
use serde_json::value::Map;
use serde_json::{Value as JsonValue, json};

pub fn collect_fragments(
    doc: &Document<'static, String>,
) -> HashMap<String, FragmentDefinition<'static, String>> {
    let mut out = HashMap::new();
    for def in &doc.definitions {
        if let Definition::Fragment(f) = def {
            out.insert(f.name.clone(), f.clone());
        }
    }
    out
}

pub fn order_fragments(
    fragments: &HashMap<String, FragmentDefinition<'static, String>>,
) -> Vec<String> {
    // This legacy helper has no stable insertion order (HashMap). Keep it deterministic by sorting.
    let mut roots: Vec<String> = fragments.keys().cloned().collect();
    roots.sort();
    order_fragments_with_roots(&roots, fragments)
}

pub fn order_fragments_with_roots(
    roots: &[String],
    fragments: &HashMap<String, FragmentDefinition<'static, String>>,
) -> Vec<String> {
    fn deps_for_selection_set(ss: &SelectionSet<'static, String>, out: &mut Vec<String>) {
        for sel in &ss.items {
            match sel {
                Selection::FragmentSpread(spread) => out.push(spread.fragment_name.clone()),
                Selection::InlineFragment(inline) => {
                    deps_for_selection_set(&inline.selection_set, out)
                }
                Selection::Field(f) => deps_for_selection_set(&f.selection_set, out),
            }
        }
    }

    fn visit(
        name: &str,
        fragments: &HashMap<String, FragmentDefinition<'static, String>>,
        visiting: &mut HashSet<String>,
        visited: &mut HashSet<String>,
        ordered: &mut Vec<String>,
    ) {
        if visited.contains(name) {
            return;
        }
        if !visiting.insert(name.to_string()) {
            // Cycle: upstream `dependency-graph` can handle cycles by returning a best-effort order.
            // We keep behavior deterministic by short-circuiting.
            return;
        }
        if let Some(f) = fragments.get(name) {
            let mut deps = Vec::new();
            deps_for_selection_set(&f.selection_set, &mut deps);
            for d in deps {
                if fragments.contains_key(&d) {
                    visit(&d, fragments, visiting, visited, ordered);
                }
            }
        }
        visiting.remove(name);
        visited.insert(name.to_string());
        ordered.push(name.to_string());
    }

    let mut ordered = Vec::new();
    let mut visiting = HashSet::new();
    let mut visited = HashSet::new();
    for r in roots {
        if fragments.contains_key(r) {
            visit(r, fragments, &mut visiting, &mut visited, &mut ordered);
        }
    }
    ordered
}

pub fn print_document_with_single_definition(def: Definition<'static, String>) -> String {
    print_document(&Document {
        definitions: vec![def],
    })
}

pub fn print_document(doc: &Document<'static, String>) -> String {
    let mut out = String::new();
    out.push_str("{\n");
    out.push_str("  kind: 'Document',\n");
    out.push_str("  definitions: [\n");
    for def in &doc.definitions {
        out.push_str(&indent(&print_definition(def), 4));
    }
    out.push_str("  ],\n");
    out.push('}');
    out
}

/// JSON-stringify style printer, closer to upstream's `JSON.stringify(DocumentNode)` output.
///
/// This intentionally emits **compact JSON** (double quotes, quoted keys). Prettier is expected
/// to normalize it to idiomatic JS object literal formatting.
pub fn print_document_json(doc: &Document<'static, String>) -> String {
    serde_json::to_string(&document_to_json(doc)).unwrap_or_else(|_| "{}".to_string())
}

fn print_definition(def: &Definition<'static, String>) -> String {
    match def {
        Definition::Fragment(f) => print_fragment_definition(f),
        Definition::Operation(op) => print_operation_definition(op),
    }
}

fn print_fragment_definition(f: &FragmentDefinition<'static, String>) -> String {
    let TypeCondition::On(type_name) = &f.type_condition;
    let mut out = String::new();
    out.push_str("{\n");
    out.push_str("  kind: 'FragmentDefinition',\n");
    out.push_str(&format!(
        "  name: {{ kind: 'Name', value: '{}' }},\n",
        f.name
    ));
    out.push_str(&format!(
        "  typeCondition: {{ kind: 'NamedType', name: {{ kind: 'Name', value: '{}' }} }},\n",
        type_name
    ));
    out.push_str("  selectionSet: ");
    out.push_str(&print_selection_set(&f.selection_set, 2));
    out.push_str(",\n");
    out.push_str("},\n");
    out
}

fn print_operation_definition(op: &OperationDefinition<'static, String>) -> String {
    match op {
        OperationDefinition::Query(q) => {
            let mut out = String::new();
            out.push_str("{\n");
            out.push_str("  kind: 'OperationDefinition',\n");
            out.push_str("  operation: 'query',\n");
            if let Some(name) = &q.name {
                out.push_str(&format!("  name: {{ kind: 'Name', value: '{}' }},\n", name));
            }
            if !q.variable_definitions.is_empty() {
                out.push_str("  variableDefinitions: [\n");
                for v in &q.variable_definitions {
                    out.push_str(&indent(&print_variable_definition(v), 4));
                }
                out.push_str("  ],\n");
            }
            out.push_str("  selectionSet: ");
            out.push_str(&print_selection_set(&q.selection_set, 2));
            out.push_str(",\n");
            out.push_str("},\n");
            out
        }
        OperationDefinition::Mutation(m) => {
            let mut out = String::new();
            out.push_str("{\n");
            out.push_str("  kind: 'OperationDefinition',\n");
            out.push_str("  operation: 'mutation',\n");
            if let Some(name) = &m.name {
                out.push_str(&format!("  name: {{ kind: 'Name', value: '{}' }},\n", name));
            }
            if !m.variable_definitions.is_empty() {
                out.push_str("  variableDefinitions: [\n");
                for v in &m.variable_definitions {
                    out.push_str(&indent(&print_variable_definition(v), 4));
                }
                out.push_str("  ],\n");
            }
            out.push_str("  selectionSet: ");
            out.push_str(&print_selection_set(&m.selection_set, 2));
            out.push_str(",\n");
            out.push_str("},\n");
            out
        }
        OperationDefinition::Subscription(s) => {
            let mut out = String::new();
            out.push_str("{\n");
            out.push_str("  kind: 'OperationDefinition',\n");
            out.push_str("  operation: 'subscription',\n");
            if let Some(name) = &s.name {
                out.push_str(&format!("  name: {{ kind: 'Name', value: '{}' }},\n", name));
            }
            if !s.variable_definitions.is_empty() {
                out.push_str("  variableDefinitions: [\n");
                for v in &s.variable_definitions {
                    out.push_str(&indent(&print_variable_definition(v), 4));
                }
                out.push_str("  ],\n");
            }
            out.push_str("  selectionSet: ");
            out.push_str(&print_selection_set(&s.selection_set, 2));
            out.push_str(",\n");
            out.push_str("},\n");
            out
        }
        OperationDefinition::SelectionSet(_) => String::new(),
    }
}

fn print_variable_definition(v: &VariableDefinition<'static, String>) -> String {
    let mut out = String::new();
    out.push_str("{\n");
    out.push_str("  kind: 'VariableDefinition',\n");
    out.push_str(&format!(
        "  variable: {{ kind: 'Variable', name: {{ kind: 'Name', value: '{}' }} }},\n",
        v.name
    ));
    out.push_str("  type: ");
    out.push_str(&print_type(&v.var_type));
    out.push_str(",\n");
    out.push_str("},\n");
    out
}

fn print_type(t: &Type<'static, String>) -> String {
    match t {
        Type::NamedType(n) => format!(
            "{{ kind: 'NamedType', name: {{ kind: 'Name', value: '{}' }} }}",
            n
        ),
        Type::ListType(inner) => format!("{{ kind: 'ListType', type: {} }}", print_type(inner)),
        Type::NonNullType(inner) => {
            format!("{{ kind: 'NonNullType', type: {} }}", print_type(inner))
        }
    }
}

fn print_selection_set(ss: &SelectionSet<'static, String>, indent_level: usize) -> String {
    let mut out = String::new();
    out.push_str("{\n");
    out.push_str(&" ".repeat(indent_level));
    out.push_str("  kind: 'SelectionSet',\n");
    out.push_str(&" ".repeat(indent_level));
    out.push_str("  selections: [\n");
    for sel in &ss.items {
        let printed = print_selection(sel);
        out.push_str(&indent(&printed, indent_level + 4));
    }
    out.push_str(&" ".repeat(indent_level));
    out.push_str("  ],\n");
    out.push_str(&" ".repeat(indent_level));
    out.push('}');
    out
}

fn print_selection(sel: &Selection<'static, String>) -> String {
    match sel {
        Selection::Field(f) => print_field(f),
        Selection::FragmentSpread(spread) => format!(
            "{{ kind: 'FragmentSpread', name: {{ kind: 'Name', value: '{}' }} }},",
            spread.fragment_name
        ),
        Selection::InlineFragment(inline) => print_inline_fragment(inline),
    }
}

fn print_field(f: &Field<'static, String>) -> String {
    let is_leaf = f.alias.is_none() && f.arguments.is_empty() && f.selection_set.items.is_empty();
    if is_leaf {
        return format!(
            "{{ kind: 'Field', name: {{ kind: 'Name', value: '{}' }} }},",
            f.name
        );
    }

    let mut out = String::new();
    out.push_str("{\n");
    out.push_str("  kind: 'Field',\n");
    out.push_str(&format!(
        "  name: {{ kind: 'Name', value: '{}' }},\n",
        f.name
    ));
    if let Some(alias) = &f.alias {
        out.push_str(&format!(
            "  alias: {{ kind: 'Name', value: '{}' }},\n",
            alias
        ));
    }
    if !f.arguments.is_empty() {
        out.push_str("  arguments: [\n");
        for a in &f.arguments {
            out.push_str("    {\n");
            out.push_str("      kind: 'Argument',\n");
            out.push_str(&format!(
                "      name: {{ kind: 'Name', value: '{}' }},\n",
                a.0
            ));
            out.push_str("      value: ");
            out.push_str(&print_value(&a.1));
            out.push_str(",\n");
            out.push_str("    },\n");
        }
        out.push_str("  ],\n");
    }
    if !f.selection_set.items.is_empty() {
        out.push_str("  selectionSet: ");
        out.push_str(&print_selection_set(&f.selection_set, 2));
        out.push_str(",\n");
    }
    out.push_str("},");
    out
}

fn print_inline_fragment(inline: &InlineFragment<'static, String>) -> String {
    let mut out = String::new();
    out.push_str("{\n");
    out.push_str("  kind: 'InlineFragment',\n");
    if let Some(TypeCondition::On(t)) = &inline.type_condition {
        out.push_str(&format!(
            "  typeCondition: {{ kind: 'NamedType', name: {{ kind: 'Name', value: '{}' }} }},\n",
            t
        ));
    }
    out.push_str("  selectionSet: ");
    out.push_str(&print_selection_set(&inline.selection_set, 2));
    out.push_str(",\n");
    out.push_str("},");
    out
}

fn print_value(v: &Value<'static, String>) -> String {
    match v {
        Value::Variable(name) => format!(
            "{{ kind: 'Variable', name: {{ kind: 'Name', value: '{}' }} }}",
            name
        ),
        Value::Int(i) => format!(
            "{{ kind: 'IntValue', value: '{}' }}",
            i.as_i64().unwrap_or(0)
        ),
        Value::Float(f) => format!("{{ kind: 'FloatValue', value: '{}' }}", f),
        Value::String(s) => format!("{{ kind: 'StringValue', value: '{}' }}", escape_string(s)),
        Value::Boolean(b) => format!("{{ kind: 'BooleanValue', value: {} }}", b),
        Value::Null => "{ kind: 'NullValue' }".to_string(),
        Value::Enum(e) => format!("{{ kind: 'EnumValue', value: '{}' }}", e),
        Value::List(items) => {
            let mut out = String::new();
            out.push_str("{ kind: 'ListValue', values: [");
            for (idx, it) in items.iter().enumerate() {
                if idx > 0 {
                    out.push_str(", ");
                }
                out.push_str(&print_value(it));
            }
            out.push_str("] }");
            out
        }
        Value::Object(obj) => {
            let mut out = String::new();
            out.push_str("{ kind: 'ObjectValue', fields: [");
            for (idx, (k, vv)) in obj.iter().enumerate() {
                if idx > 0 {
                    out.push_str(", ");
                }
                out.push_str(&format!(
                    "{{ kind: 'ObjectField', name: {{ kind: 'Name', value: '{}' }}, value: {} }}",
                    k,
                    print_value(vv)
                ));
            }
            out.push_str("] }");
            out
        }
    }
}

fn escape_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}

fn indent(s: &str, n: usize) -> String {
    let pad = " ".repeat(n);
    s.lines().map(|l| format!("{pad}{l}\n")).collect::<String>()
}

pub fn merge_documents(
    documents: &[plugin_helpers::types::DocumentFile],
) -> Document<'static, String> {
    let mut defs: Vec<Definition<'static, String>> = Vec::new();
    for d in documents {
        for def in &d.document.definitions {
            defs.push(def.clone());
        }
    }
    Document { definitions: defs }
}

pub fn build_document_with_dependent_fragments(
    root: Definition<'static, String>,
    fragments: &HashMap<String, FragmentDefinition<'static, String>>,
) -> Document<'static, String> {
    fn collect_fragment_spread_names_in_order(
        ss: &SelectionSet<'static, String>,
        out: &mut Vec<String>,
    ) {
        for sel in &ss.items {
            match sel {
                Selection::FragmentSpread(spread) => out.push(spread.fragment_name.clone()),
                Selection::InlineFragment(inline) => {
                    collect_fragment_spread_names_in_order(&inline.selection_set, out)
                }
                Selection::Field(f) => {
                    collect_fragment_spread_names_in_order(&f.selection_set, out)
                }
            }
        }
    }

    let mut root_spreads: Vec<String> = Vec::new();
    match &root {
        Definition::Fragment(f) => {
            collect_fragment_spread_names_in_order(&f.selection_set, &mut root_spreads)
        }
        Definition::Operation(op) => {
            let ss = match op {
                OperationDefinition::Query(q) => &q.selection_set,
                OperationDefinition::Mutation(m) => &m.selection_set,
                OperationDefinition::Subscription(s) => &s.selection_set,
                OperationDefinition::SelectionSet(ss) => ss,
            };
            collect_fragment_spread_names_in_order(ss, &mut root_spreads);
        }
    }

    // Dedupe root spreads while keeping first-seen order (closer to upstream DepGraph roots).
    let mut roots: Vec<String> = Vec::new();
    let mut seen = HashSet::new();
    for n in root_spreads {
        if seen.insert(n.clone()) {
            roots.push(n);
        }
    }

    // Topologically order reachable fragments with dependency-before-dependent semantics.
    let ordered_names = order_fragments_with_roots(&roots, fragments);
    let mut definitions = vec![root];
    for n in ordered_names {
        if let Some(f) = fragments.get(&n) {
            definitions.push(Definition::Fragment(f.clone()));
        }
    }
    Document { definitions }
}

// Keep API surface future-proof.
pub fn validate_typed_document_node_output_extension(_output_file: &str) -> Result<()> {
    Ok(())
}

fn document_to_json(doc: &Document<'static, String>) -> serde_json::Value {
    let mut m = Map::new();
    m.insert("kind".to_string(), json!("Document"));
    m.insert(
        "definitions".to_string(),
        JsonValue::Array(doc.definitions.iter().map(definition_to_json).collect()),
    );
    JsonValue::Object(m)
}

fn definition_to_json(def: &Definition<'static, String>) -> serde_json::Value {
    match def {
        Definition::Fragment(f) => fragment_definition_to_json(f),
        Definition::Operation(op) => operation_definition_to_json(op),
    }
}

fn fragment_definition_to_json(f: &FragmentDefinition<'static, String>) -> serde_json::Value {
    let TypeCondition::On(type_name) = &f.type_condition;
    let mut m = Map::new();
    m.insert("kind".to_string(), json!("FragmentDefinition"));
    m.insert("name".to_string(), name_to_json(&f.name));
    m.insert(
        "directives".to_string(),
        JsonValue::Array(f.directives.iter().map(directive_to_json).collect()),
    );
    m.insert("typeCondition".to_string(), named_type_to_json(type_name));
    m.insert(
        "selectionSet".to_string(),
        selection_set_to_json(&f.selection_set),
    );
    JsonValue::Object(m)
}

fn operation_definition_to_json(op: &OperationDefinition<'static, String>) -> serde_json::Value {
    match op {
        OperationDefinition::Query(q) => {
            let mut m = Map::new();
            m.insert("kind".to_string(), json!("OperationDefinition"));
            m.insert("operation".to_string(), json!("query"));
            if let Some(name) = &q.name {
                m.insert("name".to_string(), name_to_json(name));
            }
            m.insert(
                "variableDefinitions".to_string(),
                JsonValue::Array(
                    q.variable_definitions
                        .iter()
                        .map(variable_definition_to_json)
                        .collect(),
                ),
            );
            m.insert(
                "directives".to_string(),
                JsonValue::Array(q.directives.iter().map(directive_to_json).collect()),
            );
            m.insert(
                "selectionSet".to_string(),
                selection_set_to_json(&q.selection_set),
            );
            JsonValue::Object(m)
        }
        OperationDefinition::Mutation(mutation) => {
            let mut m = Map::new();
            m.insert("kind".to_string(), json!("OperationDefinition"));
            m.insert("operation".to_string(), json!("mutation"));
            if let Some(name) = &mutation.name {
                m.insert("name".to_string(), name_to_json(name));
            }
            m.insert(
                "variableDefinitions".to_string(),
                JsonValue::Array(
                    mutation
                        .variable_definitions
                        .iter()
                        .map(variable_definition_to_json)
                        .collect(),
                ),
            );
            m.insert(
                "directives".to_string(),
                JsonValue::Array(mutation.directives.iter().map(directive_to_json).collect()),
            );
            m.insert(
                "selectionSet".to_string(),
                selection_set_to_json(&mutation.selection_set),
            );
            JsonValue::Object(m)
        }
        OperationDefinition::Subscription(sub) => {
            let mut m = Map::new();
            m.insert("kind".to_string(), json!("OperationDefinition"));
            m.insert("operation".to_string(), json!("subscription"));
            if let Some(name) = &sub.name {
                m.insert("name".to_string(), name_to_json(name));
            }
            m.insert(
                "variableDefinitions".to_string(),
                JsonValue::Array(
                    sub.variable_definitions
                        .iter()
                        .map(variable_definition_to_json)
                        .collect(),
                ),
            );
            m.insert(
                "directives".to_string(),
                JsonValue::Array(sub.directives.iter().map(directive_to_json).collect()),
            );
            m.insert(
                "selectionSet".to_string(),
                selection_set_to_json(&sub.selection_set),
            );
            JsonValue::Object(m)
        }
        OperationDefinition::SelectionSet(_) => json!(null),
    }
}

fn variable_definition_to_json(v: &VariableDefinition<'static, String>) -> serde_json::Value {
    let mut m = Map::new();
    m.insert("kind".to_string(), json!("VariableDefinition"));
    m.insert(
        "variable".to_string(),
        json!({ "kind": "Variable", "name": { "kind": "Name", "value": v.name } }),
    );
    m.insert("type".to_string(), type_to_json(&v.var_type));
    if let Some(default_value) = &v.default_value {
        m.insert("defaultValue".to_string(), value_to_json(default_value));
    }
    JsonValue::Object(m)
}

fn type_to_json(t: &Type<'static, String>) -> serde_json::Value {
    match t {
        Type::NamedType(n) => named_type_to_json(n),
        Type::ListType(inner) => {
            let mut m = Map::new();
            m.insert("kind".to_string(), json!("ListType"));
            m.insert("type".to_string(), type_to_json(inner));
            JsonValue::Object(m)
        }
        Type::NonNullType(inner) => {
            let mut m = Map::new();
            m.insert("kind".to_string(), json!("NonNullType"));
            m.insert("type".to_string(), type_to_json(inner));
            JsonValue::Object(m)
        }
    }
}

fn selection_set_to_json(ss: &SelectionSet<'static, String>) -> serde_json::Value {
    let mut m = Map::new();
    m.insert("kind".to_string(), json!("SelectionSet"));
    m.insert(
        "selections".to_string(),
        JsonValue::Array(ss.items.iter().map(selection_to_json).collect()),
    );
    JsonValue::Object(m)
}

fn selection_to_json(sel: &Selection<'static, String>) -> serde_json::Value {
    match sel {
        Selection::Field(f) => field_to_json(f),
        Selection::FragmentSpread(spread) => {
            let mut m = Map::new();
            m.insert("kind".to_string(), json!("FragmentSpread"));
            m.insert("name".to_string(), name_to_json(&spread.fragment_name));
            m.insert(
                "directives".to_string(),
                JsonValue::Array(spread.directives.iter().map(directive_to_json).collect()),
            );
            JsonValue::Object(m)
        }
        Selection::InlineFragment(inline) => inline_fragment_to_json(inline),
    }
}

fn field_to_json(f: &Field<'static, String>) -> serde_json::Value {
    let mut m = Map::new();
    m.insert("kind".to_string(), json!("Field"));
    if let Some(alias) = &f.alias {
        m.insert("alias".to_string(), name_to_json(alias));
    }
    m.insert("name".to_string(), name_to_json(&f.name));
    m.insert(
        "arguments".to_string(),
        JsonValue::Array(
            f.arguments
                .iter()
                .map(|(k, v)| {
                    let mut a = Map::new();
                    a.insert("kind".to_string(), json!("Argument"));
                    a.insert("name".to_string(), name_to_json(k));
                    a.insert("value".to_string(), value_to_json(v));
                    JsonValue::Object(a)
                })
                .collect(),
        ),
    );
    m.insert(
        "directives".to_string(),
        JsonValue::Array(f.directives.iter().map(directive_to_json).collect()),
    );
    if !f.selection_set.items.is_empty() {
        m.insert(
            "selectionSet".to_string(),
            selection_set_to_json(&f.selection_set),
        );
    }
    JsonValue::Object(m)
}

fn inline_fragment_to_json(inline: &InlineFragment<'static, String>) -> serde_json::Value {
    let mut m = Map::new();
    m.insert("kind".to_string(), json!("InlineFragment"));
    if let Some(TypeCondition::On(t)) = &inline.type_condition {
        m.insert("typeCondition".to_string(), named_type_to_json(t));
    }
    m.insert(
        "directives".to_string(),
        JsonValue::Array(inline.directives.iter().map(directive_to_json).collect()),
    );
    m.insert(
        "selectionSet".to_string(),
        selection_set_to_json(&inline.selection_set),
    );
    JsonValue::Object(m)
}

fn directive_to_json(directive: &graphql_parser::query::Directive<'static, String>) -> JsonValue {
    let mut m = Map::new();
    m.insert("kind".to_string(), json!("Directive"));
    m.insert("name".to_string(), name_to_json(&directive.name));
    let args = directive
        .arguments
        .iter()
        .map(|(k, v)| {
            let mut a = Map::new();
            a.insert("kind".to_string(), json!("Argument"));
            a.insert("name".to_string(), name_to_json(k));
            a.insert("value".to_string(), value_to_json(v));
            JsonValue::Object(a)
        })
        .collect::<Vec<_>>();
    m.insert("arguments".to_string(), JsonValue::Array(args));
    JsonValue::Object(m)
}

fn value_to_json(v: &Value<'static, String>) -> serde_json::Value {
    match v {
        Value::Variable(name) => {
            let mut m = Map::new();
            m.insert("kind".to_string(), json!("Variable"));
            m.insert("name".to_string(), name_to_json(name));
            JsonValue::Object(m)
        }
        Value::Int(i) => {
            json!({ "kind": "IntValue", "value": format!("{}", i.as_i64().unwrap_or(0)) })
        }
        Value::Float(f) => json!({ "kind": "FloatValue", "value": format!("{}", f) }),
        Value::String(s) => json!({ "kind": "StringValue", "value": s, "block": false }),
        Value::Boolean(b) => json!({ "kind": "BooleanValue", "value": b }),
        Value::Null => json!({ "kind": "NullValue" }),
        Value::Enum(e) => json!({ "kind": "EnumValue", "value": e }),
        Value::List(items) => {
            let mut m = Map::new();
            m.insert("kind".to_string(), json!("ListValue"));
            m.insert(
                "values".to_string(),
                JsonValue::Array(items.iter().map(value_to_json).collect()),
            );
            JsonValue::Object(m)
        }
        Value::Object(obj) => {
            let mut m = Map::new();
            m.insert("kind".to_string(), json!("ObjectValue"));
            let fields = obj
                .iter()
                .map(|(k, vv)| {
                    let mut f = Map::new();
                    f.insert("kind".to_string(), json!("ObjectField"));
                    f.insert("name".to_string(), name_to_json(k));
                    f.insert("value".to_string(), value_to_json(vv));
                    JsonValue::Object(f)
                })
                .collect::<Vec<_>>();
            m.insert("fields".to_string(), JsonValue::Array(fields));
            JsonValue::Object(m)
        }
    }
}

fn name_to_json(value: &str) -> JsonValue {
    let mut m = Map::new();
    m.insert("kind".to_string(), json!("Name"));
    m.insert("value".to_string(), json!(value));
    JsonValue::Object(m)
}

fn named_type_to_json(type_name: &str) -> JsonValue {
    let mut m = Map::new();
    m.insert("kind".to_string(), json!("NamedType"));
    m.insert("name".to_string(), name_to_json(type_name));
    JsonValue::Object(m)
}
