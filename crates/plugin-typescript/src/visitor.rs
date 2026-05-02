//! Port of `packages/plugins/typescript/typescript/src/visitor.ts` (subset; grows with parity).

use anyhow::Context as _;
use plugin_helpers::schema_input::SchemaGenerationInput;
use serde_json::Value;

use crate::config::{TypeScriptPluginConfig, TypeScriptPluginParsedConfig};
use visitor_plugin_common::utils::{WrapInput, transform_comment, wrap_with_single_quotes};

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

    pub(crate) fn emit_enum_from_introspection(
        &self,
        out: &mut String,
        t: &Value,
    ) -> anyhow::Result<()> {
        let name = t
            .get("name")
            .and_then(|n| n.as_str())
            .context("enum without name")?;
        let values = t
            .get("enumValues")
            .and_then(|v| v.as_array())
            .context("enum without enumValues")?;

        let overrides = &self.schema_input.enum_internal_values;

        if let Some(description) = t.get("description").and_then(|d| d.as_str())
            && !description.is_empty()
        {
            out.push_str(&transform_comment(description, 0, false));
        }

        let mut enum_values: Vec<&Value> = values.iter().collect();
        enum_values.sort_by_key(|ev| ev.get("name").and_then(|n| n.as_str()).unwrap_or(""));

        if self.config.enums_as_types {
            let any_value_description = enum_values.iter().any(|ev| {
                ev.get("description")
                    .and_then(|d| d.as_str())
                    .is_some_and(|s| !s.is_empty())
            });

            if any_value_description {
                out.push_str(&format!("export type {name} =\n"));
                for ev in &enum_values {
                    if let Some(description) = ev.get("description").and_then(|d| d.as_str())
                        && !description.is_empty()
                    {
                        out.push_str(&transform_comment(description, 1, false));
                    }

                    let gql_name = ev
                        .get("name")
                        .and_then(|n| n.as_str())
                        .context("enum value without name")?;
                    let serialized = overrides
                        .get(name)
                        .and_then(|m| m.get(gql_name))
                        .map(|s| s.as_str())
                        .unwrap_or(gql_name);
                    let skip_numeric_check =
                        overrides.get(name).and_then(|m| m.get(gql_name)).is_some();
                    let lit =
                        wrap_with_single_quotes(WrapInput::Str(serialized), skip_numeric_check);
                    out.push_str(&format!("  | {lit}\n"));
                }
                out.push_str(";\n\n");
            } else {
                let mut parts: Vec<String> = Vec::with_capacity(enum_values.len());
                for ev in &enum_values {
                    let gql_name = ev
                        .get("name")
                        .and_then(|n| n.as_str())
                        .context("enum value without name")?;
                    let serialized = overrides
                        .get(name)
                        .and_then(|m| m.get(gql_name))
                        .map(|s| s.as_str())
                        .unwrap_or(gql_name);
                    let skip_numeric_check =
                        overrides.get(name).and_then(|m| m.get(gql_name)).is_some();
                    parts.push(wrap_with_single_quotes(
                        WrapInput::Str(serialized),
                        skip_numeric_check,
                    ));
                }
                out.push_str(&format!("export type {name} = {};\n\n", parts.join(" | ")));
            }
            return Ok(());
        }

        out.push_str(&format!("export enum {name} {{\n"));

        for ev in enum_values {
            if let Some(description) = ev.get("description").and_then(|d| d.as_str())
                && !description.is_empty()
            {
                out.push_str(&transform_comment(description, 1, false));
            }

            let gql_name = ev
                .get("name")
                .and_then(|n| n.as_str())
                .context("enum value without name")?;
            let ts_key = graphql_enum_value_to_ts_key(gql_name);
            let serialized = overrides
                .get(name)
                .and_then(|m| m.get(gql_name))
                .map(|s| s.as_str())
                .unwrap_or(gql_name);
            let skip_numeric_check = overrides.get(name).and_then(|m| m.get(gql_name)).is_some();
            let lit = wrap_with_single_quotes(WrapInput::Str(serialized), skip_numeric_check);
            out.push_str(&format!("  {ts_key} = {lit},\n"));
        }
        out.push_str("}\n\n");
        Ok(())
    }

    pub(crate) fn emit_object_type_from_introspection(
        &self,
        out: &mut String,
        t: &Value,
    ) -> anyhow::Result<()> {
        let name = t
            .get("name")
            .and_then(|n| n.as_str())
            .context("object without name")?;
        let fields = t.get("fields").and_then(|f| f.as_array());

        let type_description = t.get("description").and_then(|d| d.as_str());
        if let Some(description) = type_description
            && !description.is_empty()
        {
            out.push_str(&transform_comment(description, 0, false));
        }

        let implements = t
            .get("interfaces")
            .and_then(|i| i.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.get("name").and_then(|n| n.as_str()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        if implements.is_empty() {
            out.push_str(&format!("export type {name} = {{\n"));
        } else {
            out.push_str(&format!("export type {name} = "));
            for (idx, i) in implements.iter().enumerate() {
                if idx > 0 {
                    out.push_str(" & ");
                }
                out.push_str(i);
            }
            out.push_str(" & {\n");
        }
        if self.config.immutable_types {
            out.push_str("  readonly __typename?: '");
        } else {
            out.push_str("  __typename?: '");
        }
        out.push_str(name);
        out.push_str("';\n");

        let mut arg_types: Vec<String> = Vec::new();
        if let Some(fields) = fields {
            let mut sorted_fields: Vec<&Value> = fields.iter().collect();
            sorted_fields.sort_by_key(|f| f.get("name").and_then(|n| n.as_str()).unwrap_or(""));

            for f in sorted_fields {
                let fname = f
                    .get("name")
                    .and_then(|n| n.as_str())
                    .context("field without name")?;
                let fdesc = f.get("description").and_then(|d| d.as_str());
                let ftype = f.get("type").context("field without type")?;
                let (optional, ts) =
                    graphql_output_field_type_to_ts_field(ftype, self.config.immutable_types);
                let q = if optional && !self.config.avoid_optionals {
                    "?"
                } else {
                    ""
                };

                if let Some(desc) = fdesc
                    && !desc.is_empty()
                {
                    out.push_str(&transform_comment(desc, 1, false));
                }

                let ro = if self.config.immutable_types {
                    "readonly "
                } else {
                    ""
                };
                out.push_str(&format!("  {ro}{fname}{q}: {ts};\n"));

                let args = f.get("args").and_then(|a| a.as_array());
                if let Some(args) = args
                    && !args.is_empty()
                {
                    let mut args_block = String::new();
                    if let Some(description) = type_description
                        && !description.is_empty()
                    {
                        args_block.push_str(&transform_comment(description, 0, false));
                    }
                    args_block.push_str(&format!(
                        "export type {name}{}Args = {{\n",
                        to_pascal_case(fname)
                    ));
                    let mut sorted_args: Vec<&Value> = args.iter().collect();
                    sorted_args
                        .sort_by_key(|a| a.get("name").and_then(|n| n.as_str()).unwrap_or(""));
                    for arg in sorted_args {
                        let arg_name = arg
                            .get("name")
                            .and_then(|n| n.as_str())
                            .context("arg without name")?;
                        let arg_type = arg.get("type").context("arg without type")?;
                        let (arg_optional, arg_ts) = graphql_input_field_type_to_ts_field(
                            arg_type,
                            self.config.immutable_types,
                        );
                        let arg_q = if arg_optional && !self.config.avoid_optionals {
                            "?"
                        } else {
                            ""
                        };
                        args_block.push_str(&format!("  {arg_name}{arg_q}: {arg_ts};\n"));
                    }
                    args_block.push_str("};\n\n");
                    arg_types.push(args_block);
                }
            }
        }

        out.push_str("};\n\n");
        if !arg_types.is_empty() {
            for arg_type in &arg_types {
                out.push_str(arg_type);
            }
        }
        Ok(())
    }

    pub(crate) fn emit_interface_type_from_introspection(
        &self,
        out: &mut String,
        t: &Value,
    ) -> anyhow::Result<()> {
        let name = t
            .get("name")
            .and_then(|n| n.as_str())
            .context("interface without name")?;
        let fields = t
            .get("fields")
            .and_then(|f| f.as_array())
            .context("interface without fields")?;

        let type_description = t.get("description").and_then(|d| d.as_str());
        if let Some(description) = type_description
            && !description.is_empty()
        {
            out.push_str(&transform_comment(description, 0, false));
        }

        out.push_str(&format!("export type {name} = {{\n"));

        let mut arg_types: Vec<String> = Vec::new();
        let mut sorted_fields: Vec<&Value> = fields.iter().collect();
        sorted_fields.sort_by_key(|f| f.get("name").and_then(|n| n.as_str()).unwrap_or(""));

        for f in sorted_fields {
            let fname = f
                .get("name")
                .and_then(|n| n.as_str())
                .context("field without name")?;
            let fdesc = f.get("description").and_then(|d| d.as_str());
            let ftype = f.get("type").context("field without type")?;
            let (optional, ts) =
                graphql_output_field_type_to_ts_field(ftype, self.config.immutable_types);
            let q = if optional && !self.config.avoid_optionals {
                "?"
            } else {
                ""
            };

            if let Some(desc) = fdesc
                && !desc.is_empty()
            {
                out.push_str(&transform_comment(desc, 1, false));
            }

            let ro = if self.config.immutable_types {
                "readonly "
            } else {
                ""
            };
            out.push_str(&format!("  {ro}{fname}{q}: {ts};\n"));

            let args = f.get("args").and_then(|a| a.as_array());
            if let Some(args) = args
                && !args.is_empty()
            {
                let mut args_block = String::new();
                if let Some(description) = type_description
                    && !description.is_empty()
                {
                    args_block.push_str(&transform_comment(description, 0, false));
                }
                args_block.push_str(&format!(
                    "export type {name}{}Args = {{\n",
                    to_pascal_case(fname)
                ));
                let mut sorted_args: Vec<&Value> = args.iter().collect();
                sorted_args.sort_by_key(|a| a.get("name").and_then(|n| n.as_str()).unwrap_or(""));
                for arg in sorted_args {
                    let arg_name = arg
                        .get("name")
                        .and_then(|n| n.as_str())
                        .context("arg without name")?;
                    let arg_type = arg.get("type").context("arg without type")?;
                    let (arg_optional, arg_ts) =
                        graphql_input_field_type_to_ts_field(arg_type, self.config.immutable_types);
                    let arg_q = if arg_optional && !self.config.avoid_optionals {
                        "?"
                    } else {
                        ""
                    };
                    args_block.push_str(&format!("  {arg_name}{arg_q}: {arg_ts};\n"));
                }
                args_block.push_str("};\n\n");
                arg_types.push(args_block);
            }
        }

        out.push_str("};\n\n");
        for arg_type in &arg_types {
            out.push_str(arg_type);
        }
        Ok(())
    }

    pub(crate) fn emit_input_object_type_from_introspection(
        &self,
        out: &mut String,
        t: &Value,
    ) -> anyhow::Result<()> {
        let name = t
            .get("name")
            .and_then(|n| n.as_str())
            .context("input object without name")?;
        let fields = t
            .get("inputFields")
            .and_then(|f| f.as_array())
            .context("input object without inputFields")?;

        let type_description = t.get("description").and_then(|d| d.as_str());
        if let Some(description) = type_description
            && !description.is_empty()
        {
            out.push_str(&transform_comment(description, 0, false));
        }

        out.push_str(&format!("export type {name} = {{\n"));
        let mut sorted_fields: Vec<&Value> = fields.iter().collect();
        sorted_fields.sort_by_key(|f| f.get("name").and_then(|n| n.as_str()).unwrap_or(""));
        for f in sorted_fields {
            let fname = f
                .get("name")
                .and_then(|n| n.as_str())
                .context("input field without name")?;
            let fdesc = f.get("description").and_then(|d| d.as_str());
            let ftype = f.get("type").context("input field without type")?;
            let (optional, ts) =
                graphql_input_field_type_to_ts_field(ftype, self.config.immutable_types);
            let q = if optional && !self.config.avoid_optionals {
                "?"
            } else {
                ""
            };

            let ro = if self.config.immutable_types {
                "readonly "
            } else {
                ""
            };

            if let Some(desc) = fdesc
                && !desc.is_empty()
            {
                out.push_str(&transform_comment(desc, 1, false));
            }

            out.push_str(&format!("  {ro}{fname}{q}: {ts};\n"));
        }
        out.push_str("};\n\n");
        Ok(())
    }

    pub(crate) fn emit_union_type_from_introspection(
        &self,
        out: &mut String,
        t: &Value,
    ) -> anyhow::Result<()> {
        let name = t
            .get("name")
            .and_then(|n| n.as_str())
            .context("union without name")?;
        let possible = t
            .get("possibleTypes")
            .and_then(|p| p.as_array())
            .context("union without possibleTypes")?;

        let type_description = t.get("description").and_then(|d| d.as_str());
        if let Some(description) = type_description
            && !description.is_empty()
        {
            out.push_str(&transform_comment(description, 0, false));
        }

        let mut members: Vec<&str> = possible
            .iter()
            .filter_map(|v| v.get("name").and_then(|n| n.as_str()))
            .collect();
        members.sort();

        out.push_str(&format!("export type {name} = "));
        for (idx, m) in members.iter().enumerate() {
            if idx > 0 {
                out.push_str(" | ");
            }
            out.push_str(m);
        }
        out.push_str(";\n\n");
        Ok(())
    }
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

pub(crate) fn graphql_output_field_type_to_ts_field(t: &Value, immutable: bool) -> (bool, String) {
    if t.get("kind").and_then(|k| k.as_str()) == Some("NON_NULL") {
        (
            false,
            output_type_non_null(t.get("ofType").expect("NON_NULL.ofType"), immutable),
        )
    } else {
        (
            true,
            format!("Maybe<{}>", output_type_non_null(t, immutable)),
        )
    }
}

pub(crate) fn graphql_input_field_type_to_ts_field(t: &Value, immutable: bool) -> (bool, String) {
    if t.get("kind").and_then(|k| k.as_str()) == Some("NON_NULL") {
        (
            false,
            input_type_non_null(t.get("ofType").expect("NON_NULL.ofType"), immutable),
        )
    } else {
        (
            true,
            format!("InputMaybe<{}>", input_type_non_null(t, immutable)),
        )
    }
}

fn output_type_non_null(t: &Value, immutable: bool) -> String {
    let array_ty = if immutable { "ReadonlyArray" } else { "Array" };
    match t.get("kind").and_then(|k| k.as_str()) {
        Some("LIST") => {
            let inner = t.get("ofType").expect("LIST.ofType");
            format!("{array_ty}<{}>", output_list_element_type(inner, immutable))
        }
        Some("NON_NULL") => {
            output_type_non_null(t.get("ofType").expect("NON_NULL.ofType"), immutable)
        }
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

fn input_type_non_null(t: &Value, immutable: bool) -> String {
    let array_ty = if immutable { "ReadonlyArray" } else { "Array" };
    match t.get("kind").and_then(|k| k.as_str()) {
        Some("LIST") => {
            let inner = t.get("ofType").expect("LIST.ofType");
            format!("{array_ty}<{}>", input_list_element_type(inner, immutable))
        }
        Some("NON_NULL") => {
            input_type_non_null(t.get("ofType").expect("NON_NULL.ofType"), immutable)
        }
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

fn output_list_element_type(t: &Value, immutable: bool) -> String {
    if t.get("kind").and_then(|k| k.as_str()) == Some("NON_NULL") {
        output_type_non_null(
            t.get("ofType").expect("LIST element NON_NULL.ofType"),
            immutable,
        )
    } else {
        let (_, ts) = graphql_output_field_type_to_ts_field(t, immutable);
        ts
    }
}

fn input_list_element_type(t: &Value, immutable: bool) -> String {
    if t.get("kind").and_then(|k| k.as_str()) == Some("NON_NULL") {
        input_type_non_null(
            t.get("ofType").expect("LIST element NON_NULL.ofType"),
            immutable,
        )
    } else {
        let (_, ts) = graphql_input_field_type_to_ts_field(t, immutable);
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
