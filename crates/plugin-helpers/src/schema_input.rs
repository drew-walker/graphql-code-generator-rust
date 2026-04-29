use std::collections::HashMap;

use serde_json::Value;

/// Serialized schema + optional enum internal values (GraphQL.js `EnumValue.value`),
/// produced by `load_schema` and consumed by language plugins (e.g. `typescript`).
#[derive(Debug, Clone, Default)]
pub struct SchemaGenerationInput {
    /// The `__schema` object from an introspection result (not wrapped in `data`).
    pub introspection: Value,
    /// GraphQL enum type name → enum value name → serialized value (string/number/bool as string).
    pub enum_internal_values: HashMap<String, HashMap<String, String>>,
}
