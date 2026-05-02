//! Port of `packages/plugins/typescript/typescript/src/introspection-visitor.ts` (subset).

use anyhow::{Context as _, Result};
use plugin_helpers::schema_input::SchemaGenerationInput;
use serde_json::Value;

use crate::config::TypeScriptPluginConfig;
use crate::visitor::TsVisitor;

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
                    Some("ENUM")
                        | Some("OBJECT")
                        | Some("INTERFACE")
                        | Some("INPUT_OBJECT")
                        | Some("UNION")
                )
            })
            .collect();
        relevant.sort_by_key(|t| t.get("name").and_then(|n| n.as_str()).unwrap_or(""));

        for t in relevant {
            let kind = t.get("kind").and_then(|k| k.as_str()).unwrap_or("");
            if kind == "ENUM" {
                self.ts_visitor.emit_enum_from_introspection(&mut out, t)?;
            } else if kind == "INPUT_OBJECT" && !self.ts_visitor.config.only_enums {
                // Upstream `BaseTypesVisitor.InputObjectTypeDefinition`: gated on `onlyEnums` only,
                // not `onlyOperationTypes` (see visitor-plugin-common `base-types-visitor.ts`).
                self.ts_visitor
                    .emit_input_object_type_from_introspection(&mut out, t)?;
            } else if !self.ts_visitor.config.only_enums
                && !self.ts_visitor.config.only_operation_types
            {
                match kind {
                    "OBJECT" => self
                        .ts_visitor
                        .emit_object_type_from_introspection(&mut out, t)?,
                    "INTERFACE" => self
                        .ts_visitor
                        .emit_interface_type_from_introspection(&mut out, t)?,
                    "UNION" => self
                        .ts_visitor
                        .emit_union_type_from_introspection(&mut out, t)?,
                    "INPUT_OBJECT" => {}
                    _ => {}
                }
            }
        }

        if out.is_empty() {
            return Ok(out);
        }

        Ok(format!("{}\n", out.trim_end_matches('\n')))
    }
}
