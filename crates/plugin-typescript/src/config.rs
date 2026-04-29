//! Port of `packages/plugins/typescript/typescript/src/config.ts` (subset; grows with parity).

use serde::Deserialize;

/// Mechanical port of TS `getConfigValue<T>(option, defaultValue): T`.
pub fn get_config_value<T: Clone>(option: Option<&T>, default: T) -> T {
    option.cloned().unwrap_or(default)
}

/// Raw plugin options from `generates[output].config` — mirrors `TypeScriptPluginConfig` (partial).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TypeScriptPluginConfig {
    #[serde(default)]
    pub maybe_value: Option<String>,
    #[serde(default)]
    pub input_maybe_value: Option<String>,
    #[serde(default)]
    pub only_enums: Option<bool>,
    #[serde(default)]
    pub no_export: Option<bool>,
    #[serde(default)]
    pub avoid_optionals: Option<bool>,
    #[serde(default)]
    pub wrap_field_definitions: Option<bool>,
    #[serde(default)]
    pub wrap_entire_field_definitions: Option<bool>,
}

impl TypeScriptPluginConfig {
    /// Merge `generates[output].config` JSON into [`TypeScriptPluginConfig`].
    pub fn from_output_config_map(map: &serde_json::Map<String, serde_json::Value>) -> Self {
        serde_json::from_value(serde_json::Value::Object(map.clone())).unwrap_or_default()
    }
}

/// Normalized config — mirrors `TypeScriptPluginParsedConfig` from `visitor.ts`.
#[derive(Debug, Clone)]
pub struct TypeScriptPluginParsedConfig {
    pub maybe_value: String,
    pub input_maybe_value: String,
    pub only_enums: bool,
    pub no_export: bool,
    pub avoid_optionals: bool,
    pub wrap_field_definitions: bool,
    pub wrap_entire_field_definitions: bool,
}

impl TypeScriptPluginParsedConfig {
    /// Mirrors the `super(schema, pluginConfig, { ... })` block in `TsVisitor`’s constructor.
    pub fn new(raw: &TypeScriptPluginConfig) -> Self {
        let maybe_value = get_config_value(raw.maybe_value.as_ref(), "T | null".to_string());
        let default_for_input_maybe =
            get_config_value(raw.maybe_value.as_ref(), "Maybe<T>".to_string());
        let input_maybe_value =
            get_config_value(raw.input_maybe_value.as_ref(), default_for_input_maybe);
        Self {
            maybe_value,
            input_maybe_value,
            only_enums: get_config_value(raw.only_enums.as_ref(), false),
            no_export: get_config_value(raw.no_export.as_ref(), false),
            avoid_optionals: get_config_value(raw.avoid_optionals.as_ref(), false),
            wrap_field_definitions: get_config_value(raw.wrap_field_definitions.as_ref(), false),
            wrap_entire_field_definitions: get_config_value(
                raw.wrap_entire_field_definitions.as_ref(),
                false,
            ),
        }
    }
}
