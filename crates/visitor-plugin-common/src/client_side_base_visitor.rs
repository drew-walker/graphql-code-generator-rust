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

    // Deterministic root order (stable output).
    let mut names: Vec<String> = fragments.keys().cloned().collect();
    names.sort();

    let mut ordered = Vec::new();
    let mut visiting = HashSet::new();
    let mut visited = HashSet::new();

    for n in names {
        visit(&n, fragments, &mut visiting, &mut visited, &mut ordered);
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
    fn collect_deps_in_order(
        ss: &SelectionSet<'static, String>,
        fragments: &HashMap<String, FragmentDefinition<'static, String>>,
        seen: &mut HashSet<String>,
        out: &mut Vec<FragmentDefinition<'static, String>>,
    ) {
        for sel in &ss.items {
            match sel {
                Selection::FragmentSpread(spread) => {
                    if seen.insert(spread.fragment_name.clone()) {
                        if let Some(f) = fragments.get(&spread.fragment_name) {
                            collect_deps_in_order(&f.selection_set, fragments, seen, out);
                            out.push(f.clone());
                        }
                    }
                }
                Selection::InlineFragment(inline) => {
                    collect_deps_in_order(&inline.selection_set, fragments, seen, out);
                }
                Selection::Field(f) => {
                    collect_deps_in_order(&f.selection_set, fragments, seen, out)
                }
            }
        }
    }

    let mut deps: Vec<FragmentDefinition<'static, String>> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    match &root {
        Definition::Fragment(f) => {
            collect_deps_in_order(&f.selection_set, fragments, &mut seen, &mut deps)
        }
        Definition::Operation(op) => {
            let ss = match op {
                OperationDefinition::Query(q) => &q.selection_set,
                OperationDefinition::Mutation(m) => &m.selection_set,
                OperationDefinition::Subscription(s) => &s.selection_set,
                OperationDefinition::SelectionSet(ss) => ss,
            };
            collect_deps_in_order(ss, fragments, &mut seen, &mut deps);
        }
    }

    let mut definitions = vec![root];
    definitions.extend(deps.into_iter().map(Definition::Fragment));
    Document { definitions }
}

// Keep API surface future-proof.
pub fn validate_typed_document_node_output_extension(_output_file: &str) -> Result<()> {
    Ok(())
}
