use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FragmentMatcherConfig {
    pub apollo_client_version: u8,
    pub deterministic: bool,
}

impl Default for FragmentMatcherConfig {
    fn default() -> Self {
        Self {
            apollo_client_version: 3,
            deterministic: false,
        }
    }
}
