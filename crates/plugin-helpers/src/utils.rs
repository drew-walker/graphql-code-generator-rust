use crate::types::ComplexPluginOutput;
use std::collections::HashSet;

fn resolve_compare_value(value: &str) -> u8 {
    if value.starts_with("/*")
        || value.starts_with("//")
        || value.starts_with(" *")
        || value.starts_with(" */")
        || value.starts_with("*/")
    {
        return 0;
    }
    if value.starts_with("package") {
        return 1;
    }
    if value.starts_with("import") {
        return 2;
    }
    3
}

/// Mirrors `mergeOutputs` from `@graphql-codegen/plugin-helpers`.
pub fn merge_outputs(output: &ComplexPluginOutput) -> String {
    let mut seen_prepend = HashSet::new();
    let mut prepend_values: Vec<String> = output
        .prepend
        .iter()
        .filter(|value| seen_prepend.insert((*value).clone()))
        .cloned()
        .collect();
    prepend_values.sort_by_key(|value| resolve_compare_value(value));

    let mut seen_append = HashSet::new();
    let append_values: Vec<String> = output
        .append
        .iter()
        .filter(|value| seen_append.insert((*value).clone()))
        .cloned()
        .collect();

    let prepend = prepend_values.join("\n");
    let append = append_values.join("\n");

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

    parts
        .join("\n")
        .lines()
        .filter(|line| line.trim() != "[object Object]")
        .collect::<Vec<_>>()
        .join("\n")
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
