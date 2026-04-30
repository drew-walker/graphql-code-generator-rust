//! Port of `packages/plugins/typescript/operations/src/ts-selection-set-processor.ts` (formatting helpers).

/// Equivalent to `{ ${selections.join(', ')} }`, but with line feeds if necessary.
///
/// Upstream reference:
/// `packages/plugins/typescript/operations/src/ts-selection-set-processor.ts` → `formatSelections`.
pub fn format_selections(selections: &[String]) -> String {
    if selections.len() > 1 {
        let joined = selections
            .iter()
            .map(|s| s.replace('\n', "\n  "))
            .collect::<Vec<_>>()
            .join(",\n    ");
        return format!("{{\n    {joined},\n  }}");
    }
    format!("{{ {} }}", selections.join(", "))
}

/// Equivalent to `${transformName}<${target}, ${unionElements.join(' | ')}>`, but with line feeds if necessary.
///
/// Upstream reference:
/// `packages/plugins/typescript/operations/src/ts-selection-set-processor.ts` → `formattedUnionTransform`.
#[allow(dead_code)]
pub fn formatted_union_transform(
    transform_name: &str,
    target: &str,
    union_elements: &[String],
) -> String {
    if union_elements.len() > 3 {
        return format!(
            "{transform_name}<\n    {target},\n    | {}\n  >",
            union_elements.join("\n    | ")
        );
    }
    format!("{transform_name}<{target}, {}>", union_elements.join(" | "))
}
