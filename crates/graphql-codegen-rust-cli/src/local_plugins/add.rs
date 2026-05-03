use plugin_helpers::types::ComplexPluginOutput;

pub fn plugin(config: Option<&serde_json::Value>) -> anyhow::Result<ComplexPluginOutput> {
    let Some(config) = config else {
        return Ok(ComplexPluginOutput::default());
    };

    let (content, placement) = match config {
        serde_json::Value::String(content) => (vec![content.clone()], "prepend".to_string()),
        serde_json::Value::Object(map) => {
            let Some(content_value) = map.get("content") else {
                anyhow::bail!(
                    "Configuration provided for 'add' plugin is invalid: \"content\" is missing!"
                );
            };
            let content = match content_value {
                serde_json::Value::String(content) => vec![content.clone()],
                serde_json::Value::Array(items) => items
                    .iter()
                    .map(|item| {
                        item.as_str().map(ToOwned::to_owned).ok_or_else(|| {
                            anyhow::anyhow!(
                                "Configuration provided for 'add' plugin is invalid: \"content\" array must contain only strings!"
                            )
                        })
                    })
                    .collect::<anyhow::Result<Vec<_>>>()?,
                _ => {
                    anyhow::bail!(
                        "Configuration provided for 'add' plugin is invalid: \"content\" must be a string or an array of strings!"
                    );
                }
            };
            let placement = map
                .get("placement")
                .and_then(|v| v.as_str())
                .unwrap_or("prepend")
                .to_string();
            (content, placement)
        }
        other => {
            anyhow::bail!("Unsupported add plugin config: {other}");
        }
    };

    let mut out = ComplexPluginOutput::default();
    match placement.as_str() {
        "prepend" => out.prepend.extend(content),
        "content" => out.content = content.join("\n"),
        "append" => out.append.extend(content),
        other => anyhow::bail!(
            "Configuration provided for 'add' plugin is invalid: value of 'placement' field is not valid (valid values are: prepend, content, append): {other}"
        ),
    }
    Ok(out)
}
