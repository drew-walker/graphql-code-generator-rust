use serde_json::Value;

use crate::transitional_plugins::introspection::{named_type, schema_object_types, sorted_fields};

pub fn resolver_scalar_names(introspection: &Value) -> Vec<String> {
    let used = used_named_types(introspection);
    let mut scalars = introspection
        .get("types")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter(|ty| ty.get("kind").and_then(|v| v.as_str()) == Some("SCALAR"))
        .filter_map(|ty| ty.get("name").and_then(|v| v.as_str()))
        .filter(|name| !name.starts_with("__"))
        .filter(|name| *name == "Boolean" || used.contains(*name))
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    scalars.sort();
    scalars
}

pub fn ts_resolver_type_ref(type_ref: &Value) -> String {
    ts_resolver_type_ref_inner(type_ref, true)
}

fn used_named_types(introspection: &Value) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    for object in schema_object_types(introspection) {
        for field in sorted_fields(&object) {
            out.insert(named_type(field.get("type").unwrap_or(&Value::Null)));
            for arg in field
                .get("args")
                .and_then(|v| v.as_array())
                .into_iter()
                .flatten()
            {
                out.insert(named_type(arg.get("type").unwrap_or(&Value::Null)));
            }
        }
    }
    out
}

fn ts_resolver_type_ref_inner(type_ref: &Value, nullable: bool) -> String {
    match type_ref.get("kind").and_then(|v| v.as_str()).unwrap_or("") {
        "NON_NULL" => {
            ts_resolver_type_ref_inner(type_ref.get("ofType").unwrap_or(&Value::Null), false)
        }
        "LIST" => {
            let inner_ref = type_ref.get("ofType").unwrap_or(&Value::Null);
            let inner = ts_resolver_type_ref(inner_ref);
            let list = format!("Array<{inner}>");
            if nullable {
                format!("Maybe<{list}>")
            } else {
                list
            }
        }
        _ => {
            let named = format!("ResolversTypes['{}']", named_type(type_ref));
            if nullable {
                format!("Maybe<{named}>")
            } else {
                named
            }
        }
    }
}
