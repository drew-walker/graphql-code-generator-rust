//! Rust port of [`@graphql-codegen/typescript`](https://github.com/dotansimha/graphql-code-generator/tree/master/packages/plugins/typescript/typescript).
//!
//! Module layout mirrors the upstream package:
//! `config`, `index`, `visitor`, `introspection_visitor`, `typescript_variables_to_object`.

mod config;
mod index;
mod introspection_visitor;
mod typescript_variables_to_object;
mod visitor;

pub use config::{TypeScriptPluginConfig, TypeScriptPluginParsedConfig, get_config_value};
pub use index::{merge_plugin_output, plugin};
pub use visitor::{
    EXACT_SIGNATURE, MAKE_EMPTY_SIGNATURE, MAKE_INCREMENTAL_SIGNATURE, MAKE_MAYBE_SIGNATURE,
    MAKE_OPTIONAL_SIGNATURE, TsVisitor,
};
