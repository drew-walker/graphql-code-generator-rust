//! Port of `packages/plugins/typescript/typescript/src/visitor.ts` (subset; grows with parity).

use plugin_helpers::schema_input::SchemaGenerationInput;
use serde_json::Value;

use crate::config::{TypeScriptPluginConfig, TypeScriptPluginParsedConfig};

// --- Same string constants as upstream `visitor.ts` (without `export`; prefix added in getters) ---

pub const EXACT_SIGNATURE: &str =
    "type Exact<T extends { [key: string]: unknown }> = { [K in keyof T]: T[K] };";
pub const MAKE_OPTIONAL_SIGNATURE: &str =
    "type MakeOptional<T, K extends keyof T> = Omit<T, K> & { [SubKey in K]?: Maybe<T[SubKey]> };";
pub const MAKE_MAYBE_SIGNATURE: &str =
    "type MakeMaybe<T, K extends keyof T> = Omit<T, K> & { [SubKey in K]: Maybe<T[SubKey]> };";
pub const MAKE_EMPTY_SIGNATURE: &str = "type MakeEmpty<T extends { [key: string]: unknown }, K extends keyof T> = { [_ in K]?: never };";
pub const MAKE_INCREMENTAL_SIGNATURE: &str = "type Incremental<T> = T | { [P in keyof T]?: P extends ' $fragmentName' | '__typename' ? T[P] : never };";

/// Mirrors `TsVisitor` from `visitor.ts` (constructor + wrapper/scalars helpers + transitional introspection emit).
pub struct TsVisitor<'a> {
    pub(crate) schema_input: &'a SchemaGenerationInput,
    pub(crate) config: TypeScriptPluginParsedConfig,
}

impl<'a> TsVisitor<'a> {
    /// Mirrors `constructor(schema, pluginConfig, additionalConfig?)`.
    pub fn new(
        schema_input: &'a SchemaGenerationInput,
        plugin_config: &TypeScriptPluginConfig,
    ) -> Self {
        Self {
            schema_input,
            config: TypeScriptPluginParsedConfig::new(plugin_config),
        }
    }

    // --- `index.ts` prepend assembly (subset of import getters stubbed) ---

    /// Mirrors `getEnumsImports()`.
    pub fn get_enums_imports(&self) -> Vec<String> {
        vec![]
    }

    /// Mirrors `getDirectiveArgumentAndInputFieldMappingsImports()`.
    pub fn get_directive_argument_and_input_field_mappings_imports(&self) -> Vec<String> {
        vec![]
    }

    /// Mirrors `getScalarsImports()`.
    pub fn get_scalars_imports(&self) -> Vec<String> {
        vec![]
    }

    /// Mirrors `getWrapperDefinitions()`.
    pub fn get_wrapper_definitions(&self) -> Vec<String> {
        if self.config.only_enums {
            return vec![];
        }

        let mut definitions = vec![
            self.get_maybe_value(),
            self.get_input_maybe_value(),
            self.get_exact_definition(),
            self.get_make_optional_definition(),
            self.get_make_maybe_definition(),
            self.get_make_empty_definition(),
            self.get_incremental_definition(),
        ];

        if self.config.wrap_field_definitions {
            definitions.push(self.get_field_wrapper_value());
        }
        if self.config.wrap_entire_field_definitions {
            definitions.push(self.get_entire_field_wrapper_value());
        }

        definitions
    }

    /// Mirrors `getExactDefinition()`.
    pub fn get_exact_definition(&self) -> String {
        if self.config.only_enums {
            return String::new();
        }
        format!("{}{}", self.get_export_prefix(), EXACT_SIGNATURE)
    }

    /// Mirrors `getMakeOptionalDefinition()`.
    pub fn get_make_optional_definition(&self) -> String {
        format!("{}{}", self.get_export_prefix(), MAKE_OPTIONAL_SIGNATURE)
    }

    /// Mirrors `getMakeMaybeDefinition()`.
    pub fn get_make_maybe_definition(&self) -> String {
        if self.config.only_enums {
            return String::new();
        }
        format!("{}{}", self.get_export_prefix(), MAKE_MAYBE_SIGNATURE)
    }

    /// Mirrors `getMakeEmptyDefinition()`.
    pub fn get_make_empty_definition(&self) -> String {
        format!("{}{}", self.get_export_prefix(), MAKE_EMPTY_SIGNATURE)
    }

    /// Mirrors `getIncrementalDefinition()`.
    pub fn get_incremental_definition(&self) -> String {
        format!("{}{}", self.get_export_prefix(), MAKE_INCREMENTAL_SIGNATURE)
    }

    /// Mirrors `getMaybeValue()`.
    pub fn get_maybe_value(&self) -> String {
        format!(
            "{}type Maybe<T> = {};",
            self.get_export_prefix(),
            self.config.maybe_value
        )
    }

    /// Mirrors `getInputMaybeValue()`.
    pub fn get_input_maybe_value(&self) -> String {
        format!(
            "{}type InputMaybe<T> = {};",
            self.get_export_prefix(),
            self.config.input_maybe_value
        )
    }

    /// Mirrors `getFieldWrapperValue()` (stub until `wrapFieldDefinitions` is ported).
    pub fn get_field_wrapper_value(&self) -> String {
        String::new()
    }

    /// Mirrors `getEntireFieldWrapperValue()` (stub).
    pub fn get_entire_field_wrapper_value(&self) -> String {
        String::new()
    }

    /// Mirrors `getExportPrefix()` (only `noExport` branch from TS; `super.getExportPrefix()` defaults to `export `).
    pub fn get_export_prefix(&self) -> &'static str {
        if self.config.no_export { "" } else { "export " }
    }

    /// Mirrors `visitor.scalarsDefinition` — transitional fixed block; will move to base visitor parity.
    pub fn scalars_definition(&self) -> String {
        if self.config.only_enums {
            return String::new();
        }

        let mut s = String::from(
            "/** All built-in and custom scalars, mapped to their actual values */\nexport type Scalars = {\n",
        );
        s.push_str("  ID: { input: string; output: string };\n");
        s.push_str("  String: { input: string; output: string };\n");
        s.push_str("  Boolean: { input: boolean; output: boolean };\n");
        s.push_str("  Int: { input: number; output: number };\n");
        s.push_str("  Float: { input: number; output: number };\n");
        s.push_str("};\n");
        s
    }

    // Introspection-only type emission lives in `introspection_visitor`, mirroring upstream.
}

pub(crate) fn graphql_enum_value_to_ts_key(name: &str) -> String {
    name.split('_')
        .filter(|s| !s.is_empty())
        .map(|part| {
            let mut ch = part.chars();
            match ch.next() {
                None => String::new(),
                Some(f) => {
                    let rest: String = ch.collect();
                    format!("{}{}", f.to_uppercase(), rest.to_lowercase())
                }
            }
        })
        .collect()
}

pub(crate) fn graphql_output_field_type_to_ts_field(t: &Value) -> (bool, String) {
    if t.get("kind").and_then(|k| k.as_str()) == Some("NON_NULL") {
        (
            false,
            output_type_non_null(t.get("ofType").expect("NON_NULL.ofType")),
        )
    } else {
        (true, format!("Maybe<{}>", output_type_non_null(t)))
    }
}

pub(crate) fn graphql_input_field_type_to_ts_field(t: &Value) -> (bool, String) {
    if t.get("kind").and_then(|k| k.as_str()) == Some("NON_NULL") {
        (
            false,
            input_type_non_null(t.get("ofType").expect("NON_NULL.ofType")),
        )
    } else {
        (true, format!("InputMaybe<{}>", input_type_non_null(t)))
    }
}

fn output_type_non_null(t: &Value) -> String {
    match t.get("kind").and_then(|k| k.as_str()) {
        Some("LIST") => {
            let inner = t.get("ofType").expect("LIST.ofType");
            format!("Array<{}>", output_list_element_type(inner))
        }
        Some("NON_NULL") => output_type_non_null(t.get("ofType").expect("NON_NULL.ofType")),
        Some("SCALAR") => {
            let scalar = t.get("name").and_then(|n| n.as_str()).unwrap_or("unknown");
            format!("Scalars['{scalar}']['output']")
        }
        Some("OBJECT") | Some("ENUM") | Some("INTERFACE") | Some("UNION") => t
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or("unknown")
            .to_string(),
        _ => "unknown".to_string(),
    }
}

fn input_type_non_null(t: &Value) -> String {
    match t.get("kind").and_then(|k| k.as_str()) {
        Some("LIST") => {
            let inner = t.get("ofType").expect("LIST.ofType");
            format!("Array<{}>", input_list_element_type(inner))
        }
        Some("NON_NULL") => input_type_non_null(t.get("ofType").expect("NON_NULL.ofType")),
        Some("SCALAR") => {
            let scalar = t.get("name").and_then(|n| n.as_str()).unwrap_or("unknown");
            format!("Scalars['{scalar}']['input']")
        }
        Some("OBJECT") | Some("ENUM") | Some("INPUT_OBJECT") => t
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or("unknown")
            .to_string(),
        _ => "unknown".to_string(),
    }
}

fn output_list_element_type(t: &Value) -> String {
    if t.get("kind").and_then(|k| k.as_str()) == Some("NON_NULL") {
        output_type_non_null(t.get("ofType").expect("LIST element NON_NULL.ofType"))
    } else {
        let (_, ts) = graphql_output_field_type_to_ts_field(t);
        ts
    }
}

fn input_list_element_type(t: &Value) -> String {
    if t.get("kind").and_then(|k| k.as_str()) == Some("NON_NULL") {
        input_type_non_null(t.get("ofType").expect("LIST element NON_NULL.ofType"))
    } else {
        let (_, ts) = graphql_input_field_type_to_ts_field(t);
        ts
    }
}

pub(crate) fn to_pascal_case(name: &str) -> String {
    let mut chars = name.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => format!("{}{}", first.to_uppercase(), chars.collect::<String>()),
    }
}

// comment formatting lives in `visitor-plugin-common`, mirroring upstream.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TypeScriptPluginConfig;
    use plugin_helpers::schema_input::SchemaGenerationInput;

    #[test]
    fn graphql_enum_value_to_ts_key_bar() {
        assert_eq!(graphql_enum_value_to_ts_key("BAR"), "Bar");
    }

    #[test]
    fn get_wrapper_definitions_default_shape() {
        let input = SchemaGenerationInput::default();
        let v = TsVisitor::new(&input, &TypeScriptPluginConfig::default());
        let defs = v.get_wrapper_definitions();
        assert!(defs[0].contains("type Maybe"));
        assert!(defs.iter().any(|d| d.contains("type Exact")));
    }
}
