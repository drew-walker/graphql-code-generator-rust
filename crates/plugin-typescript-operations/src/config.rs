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
    #[serde(alias = "baseTypesPath")]
    pub import_operation_types_from: Option<String>,
    /// Mirrors upstream `printFieldsOnNewLines` (default false).
    pub print_fields_on_new_lines: bool,
    /// Mirrors upstream `avoidOptionals` for operation result and variables shapes.
    #[serde(default)]
    pub avoid_optionals: bool,
    /// Mirrors upstream `immutableTypes` for operation result selection objects.
    #[serde(default)]
    pub immutable_types: bool,
    /// Mirrors upstream `noExport` (omit `export` on generated operation types).
    #[serde(default)]
    pub no_export: bool,
    /// Mirrors upstream `flattenGeneratedTypes` (CLI runs Rust `optimize_operations` first).
    #[serde(default)]
    pub flatten_generated_types: bool,
    /// Mirrors upstream `flattenGeneratedTypesIncludeFragments`.
    #[serde(default)]
    pub flatten_generated_types_include_fragments: bool,
    /// Mirrors upstream `skipTypename`.
    #[serde(default)]
    pub skip_typename: bool,
    /// Mirrors upstream `mergeFragmentTypes`.
    #[serde(default)]
    pub merge_fragment_types: bool,
}
