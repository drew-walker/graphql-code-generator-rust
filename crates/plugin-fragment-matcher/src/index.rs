use anyhow::{Context, Result};
use plugin_helpers::schema_input::SchemaGenerationInput;
use plugin_helpers::types::{ComplexPluginOutput, DocumentFile};
use serde_json::{Map, Value, json};

use crate::config::FragmentMatcherConfig;

pub fn plugin(
    schema: &SchemaGenerationInput,
    _documents: &[DocumentFile],
    config: &FragmentMatcherConfig,
    output_file: &str,
) -> Result<ComplexPluginOutput> {
    if !output_file.ends_with(".json") {
        anyhow::bail!("Plugin \"fragment-matcher\" currently requires extension to be .json");
    }

    let types = schema
        .introspection
        .get("types")
        .and_then(Value::as_array)
        .context("Plugin \"fragment-matcher\" couldn't introspect the schema")?;

    let mut union_and_interface_types: Vec<&Value> = types
        .iter()
        .filter(|ty| {
            matches!(
                ty.get("kind").and_then(Value::as_str),
                Some("UNION") | Some("INTERFACE")
            )
        })
        .collect();

    if config.deterministic {
        union_and_interface_types.sort_by(|left, right| {
            left.get("name")
                .and_then(Value::as_str)
                .cmp(&right.get("name").and_then(Value::as_str))
        });
    }

    let content = match config.apollo_client_version {
        2 => build_apollo_client_2_payload(&union_and_interface_types, config.deterministic),
        _ => build_apollo_client_3_payload(&union_and_interface_types, config.deterministic),
    };

    Ok(ComplexPluginOutput {
        content: serde_json::to_string_pretty(&content)?,
        prepend: vec![],
        append: vec![],
    })
}

fn build_apollo_client_3_payload(types: &[&Value], deterministic: bool) -> Value {
    let mut possible_types = Map::new();
    for ty in types {
        let Some(name) = ty.get("name").and_then(Value::as_str) else {
            continue;
        };
        let values = sorted_possible_type_names(ty, deterministic)
            .into_iter()
            .map(Value::String)
            .collect::<Vec<_>>();
        possible_types.insert(name.to_string(), Value::Array(values));
    }

    json!({ "possibleTypes": possible_types })
}

fn build_apollo_client_2_payload(types: &[&Value], deterministic: bool) -> Value {
    let filtered_types = types
        .iter()
        .filter_map(|ty| {
            let kind = ty.get("kind")?.as_str()?;
            let name = ty.get("name")?.as_str()?;
            let possible_types = sorted_possible_type_names(ty, deterministic)
                .into_iter()
                .map(|name| json!({ "name": name }))
                .collect::<Vec<_>>();
            Some(json!({
                "kind": kind,
                "name": name,
                "possibleTypes": possible_types,
            }))
        })
        .collect::<Vec<_>>();

    json!({
        "__schema": {
            "types": filtered_types,
        }
    })
}

fn sorted_possible_type_names(ty: &Value, deterministic: bool) -> Vec<String> {
    let mut names = ty
        .get("possibleTypes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|possible_type| possible_type.get("name").and_then(Value::as_str))
        .map(ToString::to_string)
        .collect::<Vec<_>>();

    if deterministic {
        names.sort();
    }

    names
}

#[cfg(test)]
mod tests {
    use super::*;
    use plugin_helpers::schema_input::SchemaGenerationInput;
    use std::collections::HashMap;

    #[test]
    fn plugin_outputs_possible_types_json_for_apollo_client_3() {
        let schema = SchemaGenerationInput {
            introspection: json!({
                "types": [
                    {
                        "kind": "UNION",
                        "name": "People",
                        "possibleTypes": [
                            { "name": "Jedi" },
                            { "name": "Droid" }
                        ]
                    },
                    {
                        "kind": "OBJECT",
                        "name": "Query",
                        "possibleTypes": null
                    }
                ]
            }),
            enum_internal_values: HashMap::new(),
        };

        let output = plugin(&schema, &[], &FragmentMatcherConfig::default(), "foo.json").unwrap();
        assert_eq!(
            output.content,
            "{\n  \"possibleTypes\": {\n    \"People\": [\n      \"Jedi\",\n      \"Droid\"\n    ]\n  }\n}"
        );
    }
}
