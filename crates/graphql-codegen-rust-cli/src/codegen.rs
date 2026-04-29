use crate::config::CodegenContext;
use plugin_helpers::types::FileOutput;

#[derive(Debug)]
pub struct ExecuteCodegenOutput {
    pub result: Vec<FileOutput>,
    pub error: Option<anyhow::Error>,
}

pub async fn execute_codegen(_context: &CodegenContext) -> ExecuteCodegenOutput {
    // TODO: real implementation
    ExecuteCodegenOutput {
        result: vec![],
        error: None,
    }
}
