use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TypeScriptReactApolloConfig {
    pub with_hooks: bool,
    pub with_component: bool,
    pub with_hoc: bool,
    pub base_types_path: Option<String>,
}

impl Default for TypeScriptReactApolloConfig {
    fn default() -> Self {
        Self {
            with_hooks: true,
            with_component: false,
            with_hoc: false,
            base_types_path: None,
        }
    }
}
