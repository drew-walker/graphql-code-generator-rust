//! Port of `packages/plugins/typescript/typescript/src/introspection-visitor.ts` (subset).

use anyhow::{Context as _, Result};
use plugin_helpers::schema_input::SchemaGenerationInput;
use serde_json::Value;
use visitor_plugin_common::utils::{WrapInput, transform_comment, wrap_with_single_quotes};

use crate::config::TypeScriptPluginConfig;
use crate::visitor::{
    TsVisitor, graphql_enum_value_to_ts_key, graphql_input_field_type_to_ts_field,
    graphql_output_field_type_to_ts_field, to_pascal_case,
};

/// Mirrors upstream `TsIntrospectionVisitor extends TsVisitor`.
pub struct TsIntrospectionVisitor<'a> {
    ts_visitor: TsVisitor<'a>,
}

impl<'a> TsIntrospectionVisitor<'a> {
    pub fn new(
        schema_input: &'a SchemaGenerationInput,
        plugin_config: &TypeScriptPluginConfig,
    ) -> Self {
        Self {
            ts_visitor: TsVisitor::new(schema_input, plugin_config),
        }
    }

    /// Transitional: builds enum + object blocks from introspection JSON.
    pub fn build_definitions_from_introspection(&self) -> Result<String> {
        let types = self
            .ts_visitor
            .schema_input
            .introspection
            .get("types")
            .and_then(|t| t.as_array())
            .context("introspection.__schema.types missing")?;

        let mut out = String::new();
        let mut relevant: Vec<&Value> = types
            .iter()
            .filter(|t| {
                t.get("name")
                    .and_then(|n| n.as_str())
                    .map(|n| !n.starts_with("__"))
                    .unwrap_or(false)
            })
            .filter(|t| {
                matches!(
                    t.get("kind").and_then(|k| k.as_str()),
                    Some("ENUM") | Some("OBJECT")
                )
            })
            .collect();
        relevant.sort_by_key(|t| t.get("name").and_then(|n| n.as_str()).unwrap_or(""));

        for t in relevant {
            let kind = t.get("kind").and_then(|k| k.as_str()).unwrap_or("");
            if kind == "ENUM" {
                self.emit_enum(&mut out, t)?;
            } else if kind == "OBJECT" && !self.ts_visitor.config.only_enums {
                self.emit_object_type(&mut out, t)?;
            }
        }

        if out.is_empty() {
            return Ok(out);
        }

        Ok(format!("{}\n", out.trim_end_matches('\n')))
    }

    fn emit_enum(&self, out: &mut String, t: &Value) -> Result<()> {
        let name = t
            .get("name")
            .and_then(|n| n.as_str())
            .context("enum without name")?;
        let values = t
            .get("enumValues")
            .and_then(|v| v.as_array())
            .context("enum without enumValues")?;

        let overrides = &self.ts_visitor.schema_input.enum_internal_values;

        if let Some(description) = t.get("description").and_then(|d| d.as_str())
            && !description.is_empty()
        {
            out.push_str(&transform_comment(description, 0, false));
        }

        out.push_str(&format!("export enum {name} {{\n"));

        let mut enum_values: Vec<&Value> = values.iter().collect();
        enum_values.sort_by_key(|ev| ev.get("name").and_then(|n| n.as_str()).unwrap_or(""));

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

    fn emit_object_type(&self, out: &mut String, t: &Value) -> Result<()> {
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

        out.push_str(&format!("export type {name} = {{\n"));
        out.push_str("  __typename?: '");
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
                let (optional, ts) = graphql_output_field_type_to_ts_field(ftype);
                let q = if optional && !self.ts_visitor.config.avoid_optionals {
                    "?"
                } else {
                    ""
                };

                if let Some(desc) = fdesc
                    && !desc.is_empty()
                {
                    out.push_str(&transform_comment(desc, 1, false));
                }

                out.push_str(&format!("  {fname}{q}: {ts};\n"));

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
                        let (arg_optional, arg_ts) = graphql_input_field_type_to_ts_field(arg_type);
                        let arg_q = if arg_optional && !self.ts_visitor.config.avoid_optionals {
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
}
