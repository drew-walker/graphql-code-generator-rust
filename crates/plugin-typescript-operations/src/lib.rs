mod config;
mod index;
mod ts_operation_variables_to_object;
mod ts_selection_set_processor;
mod visitor;

pub use config::TypeScriptDocumentsPluginConfig;
pub use index::plugin;
pub use visitor::TypeScriptDocumentsVisitor;
