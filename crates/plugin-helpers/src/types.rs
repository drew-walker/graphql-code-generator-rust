use std::collections::HashMap;
use std::fmt;

use serde::de::{self, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};

/// Mirrors the TS pattern where a field can be a single string or an array of strings
/// (e.g. `schema: string | string[]`).
fn string_or_vec<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    struct StringOrVec;

    impl<'de> Visitor<'de> for StringOrVec {
        type Value = Vec<String>;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            write!(f, "a string, array of strings, or null")
        }

        fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(vec![])
        }

        fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(vec![])
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
            Ok(vec![v.to_owned()])
        }

        fn visit_string<E: de::Error>(self, v: String) -> Result<Self::Value, E> {
            Ok(vec![v])
        }

        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let mut out = Vec::new();
            while let Some(s) = seq.next_element::<String>()? {
                out.push(s);
            }
            Ok(out)
        }
    }

    deserializer.deserialize_any(StringOrVec)
}

/// Mirrors `HooksConfig` from `@graphql-codegen/plugin-helpers`.
/// Flexible enough to accept any JSON object (hooks are keyed by lifecycle name).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct HooksConfig {
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// Rough stand-in for `Types.PluginContext` from `@graphql-codegen/plugin-helpers`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct PluginContext(pub HashMap<String, serde_json::Value>);

#[derive(Debug, Clone)]
pub struct FileOutput {
    pub filename: String,
    pub content: Option<String>,
    pub hooks: HooksConfig,
}

/// Mirrors `Types.ConfiguredOutput` from `@graphql-codegen/plugin-helpers`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ConfiguredOutput {
    pub overwrite: Option<bool>,
    #[serde(deserialize_with = "string_or_vec")]
    pub schema: Vec<String>,
    #[serde(deserialize_with = "string_or_vec")]
    pub documents: Vec<String>,
    #[serde(deserialize_with = "string_or_vec")]
    pub external_documents: Vec<String>,
    /// In TS, plugins can be strings or `{ [name: string]: object }`.
    /// For now only string plugin names are supported.
    pub plugins: Vec<String>,
    pub preset: Option<String>,
    pub config: serde_json::Map<String, serde_json::Value>,
    pub hooks: HooksConfig,
}

/// Wrapper that mirrors how the `generates` map value is typed in TS.
/// `#[serde(untagged)]` makes serde try the inner `ConfiguredOutput` shape directly.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum OutputConfig {
    Configured(ConfiguredOutput),
}

/// Stand-in for `Types.Config` from `@graphql-codegen/plugin-helpers`.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Config {
    /// Mirrors TS: `watch: boolean | string | string[]` (simplified to bool for now).
    pub watch: bool,
    pub overwrite: Option<bool>,
    pub silent: Option<bool>,
    pub errors_only: Option<bool>,
    pub verbose: Option<bool>,
    pub debug: Option<bool>,
    pub ignore_no_documents: Option<bool>,
    /// TS key is `emitLegacyCommonJSImports` — the JS acronym is uppercase.
    #[serde(rename = "emitLegacyCommonJSImports")]
    pub emit_legacy_common_js_imports: Option<bool>,
    pub import_extension: Option<String>,
    pub config_file_path: Option<String>,
    pub cwd: Option<String>,
    pub require: Vec<String>,
    #[serde(deserialize_with = "string_or_vec")]
    pub schema: Vec<String>,
    #[serde(deserialize_with = "string_or_vec")]
    pub documents: Vec<String>,
    #[serde(deserialize_with = "string_or_vec")]
    pub external_documents: Vec<String>,
    pub root_config: serde_json::Map<String, serde_json::Value>,
    pub hooks: HooksConfig,
    pub allow_partial_outputs: bool,
    /// Only set programmatically; never in the config file.
    #[serde(skip_deserializing)]
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
            cwd: None,
            require: vec![],
            schema: vec![],
            documents: vec![],
            external_documents: vec![],
            root_config: serde_json::Map::new(),
            hooks: HooksConfig::default(),
            allow_partial_outputs: true,
            plugin_context: PluginContext::default(),
            generates: HashMap::new(),
        }
    }
}
