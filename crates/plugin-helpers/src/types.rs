use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct HooksConfig {
    // TODO: model hook configuration.
}

/// Rough stand-in for `Types.PluginContext` from `@graphql-codegen/plugin-helpers`.
#[derive(Debug, Clone, Default)]
pub struct PluginContext(pub HashMap<String, serde_json::Value>);

#[derive(Debug, Clone)]
pub struct FileOutput {
    pub filename: String,
    pub content: Option<String>,
    pub hooks: HooksConfig,
}

#[derive(Debug, Clone, Default)]
pub struct ConfiguredOutput {
    pub overwrite: Option<bool>,
}

#[derive(Debug, Clone)]
pub enum OutputConfig {
    Configured(ConfiguredOutput),
}

/// Stand-in for `Types.Config` from `@graphql-codegen/plugin-helpers`.
#[derive(Debug, Clone)]
pub struct Config {
    /// Mirrors TS: `watch: boolean | string | string[]` (we'll refine later).
    pub watch: bool,
    pub overwrite: Option<bool>,
    pub silent: Option<bool>,
    pub errors_only: Option<bool>,
    pub verbose: Option<bool>,
    pub debug: Option<bool>,
    pub ignore_no_documents: Option<bool>,
    pub emit_legacy_common_js_imports: Option<bool>,
    pub import_extension: Option<String>,
    pub config_file_path: Option<String>,
    pub hooks: HooksConfig,
    pub allow_partial_outputs: bool,
    pub plugin_context: PluginContext,
    pub generates: HashMap<String, OutputConfig>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            watch: false,
            overwrite: None,
            silent: None,
            errors_only: None,
            verbose: None,
            debug: None,
            ignore_no_documents: None,
            emit_legacy_common_js_imports: None,
            import_extension: None,
            config_file_path: None,
            hooks: HooksConfig::default(),
            allow_partial_outputs: true,
            plugin_context: PluginContext::default(),
            generates: HashMap::new(),
        }
    }
}
