use serde_json::Value;

#[derive(Debug, Clone)]
pub struct ParsedResolversConfig {
    pub use_index_signature: bool,
    pub no_schema_stitching: bool,
    pub federation: bool,
}

impl ParsedResolversConfig {
    pub fn from_map(config: &serde_json::Map<String, Value>) -> Self {
        Self {
            use_index_signature: config
                .get("useIndexSignature")
                .and_then(|value| value.as_bool())
                .unwrap_or(false),
            no_schema_stitching: config
                .get("noSchemaStitching")
                .and_then(|value| value.as_bool())
                .unwrap_or(true),
            federation: config
                .get("federation")
                .and_then(|value| value.as_bool())
                .unwrap_or(false),
        }
    }
}

pub fn is_custom_scalar(scalar: &str) -> bool {
    scalar != "Boolean"
        && scalar != "ID"
        && scalar != "String"
        && scalar != "Int"
        && scalar != "Float"
}

pub fn split_external_mapper(mapper: &str) -> Option<(String, String)> {
    let cleaned = mapper.replace("\\#", "#");
    let idx = cleaned.rfind('#')?;
    let source = cleaned[..idx].to_string();
    let imported = cleaned[idx + 1..].to_string();
    if source.is_empty() || imported.is_empty() {
        None
    } else {
        Some((source, imported))
    }
}
