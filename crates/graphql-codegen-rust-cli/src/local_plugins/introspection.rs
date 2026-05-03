use serde_json::Value;

pub fn schema_object_types(introspection: &Value) -> Vec<Value> {
    let mut objects = introspection
        .get("types")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter(|ty| ty.get("kind").and_then(|v| v.as_str()) == Some("OBJECT"))
        .filter(|ty| !type_name(ty).starts_with("__"))
        .cloned()
        .collect::<Vec<_>>();
    objects.sort_by_key(|object| type_name(object).to_string());
    objects
}

pub fn sorted_fields(object: &Value) -> Vec<Value> {
    let mut fields = object
        .get("fields")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    fields.sort_by_key(|field| {
        field
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    });
    fields
}

pub fn type_name(value: &Value) -> &str {
    value.get("name").and_then(|v| v.as_str()).unwrap_or("")
}

pub fn root_type_name(introspection: &Value, key: &str) -> Option<String> {
    introspection
        .get(key)?
        .get("name")?
        .as_str()
        .map(ToOwned::to_owned)
}

pub fn is_non_null(type_ref: &Value) -> bool {
    type_ref.get("kind").and_then(|v| v.as_str()) == Some("NON_NULL")
}

pub fn named_type(type_ref: &Value) -> String {
    type_ref
        .get("name")
        .and_then(|v| v.as_str())
        .map(ToOwned::to_owned)
        .or_else(|| type_ref.get("ofType").map(named_type))
        .unwrap_or_else(|| "String".to_string())
}

pub fn pascal_case(s: &str) -> String {
    let mut out = String::new();
    let mut upper = true;
    for ch in s.chars() {
        if matches!(ch, '_' | '-' | ' ') {
            upper = true;
        } else if upper {
            out.extend(ch.to_uppercase());
            upper = false;
        } else {
            out.push(ch);
        }
    }
    out
}
