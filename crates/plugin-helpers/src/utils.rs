use crate::types::ComplexPluginOutput;

/// Mirrors `mergeOutputs` from `@graphql-codegen/plugin-helpers`.
pub fn merge_outputs(output: &ComplexPluginOutput) -> String {
    let prepend = output.prepend.join("\n");
    let append = output.append.join("\n");

    let mut parts: Vec<String> = Vec::new();
    if !prepend.is_empty() {
        parts.push(prepend);
    }
    if !output.content.is_empty() {
        parts.push(output.content.clone());
    }
    if !append.is_empty() {
        parts.push(append);
    }

    parts.join("\n")
}

/// Merges multiple `ComplexPluginOutput` values in plugin execution order.
///
/// This is used by the core/CLI to accumulate outputs from multiple plugins.
pub fn merge_complex_plugin_output(base: &mut ComplexPluginOutput, next: ComplexPluginOutput) {
    base.prepend.extend(next.prepend);
    base.append.extend(next.append);

    if !next.content.is_empty() {
        if base.content.is_empty() {
            base.content = next.content;
        } else {
            base.content.push('\n');
            base.content.push_str(&next.content);
        }
    }
}
