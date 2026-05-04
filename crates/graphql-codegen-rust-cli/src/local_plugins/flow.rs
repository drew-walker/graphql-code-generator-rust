use serde_json::Value;

mod resolvers;

use super::introspection::{
    is_non_null, named_type, pascal_case, schema_object_types, sorted_fields, type_name,
};

pub fn combined_output(introspection: &Value) -> String {
    let mut out = String::new();
    out.push_str("// @flow\n\n");
    out.push_str("import { type GraphQLResolveInfo } from 'graphql';\n");
    out.push_str("export type $RequireFields<Origin, Keys> = $Diff<Origin, Keys> &\n");
    out.push_str("  $ObjMapi<Keys, <Key>(k: Key) => $NonMaybeType<$ElementType<Origin, Key>>>;\n");
    out.push_str("/** All built-in and custom scalars, mapped to their actual values */\n");
    out.push_str("export type Scalars = {|\n");
    out.push_str("  ID: string,\n  String: string,\n  Boolean: boolean,\n  Int: number,\n  Float: number,\n|};\n");

    let objects = schema_object_types(introspection);
    for object in &objects {
        let name = type_name(object);
        out.push('\n');
        out.push_str(&format!("export type {name} = {{|\n"));
        out.push_str(&format!("  __typename?: '{name}',\n"));
        for field in sorted_fields(object) {
            let field_name = field.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let type_ref = field.get("type").unwrap_or(&Value::Null);
            let optional = !is_non_null(type_ref);
            out.push_str(&format!(
                "  {field_name}{}: {},\n",
                if optional { "?" } else { "" },
                flow_type_ref(type_ref, false)
            ));
        }
        out.push_str("|};\n");

        for field in sorted_fields(object) {
            let args = field
                .get("args")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            if args.is_empty() {
                continue;
            }
            let field_name = field.get("name").and_then(|v| v.as_str()).unwrap_or("");
            out.push('\n');
            out.push_str(&format!(
                "export type {name}{}Args = {{|\n",
                pascal_case(field_name)
            ));
            for arg in args {
                let arg_name = arg.get("name").and_then(|v| v.as_str()).unwrap_or("");
                out.push_str(&format!(
                    "  {arg_name}: {},\n",
                    flow_input_type_ref(arg.get("type").unwrap_or(&Value::Null))
                ));
            }
            out.push_str("|};\n");
        }
    }

    out.push_str(&resolvers::output(introspection, &objects));
    out
}

pub fn resolvers_output(introspection: &Value) -> String {
    let objects = schema_object_types(introspection);
    resolvers::output(introspection, &objects)
}

fn flow_type_ref(type_ref: &Value, nested: bool) -> String {
    match type_ref.get("kind").and_then(|v| v.as_str()).unwrap_or("") {
        "NON_NULL" => flow_type_ref(type_ref.get("ofType").unwrap_or(&Value::Null), true),
        "LIST" => {
            let inner = flow_type_ref(type_ref.get("ofType").unwrap_or(&Value::Null), false);
            let list = format!("Array<{inner}>");
            if nested { list } else { format!("?{list}") }
        }
        _ => {
            let name = named_type(type_ref);
            let base = if matches!(name.as_str(), "String" | "ID" | "Boolean" | "Int" | "Float") {
                format!("$ElementType<Scalars, '{name}'>")
            } else {
                name
            };
            if nested { base } else { format!("?{base}") }
        }
    }
}

fn flow_input_type_ref(type_ref: &Value) -> String {
    let name = named_type(type_ref);
    format!("$ElementType<Scalars, '{name}'>")
}
