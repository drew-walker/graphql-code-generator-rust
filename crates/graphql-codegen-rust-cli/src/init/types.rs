use serde::Serialize;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize)]
pub struct PluginOption {
    pub name: String,
    pub package: String,
    pub value: String,
    pub default_extension: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct InitAnswers {
    pub targets: Vec<Tag>,
    pub schema: String,
    pub documents: Option<String>,
    pub plugins: Option<Vec<PluginOption>>,
    pub output: String,
    pub introspection: bool,
    pub config: String,
    pub script: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum Tag {
    Client,
    Node,
    TypeScript,
    Flow,
    Angular,
    Stencil,
    React,
    Vue,
    GraphqlRequest,
}

/// Inferred from `package.json` (TS `guessTargets` → `Record<Tags, boolean>`).
#[derive(Debug, Clone)]
pub struct PossibleTargets {
    pub angular: bool,
    pub react: bool,
    pub stencil: bool,
    pub vue: bool,
    /// TS `Tags.client` key `"Browser"` — always `false` from `guessTargets` in TS.
    pub browser: bool,
    /// TS `Tags.node` — always `false` from `guessTargets` in TS.
    pub node: bool,
    pub typescript: bool,
    pub flow: bool,
    pub graphql_request: bool,
}

/// One entry under `generates` in the codegen config.
#[derive(Debug, Clone, Serialize)]
pub struct GenerateEntry {
    pub preset: Option<String>,
    pub plugins: Vec<String>,
}

/// Subset of GraphQL Code Generator config (TS `Types.Config`).
#[derive(Debug, Clone, Serialize)]
pub struct CodegenConfig {
    pub overwrite: bool,
    pub schema: String,
    pub documents: Option<String>,
    pub generates: HashMap<String, GenerateEntry>,
}

impl CodegenConfig {
    pub fn new(
        overwrite: bool,
        schema: String,
        documents: Option<String>,
        generates: HashMap<String, GenerateEntry>,
    ) -> Self {
        Self {
            overwrite,
            schema,
            documents,
            generates,
        }
    }
}
