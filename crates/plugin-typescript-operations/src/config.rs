//! Port of `packages/plugins/typescript/operations/src/config.ts`.

use serde::Deserialize;

/// Mirrors upstream `TypeScriptDocumentsPluginConfig`.
///
/// For now, we only model the subset needed by `dev-test/githunt/types.ts`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TypeScriptDocumentsPluginConfig {
    pub global_namespace: bool,
    pub add_operation_export: bool,
    /// Mirrors upstream `printFieldsOnNewLines` (default false).
    pub print_fields_on_new_lines: bool,
    /// Mirrors upstream `avoidOptionals` for operation result and variables shapes.
    #[serde(default)]
    pub avoid_optionals: bool,
    /// Mirrors upstream `immutableTypes` for operation result selection objects.
    #[serde(default)]
    pub immutable_types: bool,
}
